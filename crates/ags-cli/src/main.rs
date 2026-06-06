//! Agent Governance Suite — Public CLI binary entry point.
//!
//! ## Commands
//!
//! - `ags task validate`          Validate task cards
//! - `ags policy resolve`         Resolve execution policy
//! - `ags policy explain`         Explain policy decisions
//! - `ags policy check`           Validate + resolve, exit with decision
//! - `ags sync check`             Multi-project protocol drift checker
//! - `ags doctor`                 Suite health diagnostics
//! - `ags bootstrap --dry-run`    Bootstrap dry-run simulation
//! - `ags bootstrap --apply`      Bootstrap a target directory
//! - `ags project detect`         Detect project identity and AGS integration
//! - `ags protocol status`        Check protocol file status
//! - `ags agent instructions`     Export agent-specific project instructions
//! - `ags session preflight`      Aggregated agent wake-up check (kernel activation)
//! - `ags verify`                 Scoped verification checks
//!
//! ## M0 Flat Commands (hidden backward-compatible aliases)
//!
//! - `ags task-card-validator`    → `ags task validate`
//! - `ags resolve-policy`         → `ags policy resolve`
//! - `ags workflow-sync-check`    → `ags sync check`
//! - `ags suite-doctor`           → `ags doctor`
//! - `ags bootstrap-dry-run`      → `ags bootstrap --dry-run`

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

// ── CLI root ──────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "ags",
    about = "Agent Governance Suite CLI",
    version = env!("CARGO_PKG_VERSION"),
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

// ── M1 Object Command Sub-enums ───────────────────────────────────────────

