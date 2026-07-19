//! AGS runner — gate-first task-card execution preparer.
//!
//! The runner orchestrates validate → policy → gate → adapter resolve →
//! launch plan. It ONLY consumes the resolved execution policy from
//! `execution-policy` — it never reads raw task-card fields to decide
//! permissions, parallelism, or launch args. It never launches an executor,
//! writes a receipt, runs verification, or claims that the task completed.
//!
//! ## Modes
//!
//! - `check_only`: validate + gate check, exit with decision code.
//! - `dry_run`: full pipeline, output structured `LaunchPlan`, no execution.
//! - default (no flags): prepare execution and return `host_execution_required`.
//!
//! ## Adapter support
//!
//! - `claude-code`: produces a fixed command preview from resolved policy.
//! - `codex-local`: structured host handoff preview.
//! - `cursor`: structured host handoff preview.
//! - `generic`: capped at plan-only, requires human handoff.

use execution_policy::{GateCheckOutput, ResolvedExecutionPolicy};
use request_governance::GovernanceStatus;
use skill_resolver::SkillTagGate;
use std::path::{Path, PathBuf};

// ── Public types ──────────────────────────────────────────────────────────

/// The result of running a task card through the full gate-first pipeline.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchPlan {
    pub schema_version: String,
    pub task_card_path: String,
    pub mode: String,
    pub governance_status: GovernanceStatus,
    /// True only when policy and runtime gates accepted the card and the host
    /// must perform the actual execution outside this crate.
    pub host_execution_required: bool,
    /// Always false: the runner is a launch-plan preparer, not an executor.
    pub execution_performed: bool,
    /// Always false: verification belongs to the host after execution.
    pub verification_performed: bool,

    // Validation
    pub validation_passed: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub validation_errors: Vec<String>,

    // Gate
    pub gate_decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate_error_kind: Option<String>,

    // Resolved policy (the sole authority for execution decisions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_policy: Option<ResolvedExecutionPolicy>,

    // Adapter plan
    pub adapter: AdapterPlan,

    // Runtime skill-tag availability gate (the third gate). `None` when the card
    // carries no trailing `[skill: …]` tags or the runner stopped before the
    // launch-plan phase (read/validation failure, check-only). Present on the
    // launch-plan path; a non-`all_accepted` gate forces `gate_decision = stop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_tags_gate: Option<SkillTagGate>,

    // Receipt / verification / delivery report planning
    pub receipt_plan: ReceiptPlan,
    pub verification_log_refs: Vec<String>,
    pub delivery_report_ref: String,
}

/// Adapter-specific launch plan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdapterPlan {
    /// Runtime adapter: claude-code, codex-local, cursor, generic
    pub adapter: String,
    /// Human-readable launch command description
    pub launch_command: String,
    /// Arguments from resolved policy (verbatim, never from raw fields)
    pub launch_args: Vec<String>,
    /// Whether this adapter is a stub (true for codex-local, cursor)
    pub is_stub: bool,
    /// Why this adapter is a stub
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stub_reason: Option<String>,
    /// Expected executor binary
    pub executor_binary: String,
    /// Always false. The host owns process launch after consuming this plan.
    pub dispatched: bool,
}

/// Plan for receipt generation after execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReceiptPlan {
    /// Whether the host should generate a receipt after real execution.
    pub host_should_generate: bool,
    pub receipt_id_prefix: String,
    pub task_card_hash: String,
    pub gate_result_for_receipt: String,
    pub suggested_verification_commands: Vec<String>,
    /// Always false. This structure describes a future host obligation.
    pub generated: bool,
}

// ── Constants ─────────────────────────────────────────────────────────────

pub const SCHEMA_VERSION: &str = "0.3.0-launch-plan";

// ── Main entry point ──────────────────────────────────────────────────────

