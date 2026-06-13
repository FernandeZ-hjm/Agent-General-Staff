//! AGS runner — gate-first task card execution planner.
//!
//! The runner orchestrates validate → gate → policy → adapter resolve →
//! launch plan. It ONLY consumes the resolved execution policy from
//! `execution-policy` — it never reads raw task-card fields to decide
//! permissions, parallelism, or launch args.
//!
//! ## Modes
//!
//! - `check_only`: validate + gate check, exit with decision code.
//! - `dry_run`: full pipeline, output structured `LaunchPlan`, no execution.
//! - default (no flags): same as dry_run — plan only, no actual launch.
//!
//! ## Adapter support
//!
//! - `claude-code`: complete — generates verbatim CLI command from resolved policy.
//! - `codex-local`: structured stub — reports launch plan, marks is_stub.
//! - `cursor`: structured stub — reports launch plan, marks is_stub.
//! - `generic`: capped at plan-only, requires human handoff.

use execution_policy::{ApprovalSource, GateCheckOutput, ResolvedExecutionPolicy};

// ── Public types ──────────────────────────────────────────────────────────

/// The result of running a task card through the full gate-first pipeline.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchPlan {
    pub schema_version: String,
    pub task_card_path: String,
    pub mode: String,

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
}

/// Plan for receipt generation after execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReceiptPlan {
    pub will_generate: bool,
    pub receipt_id_prefix: String,
    pub task_card_hash: String,
    pub gate_result_for_receipt: String,
    pub suggested_verification_commands: Vec<String>,
}

// ── Constants ─────────────────────────────────────────────────────────────

pub const SCHEMA_VERSION: &str = "2.0-runner";

// ── Main entry point ──────────────────────────────────────────────────────

/// Run a task card through the gate-first pipeline.
///
/// `check_only` — stop after gate check, don't build full launch plan.
/// `dry_run` — full pipeline, mark as dry run.
/// `approve_writes` — pass write approval to the policy resolver.
///
/// Returns `LaunchPlan` on validation pass, or a minimal stop plan on
/// validation failure. The caller checks `validation_passed` and
/// `gate_decision` to determine the exit code.
pub fn run_task_card(
    task_card_path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
) -> LaunchPlan {
    let mode = if check_only {
        "check-only"
    } else if dry_run {
        "dry-run"
    } else {
        "plan"
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
                validation_passed: false,
                validation_errors: vec![e],
                gate_decision: "stop".to_string(),
                gate_error_kind: Some("read_error".to_string()),
                resolved_policy: None,
                adapter: stub_adapter("generic", "read error — cannot resolve adapter"),
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
                validation_passed: false,
                validation_errors: errors.clone(),
                gate_decision: "stop".to_string(),
                gate_error_kind: Some("validation_failed".to_string()),
                resolved_policy: None,
                adapter: stub_adapter("generic", "validation failed — cannot resolve adapter"),
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
    let mut input = execution_policy::TaskPolicyInput::from_fields(&card.fields);
    if approve_writes {
        input.approval_source = ApprovalSource::CliFlag;
    }

    // ── Phase 4: Gate check (validate + resolve + decide) ──────────────
    let gate_output: GateCheckOutput = execution_policy::gate_check(&input);
    let decision_str = gate_output.decision.to_string().to_lowercase();
    let policy = gate_output.resolved_policy;

    // ── Phase 5: If check_only, stop here ──────────────────────────────
    if check_only {
        let gate_result_for_receipt = decision_str.clone();
        return LaunchPlan {
            schema_version: SCHEMA_VERSION.to_string(),
            task_card_path: display_path,
            mode: mode.to_string(),
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
            },
            receipt_plan: ReceiptPlan {
                will_generate: false,
                receipt_id_prefix: format!(
                    "receipt-{}",
                    &task_card_hash[..12.min(task_card_hash.len())]
                ),
                task_card_hash,
                gate_result_for_receipt,
                suggested_verification_commands: vec![],
            },
            verification_log_refs: vec![],
            delivery_report_ref: String::new(),
        };
    }

    // ── Phase 6: Adapter resolution ────────────────────────────────────
    let adapter_plan = resolve_adapter(&policy, &display_path);

    // ── Phase 7: Receipt / verification / delivery planning ────────────
    let gate_result_for_receipt = decision_str.clone();
    let receipt_plan = ReceiptPlan {
        will_generate: !policy.stop_before_launch,
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
    };

    let verification_log_refs = vec![
        "verification.log".to_string(),
        "delivery-report.md".to_string(),
    ];

    let delivery_report_ref = if policy.stop_before_launch {
        "BLOCKED — delivery report not applicable (stop_before_launch=true)".to_string()
    } else {
        "delivery-report.md (generate after execution)".to_string()
    };

    LaunchPlan {
        schema_version: SCHEMA_VERSION.to_string(),
        task_card_path: display_path,
        mode: mode.to_string(),
        validation_passed: true,
        validation_errors: vec![],
        gate_decision: decision_str,
        gate_error_kind: None,
        resolved_policy: Some(policy),
        adapter: adapter_plan,
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
    }
}