#[derive(Subcommand)]
enum TaskAction {
    /// Validate one or more task cards
    Validate {
        /// Task card files to validate (use "-" for stdin)
        paths: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Resolve execution policy for a validated task card (read-only).
    Resolve {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Explicit approval for Heavy task writes (CLI flag).
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
    },

    /// Explain each policy decision with rule IDs, downgrades, stop reasons, and safety assertions.
    Explain {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Explicit approval for Heavy task writes (CLI flag).
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
    },

    /// Validate, resolve, and exit with decision: 0 = no stop, 1 = stop/validation fail.
    Check {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Explicit approval for Heavy task writes (CLI flag).
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
    },
}

#[derive(Subcommand)]
enum SyncAction {
    /// Multi-project protocol drift checker (read-only).
    Check {
        /// Source suite root (default: current directory)
        #[arg(long, default_value = ".")]
        source: PathBuf,

        /// Target name=path pairs, e.g. "stable=/path/to/stable" "public=/path/to/public"
        #[arg(long = "targets", value_name = "NAME=PATH", value_parser = parse_target)]
        targets: Vec<(String, PathBuf)>,

        /// Single target root (backward compatible).
        #[arg(long = "target")]
        target: Option<PathBuf>,

        /// Name for --target (default: "target")
        #[arg(long = "target-name", default_value = "target")]
        target_name: String,

        /// Path to a JSON allowlist file for legal difference classification.
        #[arg(long)]
        allowlist: Option<PathBuf>,

        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

// ── M2 Agent Awareness Command Sub-enums ─────────────────────────────────

#[derive(Subcommand)]
enum ProjectAction {
    /// Detect project identity and AGS integration status (read-only).
    Detect {
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

#[derive(Subcommand)]
enum ProtocolAction {
    /// Check protocol file status and governance requirements (read-only).
    Status {
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

#[derive(Subcommand)]
enum AgentAction {
    /// Export agent-specific project instructions (read-only).
    Instructions {
        /// Agent type: codex, claude-code, or cursor
        #[arg(long = "for", value_name = "AGENT", value_parser = ["codex", "claude-code", "cursor"])]
        for_agent: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Session preflight operations (M2 — kernel activation).
#[derive(Subcommand)]
enum SessionAction {
    /// Run aggregated session preflight for an agent (kernel activation entry point).
    Preflight {
        /// Agent type: codex, claude-code, or cursor
        #[arg(long = "for", value_name = "AGENT", value_parser = ["codex", "claude-code", "cursor"])]
        for_agent: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Verification operations.
#[derive(Subcommand)]
enum VerifyAction {
    /// Run verification checks for the given scope.
    Run {
        /// Verification scope: local, full, or release
        #[arg(long, default_value = "local", value_parser = ["local", "full", "release"])]
        scope: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
}

// ── Top-level Commands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum Commands {
    /// Task card operations
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Execution policy operations
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Workflow sync operations
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Suite health diagnostics.  Use --repair for actionable fixes.
    Doctor {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Perform repair actions (default: read-only diagnosis only).
        #[arg(long)]
        repair: bool,
        /// Dry-run: show what would be repaired without executing.
        #[arg(long)]
        dry_run: bool,
        /// Target directory (default: current directory).
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Bootstrap operations — plan, dry-run, and apply to a target.
    Bootstrap {
        /// Perform a dry run (no files are written).
        #[arg(long)]
        dry_run: bool,
        /// Apply bootstrap: write bootstrap payload to target directory.
        /// Requires --target.
        #[arg(long)]
        apply: bool,
        /// Target directory for bootstrap operations.
        #[arg(long)]
        target: Option<PathBuf>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    // ── M2 Agent Awareness commands ───────────────────────────────────
    /// Project discovery and AGS integration detection (M2)
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Protocol file status and governance requirements (M2)
    Protocol {
        #[command(subcommand)]
        action: ProtocolAction,
    },
    /// Export agent-specific project instructions (M2)
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },

    // ── Session operations (M2 — kernel activation) ──────────────────
    /// Session preflight — aggregated agent wake-up check (M2)
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    // ── Verify operations ────────────────────────────────────────────
    /// Run scoped verification checks — structured, machine-readable reports
    Verify {
        /// Verification scope: local, full, or release
        #[arg(long, default_value = "local", value_parser = ["local", "full", "release"])]
        scope: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[command(subcommand)]
        action: Option<VerifyAction>,
    },

    // ── M0 backward-compatible aliases (hidden from help) ──────────────
    /// Validate one or more task cards (alias for `task validate`)
    #[command(hide = true)]
    TaskCardValidator {
        /// Task card files to validate (use "-" for stdin)
        paths: Vec<String>,
    },
    /// Resolve execution policy (alias for `policy resolve`)
    #[command(hide = true)]
    ResolvePolicy {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Explicit approval for Heavy task writes
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
    },
    /// Multi-project protocol drift checker (alias for `sync check`)
    #[command(hide = true)]
    WorkflowSyncCheck {
        #[arg(long, default_value = ".")]
        source: PathBuf,
        #[arg(long = "targets", value_name = "NAME=PATH", value_parser = parse_target)]
        targets: Vec<(String, PathBuf)>,
        #[arg(long = "target")]
        target: Option<PathBuf>,
        #[arg(long = "target-name", default_value = "target")]
        target_name: String,
        #[arg(long)]
        allowlist: Option<PathBuf>,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Suite health diagnostics (alias for `doctor`)
    #[command(hide = true)]
    SuiteDoctor {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Bootstrap dry-run (alias for `bootstrap --dry-run`)
    #[command(hide = true)]
    BootstrapDryRun {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

// ── CLI argument helpers ──────────────────────────────────────────────────

/// Parse "name=path" target specifications.
fn parse_target(s: &str) -> Result<(String, PathBuf), String> {
    let (name, path) = s.split_once('=').ok_or_else(|| {
        format!("invalid target format: '{s}'. Expected NAME=PATH (e.g. stable=/path/to/stable)")
    })?;
    Ok((name.to_string(), PathBuf::from(path)))
}

// ── Shared helpers ────────────────────────────────────────────────────────

fn guard_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }

    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.exists() {
        if let Some(name) = existing.file_name() {
            missing.push(name.to_os_string());
        }
        match existing.parent() {
            Some(parent) => existing = parent,
            None => return absolute,
        }
    }

    let mut normalized = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for component in missing.iter().rev() {
        normalized.push(component);
    }
    normalized
}

/// Guard writable target: bootstrap --apply targets must not be suite roots.
/// In the public version, we check for WORKSPACE.md and AGENT_SUITE_PROTOCOL.md
/// as indicators of a protected suite root, rather than hardcoding private paths.
fn guard_writable_target(command: &str, target: &Path) {
    let target_path = guard_path(target);

    // Check for protocol markers that indicate a protected suite root
    if target_path.join("WORKSPACE.md").exists()
        || target_path.join("AGENT_SUITE_PROTOCOL.md").exists()
    {
        eprintln!(
            "{command}: refused — target appears to be a suite root: {}",
            target.display()
        );
        eprintln!("Write-mode operations must target a tempdir or non-suite directory.");
        std::process::exit(1);
    }
}

fn ensure_bootstrap_source_repo(source_repo: &Path) {
    let required = [
        "protocol/agent-task-protocol.md",
        "protocol/task-card-template.md",
        "protocol/runtime-adapters.md",
        "protocol/task-routing.md",
        "scripts/validate.sh",
        "scripts/run-task-card.sh",
    ];

    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|rel| !source_repo.join(rel).exists())
        .collect();

    if missing.is_empty() {
        return;
    }

    eprintln!(
        "ags bootstrap --apply: refused — source is not a complete AGS suite root: {}",
        source_repo.display()
    );
    eprintln!("Missing bootstrap payload source file(s):");
    for rel in missing {
        eprintln!("  - {rel}");
    }
    std::process::exit(1);
}

fn render_bootstrap_apply_json(
    plan: &bootstrap_dry_run::BootstrapPlan,
    report: &suite_doctor::HealthReport,
) -> String {
    let output = serde_json::json!({
        "schema_version": bootstrap_dry_run::SCHEMA_VERSION,
        "plan": plan,
        "apply_report": report,
    });
    serde_json::to_string_pretty(&output)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

/// Format a `ResolvedExecutionPolicy` as human-readable text.
fn format_policy_text(policy: &execution_policy::ResolvedExecutionPolicy) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Resolved Execution Policy".to_string());
    lines.push("=========================".to_string());
    lines.push(format!("Executor:          {}", policy.executor));
    lines.push(format!("Runtime adapter:   {}", policy.runtime_adapter));
    lines.push(format!(
        "Permission mode:   {}",
        policy.effective_permission_mode
    ));
    lines.push(format!(
        "Parallelism:       {}",
        policy.effective_parallelism
    ));
    lines.push(format!(
        "Exec surface:      {}",
        policy.effective_execution_surface
    ));
    lines.push(format!("Execution effort:  {}", policy.execution_effort));
    lines.push(format!("Exhaustive mode:   {}", policy.is_exhaustive_mode));
    lines.push(String::new());

    let args_str = if policy.allowed_launch_args.is_empty() {
        "(none)".to_string()
    } else {
        policy.allowed_launch_args.join(" ")
    };
    lines.push(format!("Launch args:       {}", args_str));

    lines.push(format!("Stop before launch: {}", policy.stop_before_launch));
    if !policy.stop_reasons.is_empty() {
        lines.push("Stop reasons:".to_string());
        for (i, reason) in policy.stop_reasons.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, reason));
        }
    }

    lines.push(format!(
        "Requires confirmation gate: {}",
        policy.requires_confirmation_gate
    ));
    lines.push(format!("Approval source:   {}", policy.approval_source));
    lines.push(String::new());

    if policy.was_downgraded {
        lines.push("Downgrades:".to_string());
        for (i, reason) in policy.downgrade_reasons.iter().enumerate() {
            lines.push(format!("  {}. {}", i + 1, reason));
        }
    } else {
        lines.push("Downgrades:        none".to_string());
    }

    lines.join("\n")
}

/// Format a PolicyExplainOutput as human-readable text.
fn format_explain_text(
    output: &execution_policy::PolicyExplainOutput,
    display_path: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Policy Explanation".to_string());
    lines.push("==================".to_string());
    lines.push(format!("Task card:  {}", display_path));
    lines.push(format!("Schema:     {}", output.schema_version));
    lines.push(format!("Executor:   {}", output.task_summary.executor));
    lines.push(format!("Task level: {}", output.task_summary.task_level));
    lines.push(format!(
        "Permission: {}",
        output.task_summary.permission_mode
    ));
    lines.push(String::new());

    lines.push("Rule-by-Rule Explanation".to_string());
    lines.push("-----------------------".to_string());
    for explanation in &output.explanations {
        let field_note = match &explanation.field {
            Some(f) => format!(" [{}]", f),
            None => String::new(),
        };
        lines.push(format!(
            "  [{}] {} — {}{}",
            explanation.rule_id, explanation.decision, explanation.rule_name, field_note
        ));
        lines.push(format!("        {}", explanation.detail));
    }
    lines.push(String::new());

    lines.push("Safety Assertions".to_string());
    lines.push("-----------------".to_string());
    for (i, assertion) in output.safety_assertions.iter().enumerate() {
        lines.push(format!("  {}. {}", i + 1, assertion));
    }
    lines.push(String::new());

    lines.push("Resolved Execution Policy".to_string());
    lines.push("=========================".to_string());
    lines.push(format_policy_text(&output.resolved_policy));

    lines.join("\n")
}

// ── Shared dispatch functions ─────────────────────────────────────────────

/// Shared dispatch: `task validate` / `task-card-validator`
fn cmd_task_validate(paths: &[String]) {
    let paths: Vec<String> = if paths.is_empty() {
        vec!["-".to_string()]
    } else {
        paths.to_vec()
    };
    let ok = task_card_validator::validate_files(&paths);
    if !ok {
        std::process::exit(1);
    }
}

/// Shared dispatch: `policy resolve` / `resolve-policy`
fn cmd_policy_resolve(path: &str, format: &str, approve_writes: bool) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: read failed — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: read failed — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    // Phase 1: validate and parse
    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
            std::process::exit(1);
        }
    };

    // Phase 2: build policy input from parsed fields
    let mut input = execution_policy::TaskPolicyInput::from_fields(&card.fields);
    if approve_writes {
        input.approval_source = execution_policy::ApprovalSource::CliFlag;
    }

    // Phase 3: resolve execution policy
    let policy = execution_policy::resolve_policy(input);

    // Phase 4: output
    match format {
        "json" => match serde_json::to_string_pretty(&policy) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_policy_text(&policy));
        }
    }
}

/// Read a task card (file or stdin) and validate+parse it.
fn read_and_validate_task_card(
    path: &str,
) -> (String, task_card_validator::ParsedTaskCard, String) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: read failed — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: read failed — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
            std::process::exit(1);
        }
    };

    (content, card, display_path)
}

/// Build a TaskPolicyInput from parsed fields + optional --approve-writes.
fn build_policy_input(
    fields: &std::collections::HashMap<String, String>,
    approve_writes: bool,
) -> execution_policy::TaskPolicyInput {
    let mut input = execution_policy::TaskPolicyInput::from_fields(fields);
    if approve_writes {
        input.approval_source = execution_policy::ApprovalSource::CliFlag;
    }
    input
}

/// Shared dispatch: `policy explain`
fn cmd_policy_explain(path: &str, format: &str, approve_writes: bool) {
    let (_, card, display_path) = read_and_validate_task_card(path);
    let input = build_policy_input(&card.fields, approve_writes);
    let output = execution_policy::explain_policy(&input);

    match format {
        "json" => match serde_json::to_string_pretty(&output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_explain_text(&output, &display_path));
        }
    }
}

/// Shared dispatch: `policy check` — exit 0 if no stop, 1 if stop/validation.
fn cmd_policy_check(path: &str, format: &str, approve_writes: bool) {
    let (_, card, _display_path) = read_and_validate_task_card(path);
    let input = build_policy_input(&card.fields, approve_writes);
    let policy = execution_policy::resolve_policy(input);

    match format {
        "json" => match serde_json::to_string_pretty(&policy) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_policy_text(&policy));
        }
    }

    if policy.stop_before_launch {
        std::process::exit(1);
    }
}