/// Run a task card through the gate-first pipeline.
///
/// `check_only` — stop after gate check, don't build full launch plan.
/// `dry_run` — full pipeline, mark as dry run.
/// `approve_writes` — pass the write-approval audit/hint signal to the resolver
/// (may act as the M9 generic-adapter capability override).
/// `current_task_approval` — pass the host-detected, current-task execution
/// instruction to the resolver as an audit/hint signal. Task level does not
/// downgrade the permission mode, so neither signal is a Heavy execution unlock.
///
/// Returns a `LaunchPlan`; no branch launches a process or performs task work.
/// The caller checks `governance_status` and `host_execution_required` before
/// deciding whether the host should execute the prepared plan.
pub fn run_task_card(
    task_card_path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
    current_task_approval: bool,
) -> LaunchPlan {
    // The runtime skill-tag gate reads the manifest routing authority + the
    // machine-local ActiveSkillTable snapshot. Resolve both from the real
    // process cwd / runtime home; tests use `run_task_card_inner` to inject
    // hermetic roots.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let runtime_home = skill_resolver::locate_runtime_home();
    let authority_root = skill_resolver::resolve_capability_authority_root(
        &cwd,
        &runtime_home,
        std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
    );
    let manifest_root = authority_root.as_ref().unwrap_or(&cwd);
    let mut plan = run_task_card_inner(
        task_card_path,
        check_only,
        dry_run,
        approve_writes,
        current_task_approval,
        manifest_root,
        &runtime_home,
    );
    if let Err(error) = authority_root {
        if plan.skill_tags_gate.is_some() {
            plan.gate_decision = "stop".to_string();
            plan.gate_error_kind = Some("capability_authority_unresolved".to_string());
            plan.validation_errors.push(error.to_string());
            plan.skill_tags_gate = None;
            plan.receipt_plan.host_should_generate = false;
            plan.host_execution_required = false;
            plan.governance_status = GovernanceStatus::BlockedByPolicy;
            plan.receipt_plan.gate_result_for_receipt = "stop".to_string();
            plan.delivery_report_ref =
                "BLOCKED — capability authority root could not be resolved".to_string();
        }
    }
    plan
}

/// Map a resolved runtime adapter to the host identifier the runtime skill-tag
/// gate probes. `generic` / unknown → host-agnostic (`""`), which is fail-closed
/// conservative: with no positive host visibility evidence, any `[skill: …]` tag
/// reads as not-available and the launch stops.
fn host_for_adapter(adapter: &str) -> &str {
    match adapter {
        "claude-code" => "claude-code",
        "codex-local" => "codex",
        "cursor" => "cursor",
        _ => "",
    }
}