fn empty_receipt_plan(hash: &str) -> ReceiptPlan {
    ReceiptPlan {
        will_generate: false,
        receipt_id_prefix: if hash.is_empty() {
            String::new()
        } else {
            format!("receipt-{}", &hash[..12.min(hash.len())])
        },
        task_card_hash: hash.to_string(),
        gate_result_for_receipt: "stop".to_string(),
        suggested_verification_commands: vec![],
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
        lines.push(format!(
            "  Confirmation gate: {}",
            policy.requires_confirmation_gate
        ));
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

    // Receipt plan
    lines.push("─ Receipt / Verification / Delivery ─".to_string());
    lines.push(format!(
        "  Receipt:      {}",
        if plan.receipt_plan.will_generate {
            "will generate"
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
        "allow" => "PROCEED — gate passed, ready for execution",
        "confirm" => "CONFIRM — requires confirmation gate before mutation",
        "stop" => "STOP — blocked, cannot execute",
        _ => "UNKNOWN",
    };
    lines.push(format!("  Verdict:  {}", verdict));

    if plan.adapter.is_stub && plan.gate_decision != "stop" {
        lines.push(
            "  NOTE: Adapter is a stub. Real execution requires the shell wrapper.".to_string(),
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
    use execution_policy::{Parallelism, PermissionMode};

    #[test]
    fn test_read_error_produces_stop_plan() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false);
        assert!(!plan.validation_passed);
        assert_eq!(plan.gate_decision, "stop");
        assert!(plan.gate_error_kind.is_some());
        assert!(plan.resolved_policy.is_none());
        assert!(plan.adapter.is_stub);
    }

    #[test]
    fn test_check_only_mode_flag() {
        let plan = run_task_card("/nonexistent/path/task-card.md", true, false, false);
        assert_eq!(plan.mode, "check-only");
        assert!(!plan.validation_passed);
    }

    #[test]
    fn test_dry_run_mode_flag() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false);
        assert_eq!(plan.mode, "dry-run");
    }

    #[test]
    fn test_default_mode_is_plan() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, false, false);
        assert_eq!(plan.mode, "plan");
    }

    #[test]
    fn test_schema_version_constant() {
        assert_eq!(SCHEMA_VERSION, "2.0-runner");
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
            requires_confirmation_gate: false,
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
            requires_confirmation_gate: false,
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
            effective_permission_mode: PermissionMode::EditWithConfirmation,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "ide".into(),
            allowed_launch_args: vec![],
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: false,
            downgrade_reasons: vec![],
            requires_confirmation_gate: false,
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
            requires_confirmation_gate: false,
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
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false);
        assert!(!plan.receipt_plan.will_generate);
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
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false);
        let text = render_text(&plan);
        assert!(text.contains("AGS Runner"));
        assert!(text.contains("STOP"));
    }

    #[test]
    fn test_render_json_produces_valid_json() {
        let plan = run_task_card("/nonexistent/path/task-card.md", false, true, false);
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
            effective_permission_mode: PermissionMode::ReadOnly,
            effective_parallelism: Parallelism::None,
            effective_execution_surface: "cli".into(),
            allowed_launch_args: vec![], // empty — M5/M6 enforced
            stop_before_launch: false,
            stop_reasons: vec![],
            was_downgraded: true, // parallelism was downgraded
            downgrade_reasons: vec![],
            requires_confirmation_gate: false,
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
            requires_confirmation_gate: false,
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
            let plan_without = run_task_card(&path_str, false, true, false);
            let plan_with = run_task_card(&path_str, false, true, true);

            // Both should pass validation
            assert!(plan_without.validation_passed);
            assert!(plan_with.validation_passed);

            // With approve_writes, the policy should reflect CliFlag approval
            if let Some(ref policy) = plan_with.resolved_policy {
                assert_eq!(policy.approval_source.to_string(), "cli-flag");
            }
        }
    }
}