/// Shared dispatch: `sync check` / `workflow-sync-check`
fn cmd_sync_check(
    source: PathBuf,
    targets: Vec<(String, PathBuf)>,
    target: Option<PathBuf>,
    target_name: String,
    allowlist: Option<PathBuf>,
    format: &str,
) {
    let mut all_targets = targets;

    if let Some(target_root) = target {
        all_targets.push((target_name, target_root));
    }

    // In public version, no default target — user must specify targets explicitly
    if all_targets.is_empty() {
        eprintln!("ags sync check: no targets specified.");
        eprintln!("  Usage: ags sync check --targets name=/path/to/target ...");
        eprintln!("  Example: ags sync check --targets stable=/path/to/stable");
        std::process::exit(2);
    }

    let target_configs: Vec<workflow_sync_check::TargetConfig> = all_targets
        .into_iter()
        .map(|(name, root)| {
            let kind = match name.as_str() {
                "stable" => workflow_sync_check::ProjectKind::Stable,
                "public" | "public-core-only" => workflow_sync_check::ProjectKind::PublicCoreOnly,
                _ => workflow_sync_check::ProjectKind::Custom(name.clone()),
            };
            workflow_sync_check::TargetConfig { root, name, kind }
        })
        .collect();

    let report_format = match format {
        "json" => workflow_sync_check::ReportFormat::Json,
        _ => workflow_sync_check::ReportFormat::Text,
    };

    let options = workflow_sync_check::CheckOptions {
        source_root: source,
        source_name: "private".to_string(),
        targets: target_configs,
        allowlist_path: allowlist,
    };

    let ok = workflow_sync_check::run_cli(options, report_format);
    if !ok {
        std::process::exit(1);
    }
}