/// Core of [`run_task_card`] with the skill-tag gate's `manifest_root` and
/// `runtime_home` injected (hermetic in tests; real cwd / runtime home in
/// production via the public wrapper). The runtime skill-tag availability gate
/// (the third gate) runs on the launch-plan path only — read/validation failures
/// and check-only return before it, so the offline policy gate stays static.
#[allow(clippy::too_many_arguments)]
pub fn run_task_card_inner(
    task_card_path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
    current_task_approval: bool,
    manifest_root: &Path,
    runtime_home: &Path,
) -> LaunchPlan {
    let mode = if check_only {
        "check-only"
    } else if dry_run {
        "dry-run"
    } else {
        "prepare-execution"
    };

    let display_path = if task_card_path == "-" {
        "(stdin)".to_string()
    } else {
        task_card_path.to_string()
    };

    // ── Phase 1: Read task card ────────────────────────────────────────
    let content = match read_input(task_card_path) {
        Ok(c) => c,
        Err(e) => {
            return LaunchPlan {
                schema_version: SCHEMA_VERSION.to_string(),
                task_card_path: display_path,
                mode: mode.to_string(),
                governance_status: GovernanceStatus::BlockedByPolicy,
                host_execution_required: false,
                execution_performed: false,
                verification_performed: false,
                validation_passed: false,
                validation_errors: vec![e],
                gate_decision: "stop".to_string(),
                gate_error_kind: Some("read_error".to_string()),
                resolved_policy: None,
                adapter: stub_adapter("generic", "read error — cannot resolve adapter"),
                skill_tags_gate: None,
                receipt_plan: empty_receipt_plan(""),
                verification_log_refs: vec![],
                delivery_report_ref: String::new(),
            };
        }
    };

    let task_card_hash = receipt_hash(content.as_bytes());

    // ── Phase 2: Validate ──────────────────────────────────────────────
    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            return LaunchPlan {
                schema_version: SCHEMA_VERSION.to_string(),
                task_card_path: display_path,
                mode: mode.to_string(),
                governance_status: GovernanceStatus::BlockedByPolicy,
                host_execution_required: false,
                execution_performed: false,
                verification_performed: false,
                validation_passed: false,
                validation_errors: errors.clone(),
                gate_decision: "stop".to_string(),
                gate_error_kind: Some("validation_failed".to_string()),
                resolved_policy: None,
                adapter: stub_adapter("generic", "validation failed — cannot resolve adapter"),
                skill_tags_gate: None,
                receipt_plan: empty_receipt_plan(&task_card_hash),
                verification_log_refs: vec![],
                delivery_report_ref: String::new(),
            };
        }
    };

    // ── Phase 3: Build policy input ────────────────────────────────────
    // IMPORTANT: the runner builds TaskPolicyInput from validated fields
    // but NEVER reads raw fields directly for launch decisions. All
    // execution parameters come from the resolved policy.
    // Use the canonical approval builder shared with the CLI gate and the AGS
    // MCP. `--current-task-approval` is structured live-request evidence from
    // the host/operator; it is never read from task-card prose and is an
    // audit/hint signal only — task level does not downgrade the permission mode.
    let input = execution_policy::TaskPolicyInput::from_fields_with_approval(
        &card.fields,
        approve_writes,
        current_task_approval,
    );

    // ── Phase 4: Gate check (validate + resolve + decide) ──────────────
    let gate_output: GateCheckOutput = execution_policy::gate_check(&input);
    let decision_str = gate_output.decision.to_string().to_lowercase();
    let policy = gate_output.resolved_policy;

    // ── Phase 5: If check_only, stop here ──────────────────────────────
    if check_only {
        let gate_result_for_receipt = decision_str.clone();
        let governance_status = if policy.stop_before_launch {
            GovernanceStatus::BlockedByPolicy
        } else {
            GovernanceStatus::AdvisoryNoMutation
        };
        return LaunchPlan {
            schema_version: SCHEMA_VERSION.to_string(),
            task_card_path: display_path,
            mode: mode.to_string(),
            governance_status,
            host_execution_required: false,
            execution_performed: false,
            verification_performed: false,
            validation_passed: true,
            validation_errors: vec![],
            gate_decision: decision_str,
            gate_error_kind: None,
            resolved_policy: Some(policy),
            adapter: AdapterPlan {
                adapter: "check-only".to_string(),
                launch_command: String::new(),
                launch_args: vec![],
                is_stub: true,
                stub_reason: Some("check-only mode — no adapter resolved".to_string()),
                executor_binary: String::new(),
                dispatched: false,
            },
            // check-only stops at the offline policy gate; the runtime skill-tag
            // gate belongs to the launch-plan path (dry-run / plan) below.
            skill_tags_gate: None,
            receipt_plan: ReceiptPlan {
                host_should_generate: false,
                receipt_id_prefix: format!(
                    "receipt-{}",
                    &task_card_hash[..12.min(task_card_hash.len())]
                ),
                task_card_hash,
                gate_result_for_receipt,
                suggested_verification_commands: vec![],
                generated: false,
            },
            verification_log_refs: vec![],
            delivery_report_ref: String::new(),
        };
    }

    // ── Phase 5.5: Runtime skill-tag availability gate (the third gate) ──
    // The validator already enforced the static gates offline: (1) the tag is
    // registry-routable and (2) it has a legal `[skill: …]` invoke_hint. This is
    // the runtime gate (3): the live machine snapshot must judge each trailing
    // `[skill: …]` tag Available for the active host (enrolled + canonical
    // present + auth satisfied + host-visible + healthy). It runs automatically
    // on every launch-plan path — this is the main task-card execution chain
    // (`ags run` → `scripts/run-task-card.sh`), not a manual side command. A
    // rejected tag is a launch blocker: deterministic and fail-closed.
    let active_host = host_for_adapter(&policy.runtime_adapter);
    let skill_tags = task_card_validator::extract_skill_tags(&content);
    let skill_tags_gate: Option<SkillTagGate> = if skill_tags.is_empty() {
        None
    } else {
        Some(skill_resolver::verify_skill_tags_with_runtime_home(
            &skill_tags,
            manifest_root,
            active_host,
            runtime_home,
        ))
    };
    let skill_tags_blocked = skill_tags_gate
        .as_ref()
        .map(|g| !g.all_accepted)
        .unwrap_or(false);

    // The launch is blocked if the policy resolver stopped it OR a skill tag is
    // unavailable at runtime. Either reason makes the card non-launchable.
    let launch_blocked = policy.stop_before_launch || skill_tags_blocked;
    let (decision_str, gate_error_kind) = if skill_tags_blocked {
        (
            "stop".to_string(),
            Some("skill_tags_unavailable".to_string()),
        )
    } else {
        (decision_str, None)
    };

    // ── Phase 6: Adapter resolution ────────────────────────────────────
    let adapter_plan = resolve_adapter(&policy, &display_path);

    // ── Phase 7: Receipt / verification / delivery planning ────────────
    let gate_result_for_receipt = decision_str.clone();
    let receipt_plan = ReceiptPlan {
        host_should_generate: !launch_blocked,
        receipt_id_prefix: format!(
            "receipt-{}",
            &task_card_hash[..12.min(task_card_hash.len())]
        ),
        task_card_hash,
        gate_result_for_receipt,
        suggested_verification_commands: vec![
            "cargo fmt --check".to_string(),
            "RUSTFLAGS=\"-D warnings\" cargo test".to_string(),
            "cargo build --release".to_string(),
            "ags verify --scope local --format json".to_string(),
        ],
        generated: false,
    };

    let verification_log_refs = vec![
        "verification.log".to_string(),
        "delivery-report.md".to_string(),
    ];

    let delivery_report_ref = if skill_tags_blocked {
        "BLOCKED — delivery report not applicable (runtime skill-tag gate rejected a tag)"
            .to_string()
    } else if policy.stop_before_launch {
        "BLOCKED — delivery report not applicable (stop_before_launch=true)".to_string()
    } else {
        "host must generate delivery-report.md after execution and verification".to_string()
    };

    LaunchPlan {
        schema_version: SCHEMA_VERSION.to_string(),
        task_card_path: display_path,
        mode: mode.to_string(),
        governance_status: if launch_blocked {
            GovernanceStatus::BlockedByPolicy
        } else {
            GovernanceStatus::HostExecutionRequired
        },
        host_execution_required: !launch_blocked,
        execution_performed: false,
        verification_performed: false,
        validation_passed: true,
        validation_errors: vec![],
        gate_decision: decision_str,
        gate_error_kind,
        resolved_policy: Some(policy),
        adapter: adapter_plan,
        skill_tags_gate,
        receipt_plan,
        verification_log_refs,
        delivery_report_ref,
    }
}