/// Shared dispatch: `doctor` / `suite-doctor`
fn cmd_doctor(format: &str, repair: bool, dry_run: bool, target: &Path) {
    if !repair {
        // Read-only diagnosis
        let report = suite_doctor::run(target);
        match format {
            "json" => println!("{}", suite_doctor::render_json(&report)),
            _ => println!("{}", suite_doctor::render_text(&report)),
        }
        std::process::exit(report.exit_code());
    }

    if dry_run {
        // Repair dry-run: show what would be repaired
        let plan = suite_doctor::repair_plan(target);
        match format {
            "json" => println!("{}", suite_doctor::render_repair_plan_json(&plan)),
            _ => println!("{}", suite_doctor::render_repair_plan_text(&plan)),
        }
        std::process::exit(plan.exit_code());
    }

    // Actual repair (safe items only)
    guard_writable_target("ags doctor --repair", target);
    let result = suite_doctor::repair(target);
    match format {
        "json" => println!("{}", suite_doctor::render_repair_json(&result)),
        _ => println!("{}", suite_doctor::render_repair_text(&result)),
    }
    std::process::exit(result.exit_code());
}

/// Shared dispatch: `bootstrap --apply`
fn cmd_bootstrap_apply(target: &Path, format: &str) {
    let source_repo = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ensure_bootstrap_source_repo(&source_repo);

    let plan = bootstrap_dry_run::plan(&source_repo, target);

    if format != "json" {
        println!("{}", bootstrap_dry_run::render_plan_text(&plan));
    }

    let report = bootstrap_dry_run::apply(&source_repo, &plan);

    match format {
        "json" => println!("{}", render_bootstrap_apply_json(&plan, &report)),
        _ => {
            println!();
            println!("{}", suite_doctor::render_text(&report));
        }
    }

    if !report.passed() {
        std::process::exit(1);
    }
}

/// Shared dispatch: `bootstrap --dry-run` / `bootstrap-dry-run`
fn cmd_bootstrap_dry_run(format: &str) {
    cmd_bootstrap_dry_run_target(std::path::Path::new("."), format);
}

/// Shared dispatch: `bootstrap --dry-run --target <dir>`
fn cmd_bootstrap_dry_run_target(target: &Path, format: &str) {
    let report = bootstrap_dry_run::run(target);
    match format {
        "json" => println!("{}", suite_doctor::render_json(&report)),
        _ => println!("{}", suite_doctor::render_text(&report)),
    }
    std::process::exit(report.exit_code());
}

// ── M2 dispatch functions ──────────────────────────────────────────────────

/// Shared dispatch: `project detect`
fn cmd_project_detect(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "project detect: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let identity = project_discovery::detect_project(target);
    match format {
        "json" => println!("{}", project_discovery::render_json(&identity)),
        _ => println!(
            "{}",
            project_discovery::render_project_identity_text(&identity)
        ),
    }
    std::process::exit(project_discovery::project_detect_exit_code(&identity));
}

/// Shared dispatch: `protocol status`
fn cmd_protocol_status(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "protocol status: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let status = project_discovery::check_protocol_status(target);
    match format {
        "json" => println!("{}", project_discovery::render_json(&status)),
        _ => println!(
            "{}",
            project_discovery::render_protocol_status_text(&status)
        ),
    }
    std::process::exit(project_discovery::protocol_status_exit_code(&status));
}