// ── Adapter resolution ────────────────────────────────────────────────────

/// Resolve the adapter plan from the resolved policy.
///
/// This is the ONLY place that translates runtime_adapter to a concrete
/// launch strategy. It reads the adapter field from the RESOLVED POLICY,
/// never from the raw task card.
fn resolve_adapter(policy: &ResolvedExecutionPolicy, _task_card_path: &str) -> AdapterPlan {
    let adapter = &policy.runtime_adapter;

    // ── Resolve permission-mode launch args ────────────────────────────
    // These come from allowed_launch_args (already gated by the resolver).
    // The runner just passes them through verbatim.
    let launch_args = policy.allowed_launch_args.clone();

    match adapter.as_str() {
        "claude-code" => {
            let binary = "claude".to_string();
            let cmd = if launch_args.is_empty() {
                "claude -p - < <task-card>".to_string()
            } else {
                format!("claude {} -p - < <task-card>", launch_args.join(" "))
            };

            AdapterPlan {
                adapter: "claude-code".to_string(),
                launch_command: cmd,
                launch_args,
                is_stub: false,
                stub_reason: None,
                executor_binary: binary,
                dispatched: false,
            }
        }

        "codex-local" => AdapterPlan {
            adapter: "codex-local".to_string(),
            launch_command: format!(
                "codex execute --permission {} [task-card]",
                policy.effective_permission_mode
            ),
            launch_args: vec![],
            is_stub: true,
            stub_reason: Some(
                "codex-local adapter is a structured stub. Full adapter wiring \
                 requires Codex runtime integration (TBD after Codex review)."
                    .to_string(),
            ),
            executor_binary: "codex".to_string(),
            dispatched: false,
        },

        "cursor" => AdapterPlan {
            adapter: "cursor".to_string(),
            launch_command: format!(
                "cursor agent --mode {} [task-card]",
                policy.effective_permission_mode
            ),
            launch_args: vec![],
            is_stub: true,
            stub_reason: Some(
                "cursor adapter is a structured stub. Full adapter wiring \
                 requires Cursor IDE runtime integration (TBD after Codex review)."
                    .to_string(),
            ),
            executor_binary: "cursor".to_string(),
            dispatched: false,
        },

        _ => AdapterPlan {
            adapter: "generic".to_string(),
            launch_command: "human handoff — no automated executor".to_string(),
            launch_args: vec![],
            is_stub: true,
            stub_reason: Some(format!(
                "Unknown or generic runtime adapter '{}'. Execution requires human handoff.",
                adapter
            )),
            executor_binary: "human".to_string(),
            dispatched: false,
        },
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn read_input(path: &str) -> Result<String, String> {
    if path == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read stdin: {}", e))?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path, e))
    }
}

fn receipt_hash(data: &[u8]) -> String {
    receipt::sha256_hex(data)
}

fn stub_adapter(adapter: &str, reason: &str) -> AdapterPlan {
    AdapterPlan {
        adapter: adapter.to_string(),
        launch_command: String::new(),
        launch_args: vec![],
        is_stub: true,
        stub_reason: Some(reason.to_string()),
        executor_binary: String::new(),
        dispatched: false,
    }
}

fn empty_receipt_plan(hash: &str) -> ReceiptPlan {
    ReceiptPlan {
        host_should_generate: false,
        receipt_id_prefix: if hash.is_empty() {
            String::new()
        } else {
            format!("receipt-{}", &hash[..12.min(hash.len())])
        },
        task_card_hash: hash.to_string(),
        gate_result_for_receipt: "stop".to_string(),
        suggested_verification_commands: vec![],
        generated: false,
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// Render a LaunchPlan as human-readable text.
pub fn render_text(plan: &LaunchPlan) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("AGS Runner — Launch Plan".to_string());
    lines.push("=========================".to_string());
    lines.push(format!("Schema version:  {}", plan.schema_version));
    lines.push(format!("Task card:       {}", plan.task_card_path));
    lines.push(format!("Mode:            {}", plan.mode));
    lines.push(format!(
        "Governance:      {}",
        plan.governance_status.as_str()
    ));
    lines.push(format!("Host execution:  {}", plan.host_execution_required));
    lines.push("Execution done:  false".to_string());
    lines.push("Verification done: false".to_string());
    lines.push(String::new());

    // Validation
    lines.push("─ Validation ─".to_string());
    lines.push(format!("  Passed:  {}", plan.validation_passed));
    if !plan.validation_errors.is_empty() {
        lines.push("  Errors:".to_string());
        for err in &plan.validation_errors {
            lines.push(format!("    - {}", err));
        }
    }
    lines.push(String::new());

    // Gate
    lines.push("─ Gate Decision ─".to_string());
    lines.push(format!("  Decision:  {}", plan.gate_decision));
    if let Some(ref kind) = plan.gate_error_kind {
        lines.push(format!("  Error:     {}", kind));
    }
    lines.push(String::new());

    // Resolved policy
    if let Some(ref policy) = plan.resolved_policy {
        lines.push("─ Resolved Policy ─".to_string());
        lines.push(format!("  Executor:          {}", policy.executor));
        lines.push(format!("  Runtime adapter:   {}", policy.runtime_adapter));
        lines.push(format!(
            "  Permission mode:   {}",
            policy.effective_permission_mode
        ));
        lines.push(format!(
            "  Parallelism:       {}",
            policy.effective_parallelism
        ));
        lines.push(format!(
            "  Exec surface:      {}",
            policy.effective_execution_surface
        ));
        lines.push(format!("  Execution effort:  {}", policy.execution_effort));
        lines.push(format!(
            "  Stop before launch: {}",
            policy.stop_before_launch
        ));
        if !policy.stop_reasons.is_empty() {
            lines.push("  Stop reasons:".to_string());
            for reason in &policy.stop_reasons {
                lines.push(format!("    - {}", reason));
            }
        }
        if policy.was_downgraded {
            lines.push("  Downgrades:".to_string());
            for reason in &policy.downgrade_reasons {
                lines.push(format!("    - {}", reason));
            }
        }
        lines.push(String::new());
    }

    // Adapter
    lines.push("─ Adapter Plan ─".to_string());
    lines.push(format!("  Adapter:     {}", plan.adapter.adapter));
    lines.push(format!("  Binary:      {}", plan.adapter.executor_binary));
    lines.push(format!("  Launch cmd:  {}", plan.adapter.launch_command));
    if !plan.adapter.launch_args.is_empty() {
        lines.push(format!(
            "  Launch args: {}",
            plan.adapter.launch_args.join(" ")
        ));
    }
    if plan.adapter.is_stub {
        lines.push("  Status:      STUB — not wired for real execution".to_string());
        if let Some(ref reason) = plan.adapter.stub_reason {
            lines.push(format!("  Reason:      {}", reason));
        }
    }
    lines.push(String::new());

    // Runtime skill-tag availability gate (the third gate)
    if let Some(ref gate) = plan.skill_tags_gate {
        lines.push("─ Skill-Tag Gate (runtime) ─".to_string());
        lines.push(format!("  Active host:  {}", gate.active_host));
        lines.push(format!("  Snapshot:     {}", gate.snapshot_hash));
        lines.push(format!("  All accepted: {}", gate.all_accepted));
        for v in &gate.verdicts {
            lines.push(format!(
                "    - [skill: {}] {}{}",
                v.tag,
                if v.accepted { "ACCEPT" } else { "REJECT" },
                if v.reason.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", v.reason)
                },
            ));
        }
        lines.push(String::new());
    }

    // Receipt plan
    lines.push("─ Receipt / Verification / Delivery ─".to_string());
    lines.push(format!(
        "  Receipt:      {}",
        if plan.receipt_plan.host_should_generate {
            "host should generate after execution"
        } else {
            "skipped (stopped or check-only)"
        }
    ));
    lines.push(format!(
        "  Receipt ID:   {}",
        plan.receipt_plan.receipt_id_prefix
    ));
    lines.push(format!(
        "  Card hash:    {}",
        plan.receipt_plan.task_card_hash
    ));
    lines.push("  Verification commands:".to_string());
    for cmd in &plan.receipt_plan.suggested_verification_commands {
        lines.push(format!("    - {}", cmd));
    }
    lines.push(format!("  Delivery report: {}", plan.delivery_report_ref));
    lines.push(String::new());

    // Summary
    lines.push("─ Summary ─".to_string());
    let verdict = match plan.gate_decision.as_str() {
        "allow" => "HOST EXECUTION REQUIRED — launch plan prepared; runner did not execute",
        "stop" => "STOP — blocked; runner did not execute",
        _ => "UNKNOWN",
    };
    lines.push(format!("  Verdict:  {}", verdict));

    if plan.gate_decision != "stop" {
        lines.push(
            "  NOTE: The host owns execution, verification, and receipt writing.".to_string(),
        );
    }

    lines.join("\n")
}

/// Render a LaunchPlan as JSON string.
pub fn render_json(plan: &LaunchPlan) -> String {
    serde_json::to_string_pretty(plan)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use execution_policy::{ApprovalSource, Parallelism, PermissionMode};

    #[test]
    fn test_read_error_produces_stop_plan() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
        assert!(!plan.validation_passed);
        assert_eq!(plan.gate_decision, "stop");
        assert!(plan.gate_error_kind.is_some());
        assert!(plan.resolved_policy.is_none());
        assert!(plan.adapter.is_stub);
        assert_eq!(plan.governance_status, GovernanceStatus::BlockedByPolicy);
        assert!(!plan.host_execution_required);
        assert!(!plan.execution_performed);
        assert!(!plan.verification_performed);
    }

    #[test]
    fn test_check_only_mode_flag() {
        let plan = run_task_card("/nonexistent/path/task-card.md", true, false, false, false);
        assert_eq!(plan.mode, "check-only");
        assert!(!plan.validation_passed);
    }

    #[test]
    fn test_dry_run_mode_flag() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
        assert_eq!(plan.mode, "dry-run");
    }

    #[test]
    fn test_default_mode_prepares_execution() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, false, false, false);
        assert_eq!(plan.mode, "prepare-execution");
    }

    #[test]
    fn test_schema_version_constant() {
        assert_eq!(SCHEMA_VERSION, "0.3.0-launch-plan");
    }

    #[test]
    fn test_claude_code_adapter_is_not_stub() {
        // Build a minimal policy that maps to claude-code
        let policy = ResolvedExecutionPolicy {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            effective_permission_mode: PermissionMode::PlanOnly,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "cli".into(),
            allowed_launch_args: vec!["--permission-mode".into(), "plan".into()],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        assert_eq!(adapter.adapter, "claude-code");
        assert!(!adapter.is_stub);
        assert_eq!(adapter.executor_binary, "claude");
        assert_eq!(adapter.launch_args, vec!["--permission-mode", "plan"]);
    }

    #[test]
    fn test_codex_adapter_is_stub() {
        let policy = ResolvedExecutionPolicy {
            executor: "Codex".into(),
            runtime_adapter: "codex-local".into(),
            effective_permission_mode: PermissionMode::ExecuteAndVerify,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "local-workspace".into(),
            allowed_launch_args: vec![],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        assert_eq!(adapter.adapter, "codex-local");
        assert!(adapter.is_stub);
        assert!(adapter.stub_reason.is_some());
    }

    #[test]
    fn test_cursor_adapter_is_stub() {
        let policy = ResolvedExecutionPolicy {
            executor: "Cursor".into(),
            runtime_adapter: "cursor".into(),
            effective_permission_mode: PermissionMode::ExecuteAndVerify,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "ide".into(),
            allowed_launch_args: vec![],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        assert_eq!(adapter.adapter, "cursor");
        assert!(adapter.is_stub);
    }

    #[test]
    fn test_generic_adapter_is_stub() {
        let policy = ResolvedExecutionPolicy {
            executor: "Other".into(),
            runtime_adapter: "generic".into(),
            effective_permission_mode: PermissionMode::PlanOnly,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "cli".into(),
            allowed_launch_args: vec![],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        assert_eq!(adapter.adapter, "generic");
        assert!(adapter.is_stub);
    }

    #[test]
    fn test_receipt_plan_skipped_on_stop() {
        // Simulate: validation failure, plan has no resolved_policy
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
        assert!(!plan.receipt_plan.host_should_generate);
        assert_eq!(plan.receipt_plan.gate_result_for_receipt, "stop");
    }

    #[test]
    fn test_receipt_hash_matches_receipt_crate_sha256() {
        let content = b"## Task Card\ncanonical receipt hash\n";
        assert_eq!(receipt_hash(content), receipt::sha256_hex(content));
        assert_eq!(receipt_hash(content).len(), 64);
    }

    #[test]
    fn test_render_text_produces_output() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
        let text = render_text(&plan);
        assert!(text.contains("AGS Runner"));
        assert!(text.contains("STOP"));
    }

    #[test]
    fn test_render_json_produces_valid_json() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false, false);
        let json = render_json(&plan);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok());
        let v = parsed.unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert_eq!(v["gate_decision"], "stop");
    }

    #[test]
    fn test_runner_never_reads_raw_parallelism() {
        // The runner module does NOT import or use raw task-card fields
        // for launch decisions. All execution params come from resolved_policy.
        // This test verifies the structural invariant: resolve_adapter()
        // only reads from ResolvedExecutionPolicy, not from task card fields.
        let policy = ResolvedExecutionPolicy {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            effective_permission_mode: PermissionMode::PlanOnly,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "cli".into(),
            allowed_launch_args: vec![], // empty — M5/M6 enforced
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: true, // parallelism was downgraded
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        // Even though the raw card might have said Parallelism: worktree,
        // the resolved policy says none — and the adapter uses the resolved value.
        assert!(adapter.launch_args.is_empty());
        // No --parallel, --worktree, or other write-enabling flags appear.
        let cmd = &adapter.launch_command;
        assert!(!cmd.contains("--parallel"));
        assert!(!cmd.contains("--worktree"));
    }

    #[test]
    fn test_launch_args_flow_verbatim_from_policy() {
        let policy = ResolvedExecutionPolicy {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            effective_permission_mode: PermissionMode::PlanOnly,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "cli".into(),
            allowed_launch_args: vec![
                "--permission-mode".into(),
                "plan".into(),
                "--output-format".into(),
                "json".into(),
            ],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            execution_effort: "normal".into(),
            is_exhaustive_mode: false,
            approval_source: ApprovalSource::None,
        };

        let adapter = resolve_adapter(&policy, "test-task-card.md");
        // allowed_launch_args flow verbatim into the adapter
        assert_eq!(
            adapter.launch_args,
            vec!["--permission-mode", "plan", "--output-format", "json"]
        );
        assert!(adapter
            .launch_command
            .contains("--permission-mode plan --output-format json"));
    }

    #[test]
    fn test_approve_writes_flag_sets_approval_source() {
        // Read a real task card fixture to test the approve_writes flow
        let fixture = std::path::Path::new("../tests/fixtures");
        let card_path = fixture.join("heavy-plan-only.md");
        if card_path.exists() {
            let path_str = card_path.to_string_lossy().to_string();
            let plan_without = run_task_card(&path_str, false, true, false, false);
            let plan_with = run_task_card(&path_str, false, true, true, false);

            // Both should pass validation
            assert!(plan_without.validation_passed);
            assert!(plan_with.validation_passed);

            // With approve_writes, the policy should reflect CliFlag approval
            if let Some(ref policy) = plan_with.resolved_policy {
                assert_eq!(policy.approval_source.to_string(), "cli-flag");
            }
        }
    }

    #[test]
    fn test_current_task_approval_flag_sets_approval_source() {
        let fixture = std::path::Path::new("../tests/fixtures");
        let card_path = fixture.join("heavy-plan-only.md");
        if card_path.exists() {
            let path_str = card_path.to_string_lossy().to_string();
            let plan = run_task_card(&path_str, false, true, false, true);

            assert!(plan.validation_passed);
            if let Some(ref policy) = plan.resolved_policy {
                assert_eq!(
                    policy.approval_source.to_string(),
                    "current-task-instruction"
                );
            }
        }
    }

    // ── Runtime skill-tag availability gate (the third gate) integration ─────

    const VALID_CARD: &str = include_str!("../../../tests/fixtures/valid-full.md");

    fn repo_root_for_test() -> PathBuf {
        // crates/runner → repo root.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    fn unique_tmp(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ags-runner-skilltag-{}-{}",
            label,
            std::process::id()
        ))
    }

    #[test]
    fn host_for_adapter_maps_known_adapters() {
        assert_eq!(host_for_adapter("claude-code"), "claude-code");
        assert_eq!(host_for_adapter("codex-local"), "codex");
        assert_eq!(host_for_adapter("cursor"), "cursor");
        // generic / unknown → host-agnostic (fail-closed).
        assert_eq!(host_for_adapter("generic"), "");
        assert_eq!(host_for_adapter("anything-else"), "");
    }

    #[test]
    fn runtime_skill_tag_gate_stops_unavailable_tag() {
        // The card PASSES the offline static validator (skill-creator is a
        // routable registry tag), but its runtime availability fails: an empty
        // runtime home has no ActiveSkillTable snapshot, so skill resolution stops
        // (`not-enrolled`) and the third gate stops the launch. This proves the
        // runtime gate runs automatically on the `ags run` launch-plan path —
        // not only as the manual `ags gate skill-tags` subcommand.
        let dir = unique_tmp("stop");
        std::fs::create_dir_all(&dir).unwrap();
        let card_path = dir.join("card.md");
        std::fs::write(
            &card_path,
            format!("{VALID_CARD}\n[skill: skill-creator]\n"),
        )
        .unwrap();
        let runtime_home = dir.join("runtime-home"); // absent snapshot → governance precondition

        let plan = run_task_card_inner(
            &card_path.to_string_lossy(),
            false, // not check-only
            true,  // dry-run (launch-plan path)
            false,
            false,
            &repo_root_for_test(),
            &runtime_home,
        );
        assert!(plan.validation_passed, "card must pass static validation");
        assert_eq!(plan.gate_decision, "stop");
        assert_eq!(
            plan.gate_error_kind.as_deref(),
            Some("skill_tags_unavailable")
        );
        let gate = plan.skill_tags_gate.expect("skill_tags_gate present");
        assert!(!gate.all_accepted);
        assert!(gate.rejected.iter().any(|t| t == "skill-creator"));
        assert!(
            !plan.receipt_plan.host_should_generate,
            "a blocked launch must not plan a receipt"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runtime_skill_tag_gate_absent_when_card_has_no_tags() {
        // The base valid-full fixture has no trailing [skill: …] tags, so the
        // runtime gate has nothing to check and never appears / never stops.
        let dir = unique_tmp("notags");
        std::fs::create_dir_all(&dir).unwrap();
        let card_path = dir.join("card.md");
        std::fs::write(&card_path, VALID_CARD).unwrap();
        let runtime_home = dir.join("runtime-home");

        let plan = run_task_card_inner(
            &card_path.to_string_lossy(),
            false,
            true,
            false,
            false,
            &repo_root_for_test(),
            &runtime_home,
        );
        assert!(plan.validation_passed);
        assert!(plan.skill_tags_gate.is_none());
        assert_eq!(
            plan.governance_status,
            GovernanceStatus::HostExecutionRequired
        );
        assert!(plan.host_execution_required);
        assert!(!plan.execution_performed);
        assert!(!plan.verification_performed);
        assert!(!plan.receipt_plan.generated);
        assert!(plan.receipt_plan.host_should_generate);
        assert_ne!(
            plan.gate_error_kind.as_deref(),
            Some("skill_tags_unavailable")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_only_skips_runtime_skill_tag_gate() {
        // check-only stops at the offline policy gate; the runtime skill-tag gate
        // belongs to the launch-plan path and must NOT run in check-only mode.
        let dir = unique_tmp("checkonly");
        std::fs::create_dir_all(&dir).unwrap();
        let card_path = dir.join("card.md");
        std::fs::write(
            &card_path,
            format!("{VALID_CARD}\n[skill: skill-creator]\n"),
        )
        .unwrap();
        let runtime_home = dir.join("runtime-home");

        let plan = run_task_card_inner(
            &card_path.to_string_lossy(),
            true, // check-only
            false,
            false,
            false,
            &repo_root_for_test(),
            &runtime_home,
        );
        assert_eq!(plan.mode, "check-only");
        assert!(plan.skill_tags_gate.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