/// Shared dispatch: `agent instructions`
fn cmd_agent_instructions(for_agent: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "agent instructions: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let agent_type = match project_discovery::AgentType::from_str(for_agent) {
        Ok(at) => at,
        Err(e) => {
            eprintln!("agent instructions: {}", e);
            std::process::exit(2);
        }
    };

    let instructions = project_discovery::generate_agent_instructions(target, &agent_type);
    match format {
        "json" => println!("{}", project_discovery::render_json(&instructions)),
        _ => println!(
            "{}",
            project_discovery::render_agent_instructions_text(&instructions)
        ),
    }
    std::process::exit(instructions.exit_code);
}

/// Shared dispatch: `session preflight`
fn cmd_session_preflight(for_agent: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "session preflight: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let agent_type = match project_discovery::AgentType::from_str(for_agent) {
        Ok(at) => at,
        Err(e) => {
            eprintln!("session preflight: {}", e);
            std::process::exit(2);
        }
    };

    let preflight = project_discovery::run_session_preflight(target, &agent_type);
    match format {
        "json" => println!("{}", project_discovery::render_json(&preflight)),
        _ => println!(
            "{}",
            project_discovery::render_session_preflight_text(&preflight)
        ),
    }
    std::process::exit(preflight.exit_code);
}

// ── Verify dispatch ────────────────────────────────────────────────────────

/// Shared dispatch: `verify` and backward-compatible `verify run`.
fn cmd_verify_run(scope: &str, format: &str, target: &Path) {
    if !target.exists() {
        eprintln!("verify: target does not exist — {}", target.display());
        std::process::exit(1);
    }

    let scope = match ags_verify::Scope::from_str(scope) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("verify: {}", e);
            std::process::exit(2);
        }
    };

    let report = ags_verify::run_verify(scope, target);

    match format {
        "json" => println!("{}", ags_verify::render_json(&report)),
        _ => println!("{}", ags_verify::render_text(&report)),
    }

    std::process::exit(report.exit_code());
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // ── M1 object commands ──
        Commands::Task { action } => match action {
            TaskAction::Validate { paths } => cmd_task_validate(&paths),
        },
        Commands::Policy { action } => match action {
            PolicyAction::Resolve {
                path,
                format,
                approve_writes,
            } => cmd_policy_resolve(&path, &format, approve_writes),
            PolicyAction::Explain {
                path,
                format,
                approve_writes,
            } => cmd_policy_explain(&path, &format, approve_writes),
            PolicyAction::Check {
                path,
                format,
                approve_writes,
            } => cmd_policy_check(&path, &format, approve_writes),
        },
        Commands::Sync { action } => match action {
            SyncAction::Check {
                source,
                targets,
                target,
                target_name,
                allowlist,
                format,
            } => cmd_sync_check(source, targets, target, target_name, allowlist, &format),
        },
        Commands::Doctor {
            format,
            repair,
            dry_run,
            target,
        } => cmd_doctor(&format, repair, dry_run, &target),
        Commands::Bootstrap {
            dry_run,
            apply,
            target,
            format,
        } => match (dry_run, apply) {
            (false, false) => {
                eprintln!("ags bootstrap: one of --dry-run or --apply is required.");
                eprintln!("  ags bootstrap --dry-run              Check this workspace");
                eprintln!("  ags bootstrap --apply --target <dir>  Bootstrap a target directory");
                std::process::exit(2);
            }
            (true, true) => {
                eprintln!("ags bootstrap: --dry-run and --apply are mutually exclusive.");
                std::process::exit(2);
            }
            (true, false) => {
                let t = target.as_deref().unwrap_or_else(|| Path::new("."));
                cmd_bootstrap_dry_run_target(t, &format);
            }
            (false, true) => {
                let t = match target {
                    Some(ref t) => t.clone(),
                    None => {
                        eprintln!("ags bootstrap: --apply requires --target.");
                        eprintln!("  ags bootstrap --apply --target /tmp/my-target");
                        std::process::exit(2);
                    }
                };
                guard_writable_target("ags bootstrap --apply", &t);
                cmd_bootstrap_apply(&t, &format);
            }
        },

        // ── M2 Agent Awareness commands ──
        Commands::Project { action } => match action {
            ProjectAction::Detect { target, format } => cmd_project_detect(&target, &format),
        },
        Commands::Protocol { action } => match action {
            ProtocolAction::Status { target, format } => cmd_protocol_status(&target, &format),
        },
        Commands::Agent { action } => match action {
            AgentAction::Instructions {
                for_agent,
                target,
                format,
            } => cmd_agent_instructions(&for_agent, &target, &format),
        },

        // ── Session operations (M2 — kernel activation) ──
        Commands::Session { action } => match action {
            SessionAction::Preflight {
                for_agent,
                target,
                format,
            } => cmd_session_preflight(&for_agent, &target, &format),
        },

        // ── Verify operations ──
        Commands::Verify {
            action,
            scope,
            format,
            target,
        } => match action {
            Some(VerifyAction::Run {
                scope,
                format,
                target,
            }) => cmd_verify_run(&scope, &format, &target),
            None => cmd_verify_run(&scope, &format, &target),
        },

        // ── M0 backward-compatible aliases (hidden from help) ──
        Commands::TaskCardValidator { paths } => cmd_task_validate(&paths),
        Commands::ResolvePolicy {
            path,
            format,
            approve_writes,
        } => cmd_policy_resolve(&path, &format, approve_writes),
        Commands::WorkflowSyncCheck {
            source,
            targets,
            target,
            target_name,
            allowlist,
            format,
        } => cmd_sync_check(source, targets, target, target_name, allowlist, &format),
        Commands::SuiteDoctor { format } => {
            cmd_doctor(&format, false, false, std::path::Path::new("."))
        }
        Commands::BootstrapDryRun { format } => cmd_bootstrap_dry_run(&format),
    }
}
