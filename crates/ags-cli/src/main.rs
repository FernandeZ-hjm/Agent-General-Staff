//! Agent Governance Suite — Public CLI binary entry point (full-blood edition).
//!
//! ## M1 Object Commands
//!
//! - `ags task validate`          Validate task cards
//! - `ags task compile`           Compile execution intent into canonical task card
//! - `ags task new`               Generate an empty task card template
//! - `ags policy resolve`         Resolve execution policy
//! - `ags policy explain`         Explain policy decisions
//! - `ags policy check`           Validate + resolve, exit with decision
//! - `ags gate check`             Runner-facing gate check (M3)
//! - `ags sync check`             Multi-project protocol drift checker
//! - `ags doctor`                 Suite health diagnostics
//! - `ags bootstrap --dry-run`    Bootstrap dry-run simulation
//! - `ags bootstrap --apply`      Bootstrap a target directory
//!
//! ## M2 Agent Awareness Commands
//!
//! - `ags project detect`         Detect project identity and AGS integration
//! - `ags protocol status`        Check protocol file status
//! - `ags agent instructions`     Export agent-specific project instructions
//! - `ags session preflight`      Aggregated agent wake-up check (kernel activation)
//! - `ags project integrate`      Incrementally merge AGS entry rules into project entry files
//!
//! ## Execution & Verification
//!
//! - `ags run`                    Gate-first task card execution pipeline
//! - `ags verify`                 Scoped verification checks
//!
//! ## Receipt / Compliance (M6)
//!
//! - `ags receipt generate`       Generate a task run receipt
//! - `ags receipt verify`         Verify receipt integrity
//! - `ags compliance check`       Check receipt compliance with policy gates
//!
//! ## Skill Governance
//!
//! - `ags skill scan`             Discover skill status from suite manifest
//! - `ags skill check`            Validate governance YAML consistency
//! - `ags skill propose`          Dry-run proposal for skill changes
//! - `ags skill install`          Install a recommended skill (requires confirmation)
//!
//! ## Capability Registry (M5)
//!
//! - `ags capability list`        List all discovered capabilities
//! - `ags capability show`        Show a specific capability by ID
//!
//! ## Operations
//!
//! - `ags archive`                Archive delivery report to memory directory
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

const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// Compile an execution intent into a canonical task card (M4).
    ///
    /// Reads a flexible intent file (or stdin with "-") and deterministically
    /// compiles it into the canonical compact task-card skeleton. This is a
    /// rule engine only — no AI calls, no free-form prompt generation.
    Compile {
        /// Intent file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Output mode: `card` prints only the compiled task card (pipeable);
        /// `report` prints the full compile report. Default: `report`.
        #[arg(long, default_value = "report", value_parser = ["card", "report"])]
        output: String,
        /// Check only: report if compilation is possible without producing card.
        #[arg(long, default_value_t = false)]
        check_only: bool,
        /// Task card explicitly requested by the user (hard gate).
        #[arg(long, default_value_t = false)]
        task_card_requested: bool,
    },
    /// Generate an empty task card template (compact or full).
    New {
        /// Card type: compact or full
        #[arg(long, default_value = "compact", value_parser = ["compact", "full"])]
        card_type: String,
        /// Write to file instead of stdout
        #[arg(long)]
        output: Option<PathBuf>,
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

/// Runner-facing gate operations (M3).
#[derive(Subcommand)]
enum GateAction {
    /// Run the gate check and output a runner-level decision.
    ///
    /// Outputs decision: allow|confirm|stop with embedded resolved policy.
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

/// Capability registry operations (M5).
#[derive(Subcommand)]
enum CapabilityAction {
    /// List all discovered capabilities.
    List {
        /// Project root path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Show details for a specific capability by ID.
    Show {
        /// Capability ID (e.g. "rust:task-card-validator", "policy:agent-task-protocol")
        name: String,
        /// Project root path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Receipt operations (M6).
#[derive(Subcommand)]
enum ReceiptAction {
    /// Generate a receipt from a task card.
    Generate {
        /// Task card file (use "-" for stdin)
        #[arg(long = "task-card")]
        task_card: String,
        /// Gate decision: allow, confirm, or stop
        #[arg(long, default_value = "allow", value_parser = ["allow", "confirm", "stop"])]
        gate_result: String,
        /// Optional gate reason
        #[arg(long)]
        gate_reason: Option<String>,
        /// Verification results in "command:exit_code" format (repeatable)
        #[arg(long = "verification", value_name = "CMD:EXIT_CODE")]
        verifications: Vec<String>,
        /// Delivery report file path (optional)
        #[arg(long)]
        delivery_report: Option<String>,
        /// Review gate status (Light/Medium/Heavy completion state)
        #[arg(long)]
        review_gate_status: Option<String>,
        /// Metadata key=value pairs for extensibility (repeatable)
        #[arg(long = "metadata", value_name = "KEY=VALUE")]
        metadata: Vec<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify a receipt's integrity.
    Verify {
        /// Receipt file path
        path: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Compliance check operations (M6).
#[derive(Subcommand)]
enum ComplianceAction {
    /// Check a receipt for compliance with policy gates.
    Check {
        /// Receipt file path
        path: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Hook management — install, check, uninstall stop-archive hook.
///
/// Never auto-modifies user config files. Install writes a hook config
/// snippet for manual review and application.
#[derive(Subcommand)]
enum HookAction {
    /// Install a hook — shows plan (dry-run) or writes config snippet.
    Install {
        /// Hook name (currently: stop-archive)
        #[arg(long = "hook", default_value = "stop-archive")]
        hook_name: String,
        /// Dry-run: show plan without writing
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Confirm — write the hook config snippet
        #[arg(long, default_value_t = false)]
        confirm: bool,
        /// Target directory for hook snippet (default: .claude/)
        #[arg(long)]
        target: Option<PathBuf>,
        /// Output format
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Check whether a hook is installed.
    Check {
        /// Hook name (currently: stop-archive)
        #[arg(long = "hook", default_value = "stop-archive")]
        hook_name: String,
        /// Output format
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Uninstall a hook — removes the generated snippet file.
    Uninstall {
        /// Hook name (currently: stop-archive)
        #[arg(long = "hook", default_value = "stop-archive")]
        hook_name: String,
        /// Confirm uninstall
        #[arg(long, default_value_t = false)]
        confirm: bool,
        /// Target directory (default: .claude/)
        #[arg(long)]
        target: Option<PathBuf>,
        /// Output format
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Skill governance operations — read-only inventory, dry-run proposal,
/// and confirmed install.
#[derive(Subcommand)]
enum SkillAction {
    /// Scan the suite manifest and governance files for skill status.
    Scan {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Validate governance YAML files for schema compliance.
    Check {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Propose skill changes — dry-run ONLY, no files modified.
    Propose {
        /// Action: adopt, enable, or disable
        #[arg(long, value_parser = ["adopt", "enable", "disable"])]
        action: String,
        /// Skill name to act on
        #[arg(long)]
        skill: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Install a recommended skill (requires explicit --confirm).
    ///
    /// First run without --confirm to see the installation plan (what skills,
    /// source, target directory, risk summary). Then re-run with --confirm to
    /// actually install.
    ///
    /// Install mode:
    /// - `template` (default): generates a SKILL.md template with frontmatter and
    ///   stub directories. Clearly labeled as TEMPLATE INSTALL — user must copy
    ///   the real SKILL.md and other files from the source repository.
    /// - `full`: copies a complete skill package from --source-dir. Requires
    ///   --source-dir pointing to a local directory containing SKILL.md.
    Install {
        /// Skill name to install, or "recommended" for all recommended skills
        #[arg(long)]
        skill: String,
        /// Confirm installation — required to actually write to skills directory
        #[arg(long, default_value_t = false)]
        confirm: bool,
        /// Dry-run: show what would be installed without installing
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Target skills directory (default: $HOME/.agents/skills)
        #[arg(long)]
        target: Option<PathBuf>,
        /// Install mode: template (generates skeleton) or full (copies from source-dir)
        #[arg(long, default_value = "template", value_parser = ["template", "full"])]
        mode: String,
        /// Source directory for full install mode — must contain SKILL.md
        #[arg(long)]
        source_dir: Option<PathBuf>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Adopt a skill — mark it as adopted in governance log.
    /// Requires --apply to write the adoption log.
    Adopt {
        /// Skill name
        #[arg(long)]
        skill: String,
        /// Apply the adoption (write to governance log)
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Ignore a skill — add it to the ignore list.
    /// Requires --apply to write the ignore list.
    Ignore {
        /// Skill name
        #[arg(long)]
        skill: String,
        /// Reason for ignoring
        #[arg(long)]
        reason: Option<String>,
        /// Apply the ignore (write to ignore list)
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// Output format: text (default) or json
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
    /// Incrementally merge AGS managed entry blocks into AGENTS.md and CLAUDE.md.
    ///
    /// Existing user content is preserved. Existing AGS managed blocks are
    /// updated in place. Writes require --confirm; otherwise this is a dry-run.
    Integrate {
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Dry-run: show intended changes without writing
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Confirm writing managed blocks and backups
        #[arg(long, default_value_t = false)]
        confirm: bool,
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

/// MCP server operations.
#[derive(Subcommand)]
enum McpAction {
    /// Start AGS MCP server on stdio.
    Serve {
        /// Transport protocol; v1 supports stdio only.
        #[arg(long, default_value = "stdio", value_parser = ["stdio"])]
        transport: String,
    },
}

// ── Top-level Commands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum Commands {
    /// Initialize local AGS host entrypoints: /ags, Codex command skills, MCP snippets.
    Setup {
        /// Confirm writes to AGS-owned command/snippet files.
        #[arg(long, default_value_t = false)]
        yes: bool,
        /// Replace existing AGS-owned command files.
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Runtime/share target for generated MCP snippets.
        #[arg(long)]
        target: Option<PathBuf>,
        /// Register AGS MCP in Claude Code using `claude mcp add`.
        #[arg(long, default_value_t = false)]
        register_claude: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Onboard the current project into AGS governance.
    Init {
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Dry-run only; do not write managed blocks or memory templates.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
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

    // ── M3 Gate operations ────────────────────────────────────────────
    /// Gate check — runner-facing gate decision (M3)
    Gate {
        #[command(subcommand)]
        action: GateAction,
    },

    // ── M5 Capability Registry ────────────────────────────────────────
    /// Capability discovery and registry operations (M5)
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },

    // ── M6 Receipt / Compliance ──────────────────────────────────────
    /// Receipt generation and verification operations (M6)
    Receipt {
        #[command(subcommand)]
        action: ReceiptAction,
    },
    /// Compliance checking against policy gates (M6)
    Compliance {
        #[command(subcommand)]
        action: ComplianceAction,
    },

    // ── Hook management ──────────────────────────────────────────────
    /// Stop-archive hook: install, check, uninstall.
    /// Never auto-modifies user config files.
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },

    // ── Skill governance ─────────────────────────────────────────────
    /// Skill governance — scan, check, propose, and confirmed install
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },

    // ── MCP host adapter ─────────────────────────────────────────────
    /// Start AGS MCP host initialization adapter.
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },

    // ── Runner ───────────────────────────────────────────────────────
    /// Run a task card through the gate-first execution pipeline.
    ///
    /// Flow: validate → gate → policy → adapter resolve → launch plan.
    /// --check-only stops after gate check. --dry-run outputs the full plan.
    Run {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Stop after gate check
        #[arg(long, default_value_t = false)]
        check_only: bool,
        /// Full pipeline, output launch plan, do not execute
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Pass write approval to the policy resolver
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    // ── Archive ──────────────────────────────────────────────────────
    /// Archive a delivery report and task summary to the memory directory.
    Archive {
        /// Delivery report file path
        #[arg(long = "delivery-report")]
        delivery_report: Option<PathBuf>,
        /// Task card file (for hash recording)
        #[arg(long = "task-card")]
        task_card: Option<PathBuf>,
        /// Verification results JSON file
        #[arg(long)]
        verification_results: Option<PathBuf>,
        /// Receipt JSON file (optional)
        #[arg(long)]
        receipt: Option<PathBuf>,
        /// Task summary (one-line)
        #[arg(long)]
        summary: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
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

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn default_setup_target() -> PathBuf {
    std::env::var_os("AGS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".ags").join("public-runtime"))
}

fn claude_ags_command_path() -> PathBuf {
    home_dir().join(".claude").join("commands").join("ags.md")
}

fn codex_ags_named_skill_dir(name: &str) -> PathBuf {
    home_dir().join(".codex").join("skills").join(name)
}

fn codex_ags_named_skill_path(name: &str) -> PathBuf {
    codex_ags_named_skill_dir(name).join("SKILL.md")
}

fn codex_ags_named_skill_agent_metadata_path(name: &str) -> PathBuf {
    codex_ags_named_skill_dir(name)
        .join("agents")
        .join("openai.yaml")
}

fn retired_codex_ags_skill_dirs() -> Vec<PathBuf> {
    vec![
        codex_ags_named_skill_dir("ags"),
        codex_ags_named_skill_dir("ags-preflight"),
        codex_ags_named_skill_dir("ags-verify"),
    ]
}

#[derive(Debug, Clone)]
struct SetupFile {
    path: PathBuf,
    description: String,
    content: String,
    executable: bool,
}

fn claude_ags_command_content() -> String {
    format!(
        r#"---
description: AGS one-command setup, project onboarding, and governance
argument-hint: [setup|init|preflight|doctor|verify|request...]
---

# AGS

Route by the first token in `$ARGUMENTS`.

## `/ags setup`

Initialize this machine into AGS:

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
ags setup --yes --force --register-claude
ags doctor --target .
```

Expected result: `ags`, `/ags`, Codex AGS command skills, and AGS MCP are ready.

## `/ags init`

Onboard the current repository into AGS governance:

```bash
ags init --target .
ags session preflight --for claude-code --target .
```

Aliases: `/ags onboard`, `/ags manage`, `/ags 纳管`.

## Other routes

- Empty or `preflight`: call AGS MCP `ags_preflight` first. If MCP is unavailable, run `ags session preflight --for claude-code --target .`.
- `doctor`: run `ags doctor --target .` and summarize failures first.
- `verify`: run `ags verify --scope local --target .` and summarize evidence.
- Any other text: treat it as the user request. For AGS scenarios, AGS MCP `ags_preflight` is mandatory first; CLI preflight is only a fallback. Do not generate an executable task card until the user explicitly asks for one.

Current AGS version expected by this command: {AGS_VERSION}.
"#
    )
}

fn codex_ags_command_skill_specs() -> &'static [(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)] {
    &[
        (
            "ags-setup",
            "AGS Setup",
            "初始化本机 AGS 环境",
            "用 $ags-setup 初始化本机 AGS 环境。",
            "初始化本机 AGS 环境：运行 `ags setup --yes --force`，确保 `/ags`、Codex AGS command skills 和 `ags mcp serve --transport stdio` 可用",
        ),
        (
            "ags-init",
            "AGS Init",
            "纳管当前项目",
            "用 $ags-init 纳管当前项目。",
            "纳管当前仓库：运行 `ags init --target .`，然后运行 `ags session preflight --for codex --target .`",
        ),
        (
            "ags-skill",
            "AGS Skill",
            "管理第三方技能",
            "用 $ags-skill 管理第三方技能。",
            "管理第三方技能：运行 `ags skill` 查看概览，或运行 `ags skill scan`、`ags skill check`、`ags skill propose --action adopt --skill <name>` 生成纳管建议",
        ),
        (
            "ags-doctor",
            "AGS Doctor",
            "诊断 AGS 状态",
            "用 $ags-doctor 诊断 AGS 状态。",
            "诊断 AGS 安装和项目状态：运行 `ags doctor --target .` 并优先汇总失败项",
        ),
    ]
}

fn codex_ags_command_skill_content(name: &str, display_name: &str, summary: &str) -> String {
    let route = name.strip_prefix("ags-").unwrap_or(name);
    format!(
        r#"---
name: "{name}"
description: "当用户提到 /ags {route}、{display_name}、AGS {route}，或需要{summary}时使用。"
---

# {display_name}

这是 Codex 顶层 AGS 命令技能，用来把明确的 AGS 操作路由到已安装的 `ags` CLI、AGS MCP 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先调用 AGS MCP `ags_preflight`。如果 MCP 不可用，才使用 CLI fallback：

```bash
ags session preflight --for codex --target .
```

如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。

## 路由

{summary}.

## 安全边界

不要绕过 AGS 做临时初始化。除非用户明确要求生成任务卡，否则不要生成可执行任务卡。

此技能期望的 AGS 版本：{AGS_VERSION}。
"#
    )
}

fn codex_ags_command_skill_agent_metadata_content(
    display_name: &str,
    short_description: &str,
    default_prompt: &str,
) -> String {
    format!(
        r#"interface:
  display_name: "{display_name}"
  short_description: "{short_description}"
  default_prompt: "{default_prompt}"

policy:
  allow_implicit_invocation: true
"#
    )
}

fn setup_files(target: &Path) -> Vec<SetupFile> {
    let target_s = target.to_string_lossy();
    let ags_mcp_json = format!(
        r#"{{
  "mcpServers": {{
    "ags": {{
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {{
        "AGS_RUNTIME_HOME": "{target_s}"
      }}
    }}
  }},
  "initialization_gate": {{
    "mandatory_first_tool": "ags_preflight",
    "failed_preflight_opens_gate": false
  }}
}}
"#
    );
    let codex_snippet = format!(
        r#"# AGS MCP host initialization adapter
# Merge this snippet into ~/.codex/config.toml after review.
[mcp_servers.ags]
command = "ags"
args = ["mcp", "serve", "--transport", "stdio"]

[mcp_servers.ags.env]
AGS_RUNTIME_HOME = "{target_s}"
"#
    );
    let claude_snippet = format!(
        r#"{{
  "mcpServers": {{
    "ags": {{
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {{
        "AGS_RUNTIME_HOME": "{target_s}"
      }}
    }}
  }}
}}
"#
    );
    let workbuddy_snippet = format!(
        r#"{{
  "mcps": [
    {{
      "name": "ags",
      "transport": "stdio",
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "role": "host_initialization_adapter",
      "mandatory_first": true,
      "env": {{
        "AGS_RUNTIME_HOME": "{target_s}"
      }}
    }}
  ]
}}
"#
    );
    let launcher = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport AGS_RUNTIME_HOME={}\nexec ags mcp serve --transport stdio\n",
        shell_quote(target)
    );
    let readme = format!(
        "# AGS Public Runtime\n\n\
This directory was generated by `ags setup`.\n\n\
## Commands\n\n\
- MCP server: `ags mcp serve --transport stdio`\n\
- Doctor: `ags doctor`\n\
- Project onboarding: `/ags init` or `ags init --target .`\n\n\
## Host snippets\n\n\
Review files in `hosts/` before merging them into host-specific global config.\n\
AGS scenarios must call `ags_preflight` before any other AGS tool.\n\n\
## Claude Code slash command\n\n\
`ags setup --yes` refreshes `/ags` at `~/.claude/commands/ags.md`.\n\
\n\
## Codex skills\n\n\
`ags setup --yes` installs visible top-level command skills: `$ags-setup`, `$ags-init`, `$ags-skill`, and `$ags-doctor`.\n\
Retired visible skills (`$ags`, `$ags-preflight`, `$ags-verify`) are removed when `--force` is used.\n\
\n\
## Boundary\n\n\
AGS MCP is the mandatory governance interface. Advisory memory MCPs, when installed separately, remain parallel peers and are not proxied by AGS MCP.\n"
    );

    let mut files = vec![
        SetupFile {
            path: target.join("README.md"),
            description: "operator notes for this public runtime".to_string(),
            content: readme,
            executable: false,
        },
        SetupFile {
            path: target.join("mcp/ags.mcp.json"),
            description: "generic MCP registration snippet for AGS host adapter".to_string(),
            content: ags_mcp_json,
            executable: false,
        },
        SetupFile {
            path: target.join("hosts/codex.config.snippet.toml"),
            description: "Codex MCP config snippet".to_string(),
            content: codex_snippet,
            executable: false,
        },
        SetupFile {
            path: target.join("hosts/claude-code.mcp.snippet.json"),
            description: "Claude Code MCP snippet".to_string(),
            content: claude_snippet,
            executable: false,
        },
        SetupFile {
            path: target.join("hosts/workbuddy.mcp.snippet.json"),
            description: "WorkBuddy MCP config snippet".to_string(),
            content: workbuddy_snippet,
            executable: false,
        },
        SetupFile {
            path: target.join("bin/ags-mcp-stdio.sh"),
            description: "portable launcher for AGS MCP stdio server".to_string(),
            content: launcher,
            executable: true,
        },
        SetupFile {
            path: claude_ags_command_path(),
            description: "Claude Code user slash command for AGS governance".to_string(),
            content: claude_ags_command_content(),
            executable: false,
        },
    ];

    for (name, display_name, short_description, default_prompt, summary) in
        codex_ags_command_skill_specs()
    {
        files.push(SetupFile {
            path: codex_ags_named_skill_path(name),
            description: format!("Codex AGS command skill: {name}"),
            content: codex_ags_command_skill_content(name, display_name, summary),
            executable: false,
        });
        files.push(SetupFile {
            path: codex_ags_named_skill_agent_metadata_path(name),
            description: format!("Codex AGS command skill UI metadata: {name}"),
            content: codex_ags_command_skill_agent_metadata_content(
                display_name,
                short_description,
                default_prompt,
            ),
            executable: false,
        });
    }

    files
}

fn render_setup_plan_text(target: &Path, yes: bool, force: bool, files: &[SetupFile]) -> String {
    let mut lines = vec![
        "AGS Public Runtime Setup".to_string(),
        "========================".to_string(),
        format!("Target: {}", target.display()),
        format!("Mode: {}", if yes { "apply" } else { "plan-only" }),
        format!("Force: {}", force),
        String::new(),
        "Files:".to_string(),
    ];
    for file in files {
        let status = if file.path.exists() {
            if force {
                "replace"
            } else {
                "exists-skip"
            }
        } else {
            "create"
        };
        lines.push(format!(
            "- [{}] {} — {}",
            status,
            file.path.display(),
            file.description
        ));
    }
    lines.push(String::new());
    lines.push(
        "MCP rule: AGS scenarios must call `ags_preflight` first; CLI preflight is fallback only."
            .to_string(),
    );
    if !yes {
        lines.push(
            "Dry-run only. Re-run with `ags setup --yes` to write AGS-owned command/snippet files."
                .to_string(),
        );
    }
    lines.join("\n")
}

fn write_setup_file(file: &SetupFile, force: bool) -> Result<String, String> {
    if file.path.exists() && !force {
        return Ok("exists-skip".to_string());
    }
    if let Some(parent) = file.path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
    }
    std::fs::write(&file.path, &file.content)
        .map_err(|e| format!("cannot write {}: {}", file.path.display(), e))?;
    if file.executable {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&file.path)
                .map_err(|e| format!("cannot stat {}: {}", file.path.display(), e))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&file.path, perms)
                .map_err(|e| format!("cannot chmod {}: {}", file.path.display(), e))?;
        }
    }
    Ok("written".to_string())
}

fn cmd_setup(yes: bool, force: bool, target: Option<PathBuf>, register_claude: bool, format: &str) {
    let target = target.unwrap_or_else(default_setup_target);
    let files = setup_files(&target);

    if !yes {
        match format {
            "json" => {
                let output = serde_json::json!({
                    "schema_version": "2.4-public-setup",
                    "target": target.display().to_string(),
                    "mode": "plan-only",
                    "force": force,
                    "register_claude": register_claude,
                    "mcp": {
                        "command": "ags mcp serve --transport stdio",
                        "mandatory_first_tool": "ags_preflight"
                    },
                    "files": files.iter().map(|file| serde_json::json!({
                        "path": file.path.display().to_string(),
                        "description": file.description,
                        "status": if file.path.exists() { if force { "replace" } else { "exists-skip" } } else { "create" },
                    })).collect::<Vec<_>>(),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                );
            }
            _ => println!("{}", render_setup_plan_text(&target, false, force, &files)),
        }
        return;
    }

    let mut results = Vec::new();
    for file in &files {
        match write_setup_file(file, force) {
            Ok(status) => results.push((file.path.display().to_string(), status, None)),
            Err(e) => results.push((
                file.path.display().to_string(),
                "error".to_string(),
                Some(e),
            )),
        }
    }

    for retired_dir in retired_codex_ags_skill_dirs() {
        if retired_dir.exists() && force {
            match std::fs::remove_dir_all(&retired_dir) {
                Ok(()) => results.push((
                    retired_dir.display().to_string(),
                    "removed-retired".to_string(),
                    None,
                )),
                Err(e) => results.push((
                    retired_dir.display().to_string(),
                    "error".to_string(),
                    Some(e.to_string()),
                )),
            }
        }
    }

    if register_claude {
        match std::process::Command::new("claude")
            .args([
                "mcp",
                "add",
                "-s",
                "user",
                "ags",
                "--",
                "ags",
                "mcp",
                "serve",
                "--transport",
                "stdio",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                results.push(("claude mcp ags".to_string(), "registered".to_string(), None))
            }
            Ok(output) => results.push((
                "claude mcp ags".to_string(),
                "error".to_string(),
                Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            )),
            Err(e) => results.push((
                "claude mcp ags".to_string(),
                "error".to_string(),
                Some(e.to_string()),
            )),
        }
    }

    let failed = results.iter().any(|(_, status, _)| status == "error");
    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": "2.4-public-setup",
                "target": target.display().to_string(),
                "mode": "apply",
                "force": force,
                "register_claude": register_claude,
                "results": results.iter().map(|(path, status, error)| serde_json::json!({
                    "path": path,
                    "status": status,
                    "error": error,
                })).collect::<Vec<_>>(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", render_setup_plan_text(&target, true, force, &files));
            println!();
            println!("Results:");
            for (path, status, error) in &results {
                println!("- [{status}] {path}");
                if let Some(error) = error {
                    println!("  error: {error}");
                }
            }
        }
    }

    if failed {
        std::process::exit(1);
    }
}

fn cmd_init(target: &Path, dry_run: bool, format: &str) {
    cmd_project_integrate(target, dry_run, !dry_run, format);
}

fn cmd_mcp_serve(transport: &str) {
    match transport {
        "stdio" => {
            eprintln!(
                "[ags-mcp] starting AGS MCP host initialization adapter v{} on stdio",
                AGS_VERSION
            );
            eprintln!(
                "[ags-mcp] AGS MCP is the mandatory governance interface; call ags_preflight first."
            );
            ags_mcp::run_mcp_server();
        }
        other => {
            eprintln!(
                "ags mcp serve: unsupported transport '{}' — only 'stdio' is supported in v1",
                other
            );
            std::process::exit(2);
        }
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
                "public" | "public-core-only" | "public-full" | "public-full-sanitized" => {
                    workflow_sync_check::ProjectKind::PublicCoreOnly
                }
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

const AGS_ENTRY_BEGIN: &str = "<!-- AGS:BEGIN managed-entry v2 -->";
const AGS_ENTRY_END: &str = "<!-- AGS:END managed-entry v2 -->";

#[derive(Debug, Clone, Eq, PartialEq)]
struct EntryFileReport {
    path: String,
    action: String,
    changed: bool,
    backup_path: Option<String>,
    conflicts: Vec<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
struct MemoryEntryReport {
    path: String,
    action: String,
    changed: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
struct ProjectMemoryReport {
    memory_dir: String,
    changed: bool,
    entries: Vec<MemoryEntryReport>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum ManagedBlockOutcome {
    Created,
    Appended,
    Updated,
    Unchanged,
    Conflict(Vec<String>),
}

fn render_agents_managed_block() -> String {
    format!(
        "{AGS_ENTRY_BEGIN}\n\
## Agent Governance Suite\n\
\n\
This project is governed by AGS. Preserve project-specific instructions above\n\
and below this managed block; only AGS should update the block itself.\n\
\n\
Before development, debugging, review, commit, or task-card generation:\n\
- Run or equivalently complete `ags session preflight --for <codex|claude-code|cursor>`.\n\
- Read `CLAUDE.md` and the canonical protocol files under `protocol/`.\n\
- Treat AGS as a standing engineering hub, not a CLI toolbox the user must invoke manually.\n\
- Complete ambient preflight and solution formation before any Light / Medium / Heavy routing.\n\
- Do not treat raw user requests as executable task cards.\n\
- `方案 OK` only ends the solution phase; wait for an explicit task-card instruction before routing or calling `ags task compile --task-card-requested`.\n\
- Preserve user-owned entry-file rules; if they conflict with this block, stop and report the conflict.\n\
\n\
Canonical references:\n\
- `AGENT_SUITE_PROTOCOL.md`\n\
- `protocol/agent-task-protocol.md`\n\
- `protocol/task-routing.md`\n\
- `protocol/runtime-adapters.md`\n\
- `protocol/task-card-template.md`\n\
{AGS_ENTRY_END}\n"
    )
}

fn render_claude_managed_block() -> String {
    format!(
        "{AGS_ENTRY_BEGIN}\n\
## Agent Governance Suite Execution Rules\n\
\n\
This project is AGS-governed. Keep user-authored Claude instructions outside\n\
this managed block. AGS may update only the marked block.\n\
\n\
Claude Code role:\n\
- Consume bounded task cards formed from confirmed execution contracts.\n\
- Read `AGENTS.md`, this file, and the canonical protocol files before execution.\n\
- Do not form Light / Medium / Heavy classifications from raw user requests.\n\
- Do not generate executable task cards from raw requests or from `方案 OK` alone.\n\
- For Heavy tasks, start plan-only and wait for explicit human approval before mutation.\n\
- On resume or `继续`, reread the task card, run `git status --short`, and stop if mutation approval is unclear.\n\
- Do not install hooks, dependencies, runner adapters, or production wiring unless the task card explicitly authorizes it.\n\
- Do not run destructive git commands or touch secrets unless explicitly authorized.\n\
- Complete the narrowest relevant verification and report evidence before claiming completion.\n\
\n\
Canonical references:\n\
- `protocol/agent-task-protocol.md`\n\
- `protocol/task-routing.md`\n\
- `protocol/runtime-adapters.md`\n\
- `protocol/task-card-template.md`\n\
{AGS_ENTRY_END}\n"
    )
}

fn upsert_managed_block(existing: Option<&str>, block: &str) -> (String, ManagedBlockOutcome) {
    let Some(existing) = existing else {
        return (
            format!("# AGS Project Entry\n\n{block}"),
            ManagedBlockOutcome::Created,
        );
    };

    let begin = existing.find(AGS_ENTRY_BEGIN);
    let end = existing.find(AGS_ENTRY_END);
    match (begin, end) {
        (Some(begin), Some(end)) if begin <= end => {
            let end_with_marker = end + AGS_ENTRY_END.len();
            let mut next = String::new();
            next.push_str(&existing[..begin]);
            next.push_str(block.trim_end());
            next.push_str(&existing[end_with_marker..]);
            if !next.ends_with('\n') {
                next.push('\n');
            }
            if next == existing {
                (existing.to_string(), ManagedBlockOutcome::Unchanged)
            } else {
                (next, ManagedBlockOutcome::Updated)
            }
        }
        (None, None) => {
            let mut next = existing.trim_end().to_string();
            next.push_str("\n\n");
            next.push_str(block);
            (next, ManagedBlockOutcome::Appended)
        }
        _ => (
            existing.to_string(),
            ManagedBlockOutcome::Conflict(vec![
                "partial AGS managed-entry marker found; manual repair required".to_string(),
            ]),
        ),
    }
}

fn detect_entry_conflicts(content: &str) -> Vec<String> {
    let mut conflicts = Vec::new();
    let lowered = content.to_lowercase();
    let patterns = [
        (
            "无需确认直接执行",
            "entry file allows execution without confirmation",
        ),
        (
            "不需要确认直接执行",
            "entry file allows execution without confirmation",
        ),
        (
            "directly execute without confirmation",
            "entry file allows execution without confirmation",
        ),
        ("skip preflight", "entry file says to skip preflight"),
        ("不要运行 preflight", "entry file says to skip preflight"),
        (
            "auto install hooks",
            "entry file allows automatic hook installation",
        ),
        (
            "自动安装 hook",
            "entry file allows automatic hook installation",
        ),
    ];

    for (needle, message) in patterns {
        if lowered.contains(&needle.to_lowercase()) {
            conflicts.push(message.to_string());
        }
    }
    conflicts.sort();
    conflicts.dedup();
    conflicts
}

fn epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn write_backup(target: &Path, rel_path: &str, content: &str) -> Result<String, String> {
    let backup_dir = target.join(".ags").join("backups");
    std::fs::create_dir_all(&backup_dir).map_err(|e| {
        format!(
            "cannot create backup directory {}: {}",
            backup_dir.display(),
            e
        )
    })?;
    let backup_path = backup_dir.join(format!("{}.{}.bak", rel_path, epoch_seconds()));
    std::fs::write(&backup_path, content)
        .map_err(|e| format!("cannot write backup {}: {}", backup_path.display(), e))?;
    Ok(backup_path.display().to_string())
}

fn integrate_entry_file(
    target: &Path,
    rel_path: &str,
    block: &str,
    confirm: bool,
) -> Result<EntryFileReport, String> {
    let full = target.join(rel_path);
    let existing = if full.exists() {
        Some(
            std::fs::read_to_string(&full)
                .map_err(|e| format!("cannot read {}: {}", full.display(), e))?,
        )
    } else {
        None
    };

    let mut conflicts = existing
        .as_deref()
        .map(detect_entry_conflicts)
        .unwrap_or_default();
    let (next, outcome) = upsert_managed_block(existing.as_deref(), block);
    if let ManagedBlockOutcome::Conflict(mut marker_conflicts) = outcome.clone() {
        conflicts.append(&mut marker_conflicts);
        conflicts.sort();
        conflicts.dedup();
        return Ok(EntryFileReport {
            path: rel_path.to_string(),
            action: "conflict".to_string(),
            changed: false,
            backup_path: None,
            conflicts,
        });
    }

    if !conflicts.is_empty() {
        return Ok(EntryFileReport {
            path: rel_path.to_string(),
            action: "conflict".to_string(),
            changed: false,
            backup_path: None,
            conflicts,
        });
    }

    let action = match outcome {
        ManagedBlockOutcome::Created => "create",
        ManagedBlockOutcome::Appended => "append",
        ManagedBlockOutcome::Updated => "update",
        ManagedBlockOutcome::Unchanged => "unchanged",
        ManagedBlockOutcome::Conflict(_) => unreachable!(),
    };
    let changed = action != "unchanged";
    let mut backup_path = None;

    if confirm && changed {
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create directory {}: {}", parent.display(), e))?;
        }
        if let Some(ref existing) = existing {
            backup_path = Some(write_backup(target, rel_path, existing)?);
        }
        std::fs::write(&full, next)
            .map_err(|e| format!("cannot write {}: {}", full.display(), e))?;
    }

    Ok(EntryFileReport {
        path: rel_path.to_string(),
        action: action.to_string(),
        changed,
        backup_path,
        conflicts,
    })
}

fn render_project_integrate_text(
    target: &Path,
    confirm: bool,
    files: &[EntryFileReport],
    memory: Option<&ProjectMemoryReport>,
) -> String {
    let mut lines = Vec::new();
    lines.push("AGS Project Entry Integration".to_string());
    lines.push("=============================".to_string());
    lines.push(format!("Target: {}", target.display()));
    lines.push(format!(
        "Mode: {}",
        if confirm { "confirm" } else { "dry-run" }
    ));
    lines.push(String::new());
    for file in files {
        lines.push(format!(
            "- {}: {}{}",
            file.path,
            file.action,
            if file.changed { " (changed)" } else { "" }
        ));
        if let Some(backup) = &file.backup_path {
            lines.push(format!("  backup: {backup}"));
        }
        for conflict in &file.conflicts {
            lines.push(format!("  conflict: {conflict}"));
        }
    }
    if let Some(memory) = memory {
        lines.push(String::new());
        lines.push(format!("Project memory: {}", memory.memory_dir));
        for entry in &memory.entries {
            lines.push(format!(
                "- {}: {}{}",
                entry.path,
                entry.action,
                if entry.changed { " (changed)" } else { "" }
            ));
        }
    }
    if !confirm {
        lines.push(String::new());
        lines.push(
            "Dry-run only. Re-run with --confirm to write managed blocks and initialize memory."
                .to_string(),
        );
    }
    lines.join("\n")
}

fn project_memory_base() -> PathBuf {
    if let Ok(dir) = std::env::var("AGS_MEMORY_DIR") {
        PathBuf::from(dir)
    } else {
        let home = ags_platform::home_dir_or_temp();
        PathBuf::from(home).join(".agents/memory/projects")
    }
}

fn project_slug(target: &Path) -> String {
    target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string()
}

fn memory_entry(path: &Path, exists: bool, write: bool, create_dir: bool) -> MemoryEntryReport {
    let action = match (exists, write) {
        (true, _) => "exists",
        (false, true) => "created",
        (false, false) => {
            if create_dir {
                "would-create-dir"
            } else {
                "would-create"
            }
        }
    };
    MemoryEntryReport {
        path: path.display().to_string(),
        action: action.to_string(),
        changed: !exists,
    }
}

fn ensure_project_memory(target: &Path, write: bool) -> Result<ProjectMemoryReport, String> {
    let slug = project_slug(target);
    let memory_dir = project_memory_base().join(&slug);
    let archive_dir = memory_dir.join("task-archive");
    let sessions_dir = memory_dir.join("sessions");
    let capsule = memory_dir.join("context-capsule.md");
    let task_memory = memory_dir.join("task-memory.md");
    let archive_index = memory_dir.join("archive-index.md");

    let mut entries = Vec::new();

    for dir in [&memory_dir, &archive_dir, &sessions_dir] {
        let exists = dir.exists();
        entries.push(memory_entry(dir, exists, write, true));
        if write && !exists {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create memory directory {}: {}", dir.display(), e))?;
        }
    }

    let files = [
        (
            capsule.as_path(),
            render_context_capsule_template(&slug, target),
        ),
        (task_memory.as_path(), render_task_memory_template(&slug)),
        (
            archive_index.as_path(),
            render_archive_index_template(&slug),
        ),
    ];

    for (path, content) in files {
        let exists = path.exists();
        entries.push(memory_entry(path, exists, write, false));
        if write && !exists {
            std::fs::write(path, content)
                .map_err(|e| format!("cannot write memory file {}: {}", path.display(), e))?;
        }
    }

    Ok(ProjectMemoryReport {
        memory_dir: memory_dir.display().to_string(),
        changed: entries.iter().any(|e| e.changed),
        entries,
    })
}

fn cmd_project_integrate(target: &Path, dry_run: bool, confirm: bool, format: &str) {
    if !target.exists() {
        eprintln!(
            "project integrate: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }
    if dry_run && confirm {
        eprintln!("project integrate: --dry-run and --confirm cannot be used together");
        std::process::exit(2);
    }

    let write = confirm;
    let specs = [
        ("AGENTS.md", render_agents_managed_block()),
        ("CLAUDE.md", render_claude_managed_block()),
    ];
    let mut files = Vec::new();
    for (rel_path, block) in specs {
        match integrate_entry_file(target, rel_path, &block, write) {
            Ok(report) => files.push(report),
            Err(e) => {
                eprintln!("project integrate: {e}");
                std::process::exit(1);
            }
        }
    }

    let has_conflict = files.iter().any(|f| !f.conflicts.is_empty());
    let memory = if has_conflict && write {
        None
    } else {
        match ensure_project_memory(target, write) {
            Ok(report) => Some(report),
            Err(e) => {
                eprintln!("project integrate: {e}");
                std::process::exit(1);
            }
        }
    };
    match format {
        "json" => {
            let output = serde_json::json!({
                "target": target.display().to_string(),
                "mode": if write { "confirm" } else { "dry-run" },
                "changed": files.iter().any(|f| f.changed),
                "conflicts": has_conflict,
                "memory": memory,
                "files": files.iter().map(|f| {
                    serde_json::json!({
                        "path": f.path,
                        "action": f.action,
                        "changed": f.changed,
                        "backup_path": f.backup_path,
                        "conflicts": f.conflicts,
                    })
                }).collect::<Vec<_>>(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => println!(
            "{}",
            render_project_integrate_text(target, write, &files, memory.as_ref())
        ),
    }

    if has_conflict {
        std::process::exit(1);
    }
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

// ── New dispatch functions (M3-M6) ────────────────────────────────────────

/// Dispatch: `task compile` (M4)
fn cmd_task_compile(
    path: &str,
    format: &str,
    output: &str,
    check_only: bool,
    task_card_requested: bool,
) {
    use std::io::Read;

    if check_only && output == "card" {
        eprintln!("task compile: --check-only cannot be combined with --output card");
        std::process::exit(2);
    }
    if !task_card_requested && output == "card" {
        eprintln!("task compile: --task-card-requested is required for --output card");
        eprintln!("  The user must explicitly issue a task-card instruction before an executable card can be generated.");
        std::process::exit(1);
    }

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };
    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: read failed: {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: read failed: {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (compiled_card, report) =
        task_compiler::compile(&content, &project_root, check_only, task_card_requested);

    let (vp, ve) = if !report.missing_slots.is_empty() {
        (
            false,
            vec![format!(
                "Missing required slots: {}",
                report.missing_slots.join(", ")
            )],
        )
    } else {
        let errors = task_card_validator::validate(&compiled_card);
        (errors.is_empty(), errors)
    };

    let final_report = task_compiler::CompileReport {
        schema_version: report.schema_version,
        compiled_task_card: report.compiled_task_card,
        slot_sources: report.slot_sources,
        missing_slots: report.missing_slots,
        assumptions: report.assumptions,
        validation_passed: if report.executable_allowed {
            vp
        } else {
            report.validation_passed
        },
        validation_errors: if report.executable_allowed {
            ve
        } else {
            report.validation_errors
        },
        check_only,
        task_card_requested: report.task_card_requested,
        executable_allowed: report.executable_allowed,
        block_reason: report.block_reason,
    };

    match format {
        "json" => println!("{}", task_compiler::render_report_json(&final_report)),
        _ => {
            if output == "card" && final_report.executable_allowed {
                println!("{}", task_compiler::render_card_text(&final_report));
            } else {
                println!("{}", task_compiler::render_report_text(&final_report));
            }
        }
    }

    let success = if final_report.check_only {
        final_report.missing_slots.is_empty()
    } else {
        final_report.executable_allowed && final_report.validation_passed
    };
    if !success {
        std::process::exit(1);
    }
}

/// Dispatch: `task new`
fn cmd_task_new(card_type: &str, output: Option<&PathBuf>) {
    let template = if card_type == "full" {
        "## 任务卡\n读取并遵守：\n- AGENTS.md\n- CLAUDE.md\n- protocol/agent-task-protocol.md\nExecutor: Claude Code\nRuntime adapter: claude-code\nExecution surface: cli\nPermission mode: edit-with-confirmation\nParallelism: none\n任务级别：Medium\nReview gate:\n- Medium Codex review\n任务：\n背景：\n项目画像：\n记忆胶囊：\n任务存档：\n相关路径：\n- .\n本次任务相关文件：\n目标：\n非目标：\n验证：\nVerification gate:\n- commands:\n  - echo done\n- expected evidence:\n  - test passes\n- stop condition:\n  - any failure\n交付：\n按协议输出交付报告\n"
    } else {
        "## 任务卡\n路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\nExecution surface: cli\nPermission mode: edit-with-confirmation\nParallelism: none\n任务级别：Medium\n读取：\n- .\n任务：\n目标：\n非目标：\n关键路径：\n- .\n验证：\n停止条件：\n交付：\n"
    };

    match output {
        Some(p) => {
            if let Err(e) = std::fs::write(p, template) {
                eprintln!("task new: write failed: {}", e);
                std::process::exit(1);
            }
            eprintln!(
                "task new: wrote {} task card template to {}",
                card_type,
                p.display()
            );
        }
        None => print!("{}", template),
    }
}

/// Dispatch: `gate check`
fn cmd_gate_check(path: &str, format: &str, approve_writes: bool) {
    let (_, card, _display_path) = read_and_validate_task_card(path);
    let input = build_policy_input(&card.fields, approve_writes);
    let output = execution_policy::gate_check(&input);

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&output)
                .unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e));
            println!("{}", json);
        }
        _ => {
            println!("Gate Decision: {}", output.decision);
            println!("{}", format_policy_text(&output.resolved_policy));
        }
    }
    if output.decision == execution_policy::GateDecision::Stop {
        std::process::exit(1);
    }
}

/// Dispatch: `run`
fn cmd_run(path: &str, check_only: bool, dry_run: bool, approve_writes: bool, format: &str) {
    let _mode = if check_only {
        "check-only"
    } else if dry_run {
        "dry-run"
    } else {
        "dry-run"
    };
    let plan = runner::run_task_card(path, check_only, dry_run || !check_only, approve_writes);

    match format {
        "json" => println!("{}", runner::render_json(&plan)),
        _ => println!("{}", runner::render_text(&plan)),
    }

    if !plan.validation_passed || plan.gate_decision == "stop" {
        std::process::exit(1);
    }
}

/// Dispatch: `receipt generate`
fn cmd_receipt_generate(
    task_card: &str,
    gate_result: &str,
    gate_reason: Option<&str>,
    verifications: &[String],
    delivery_report: Option<&str>,
    review_gate_status: Option<&str>,
    metadata_pairs: &[String],
    format: &str,
) {
    let task_path = if task_card == "-" {
        eprintln!("receipt generate: stdin not supported for --task-card; use a file path");
        std::process::exit(2);
    } else {
        Path::new(task_card)
    };

    let vrs: Vec<receipt::VerificationResult> = verifications
        .iter()
        .filter_map(|v| {
            let parts: Vec<&str> = v.splitn(2, ':').collect();
            if parts.len() == 2 {
                let exit_code: i32 = parts[1].parse().unwrap_or(-1);
                Some(receipt::VerificationResult {
                    command: parts[0].to_string(),
                    exit_code,
                    output_hash: receipt::sha256_hex(parts[1].as_bytes()),
                })
            } else {
                None
            }
        })
        .collect();

    // Parse metadata key=value pairs
    let metadata: Option<std::collections::HashMap<String, String>> = if metadata_pairs.is_empty() {
        None
    } else {
        let mut map = std::collections::HashMap::new();
        for pair in metadata_pairs {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            }
        }
        Some(map)
    };

    let delivery_path = delivery_report.map(|s| Path::new(s));
    match receipt::generate_receipt(
        task_path,
        gate_result,
        gate_reason,
        vrs,
        delivery_path,
        review_gate_status,
        metadata,
    ) {
        Ok(r) => match format {
            "json" => println!("{}", receipt::render_receipt_json(&r)),
            _ => println!(
                "Receipt generated: {}\n  Task card hash: {}\n  Gate: {}",
                r.receipt_id, r.task_card_hash, r.gate_result.decision
            ),
        },
        Err(e) => {
            eprintln!("receipt generate: {}", e);
            std::process::exit(1);
        }
    }
}

/// Dispatch: `receipt verify`
fn cmd_receipt_verify(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("receipt verify: {}", e);
            std::process::exit(1);
        }
    };
    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("receipt verify: invalid JSON: {}", e);
            std::process::exit(1);
        }
    };
    let result = receipt::verify_receipt(&receipt);
    match format {
        "json" => println!("{}", receipt::render_verify_json(&result)),
        _ => println!("{}", receipt::render_verify_text(&result)),
    }
    if !result.valid {
        std::process::exit(1);
    }
}

/// Dispatch: `compliance check`
fn cmd_compliance_check(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("compliance check: {}", e);
            std::process::exit(1);
        }
    };
    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("compliance check: invalid JSON: {}", e);
            std::process::exit(1);
        }
    };
    let result = receipt::check_compliance(&receipt);
    match format {
        "json" => println!("{}", receipt::render_compliance_json(&result)),
        _ => println!("{}", receipt::render_compliance_text(&result)),
    }
    if !result.compliant {
        std::process::exit(1);
    }
}

/// Dispatch: `skill scan`
fn cmd_skill_scan(format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let result = skill_governance::scan_skills(&root);
    match format {
        "json" => println!("{}", skill_governance::render_scan_json(&result)),
        _ => println!("{}", skill_governance::render_scan_text(&result)),
    }
}

/// Dispatch: `skill check`
fn cmd_skill_check(format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let result = skill_governance::check_skills(&root);
    match format {
        "json" => println!("{}", skill_governance::render_check_json(&result)),
        _ => println!("{}", skill_governance::render_check_text(&result)),
    }
    if !result.passed {
        std::process::exit(1);
    }
}

/// Dispatch: `skill propose`
fn cmd_skill_propose(action: &str, skill: &str, format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let result = skill_governance::propose_skills(&root, action, skill);
    match format {
        "json" => println!("{}", skill_governance::render_proposal_json(&result)),
        _ => println!("{}", skill_governance::render_proposal_text(&result)),
    }
}

/// Dispatch: `skill install`
///
/// Delegates to `skill_governance::install_skills` for real skill installation
/// with directory structure and SKILL.md frontmatter.
fn cmd_skill_install(
    skill: &str,
    confirm: bool,
    dry_run: bool,
    target: Option<&PathBuf>,
    mode: &str,
    source_dir: Option<&PathBuf>,
    format: &str,
) {
    let skills_dir = target.cloned().unwrap_or_else(|| {
        let home = ags_platform::home_dir_or_temp();
        PathBuf::from(home).join(".agents/skills")
    });

    let install_mode = match mode {
        "full" => skill_governance::InstallMode::Full,
        _ => skill_governance::InstallMode::Template,
    };

    // Show plan before installing (text format)
    if format != "json" && !confirm {
        let (defs, warnings, target_str) = skill_governance::install_plan(skill, &skills_dir);
        let mode_banner = match install_mode {
            skill_governance::InstallMode::Template => {
                "TEMPLATE INSTALL — generates SKILL.md skeleton with frontmatter"
            }
            skill_governance::InstallMode::Full => "FULL INSTALL — copies complete skill package",
        };
        println!("Skill Install Plan [{}]", mode_banner);
        println!("==================");
        println!("Target directory: {}", target_str);
        println!("Skills to install:");
        for def in &defs {
            let cat = match def.category {
                skill_governance::SkillCategory::Auto => "auto-trigger",
                skill_governance::SkillCategory::Manual => "manual",
            };
            println!("  - {}  (source: {}, type: {})", def.name, def.source, cat);
        }
        if defs.is_empty() {
            for w in &warnings {
                println!("  ! {}", w);
            }
        }
        println!();
        println!("Risk summary:");
        println!("  - Skills will be installed to: {}", target_str);
        if install_mode == skill_governance::InstallMode::Template {
            println!("  - MODE: TEMPLATE — only a skeleton is created");
            println!("  - You MUST copy real content from the source repository");
            println!("  - Template files are clearly marked");
        }
        println!("  - Existing installs may be overwritten");
        println!();
    }

    // Delegate to the library
    let source = source_dir.map(|p| p.as_path());
    let result = skill_governance::install_skills(
        skill,
        &skills_dir,
        confirm,
        dry_run,
        install_mode,
        source,
    );

    match format {
        "json" => println!("{}", skill_governance::render_install_json(&result)),
        _ => println!("{}", skill_governance::render_install_text(&result)),
    }

    match result.status {
        skill_governance::InstallStatus::Blocked => {
            if !confirm {
                std::process::exit(1);
            }
        }
        skill_governance::InstallStatus::PartialFailure => {
            std::process::exit(1);
        }
        _ => {}
    }
}

/// Dispatch: `skill adopt`
fn cmd_skill_adopt(skill: &str, apply: bool, format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let governance_dir = root.join("governance");
    if !apply {
        let result = skill_governance::propose_skills(&root, "adopt", skill);
        match format {
            "json" => println!("{}", skill_governance::render_proposal_json(&result)),
            _ => {
                println!("{}", skill_governance::render_proposal_text(&result));
                println!("DRY-RUN ONLY. Use --apply to adopt the skill.");
            }
        }
        return;
    }
    // Apply: append to adoption log
    if let Err(e) = std::fs::create_dir_all(&governance_dir) {
        eprintln!("skill adopt: {}", e);
        std::process::exit(1);
    }
    let log_path = governance_dir.join("skill-adoption-log.yaml");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = format!("\n- id: adopt-{timestamp}\n  skill_name: {skill}\n  decision: adopted\n  timestamp: \"{timestamp}\"\n");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|e| {
            eprintln!("skill adopt: cannot open adoption log: {}", e);
            std::process::exit(1);
        });
    use std::io::Write;
    if file.write_all(entry.as_bytes()).is_err() {
        eprintln!("skill adopt: write failed");
        std::process::exit(1);
    }
    if format != "json" {
        println!(
            "Skill '{}' adopted. Log updated: {}",
            skill,
            log_path.display()
        );
    }
}

/// Dispatch: `skill ignore`
fn cmd_skill_ignore(skill: &str, reason: Option<&str>, apply: bool, format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let governance_dir = root.join("governance");
    if !apply {
        let result = skill_governance::propose_skills(&root, "disable", skill);
        match format {
            "json" => println!("{}", skill_governance::render_proposal_json(&result)),
            _ => {
                println!("{}", skill_governance::render_proposal_text(&result));
                println!("DRY-RUN ONLY. Use --apply to ignore the skill.");
            }
        }
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&governance_dir) {
        eprintln!("skill ignore: {}", e);
        std::process::exit(1);
    }
    let log_path = governance_dir.join("skill-ignore-list.yaml");
    let reason_str = reason.unwrap_or("manually ignored");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = format!("\n- id: ignore-{timestamp}\n  skill_name: {skill}\n  reason: \"{reason_str}\"\n  status: active\n  timestamp: \"{timestamp}\"\n");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|e| {
            eprintln!("skill ignore: cannot open ignore list: {}", e);
            std::process::exit(1);
        });
    use std::io::Write;
    if file.write_all(entry.as_bytes()).is_err() {
        eprintln!("skill ignore: write failed");
        std::process::exit(1);
    }
    if format != "json" {
        println!(
            "Skill '{}' ignored. Log updated: {}",
            skill,
            log_path.display()
        );
    }
}

/// Dispatch: `capability list`
fn cmd_capability_list(target: &Path, format: &str) {
    let registry = capability_registry::discover_all(target);
    match format {
        "json" => println!("{}", capability_registry::render_json(&registry)),
        _ => println!("{}", capability_registry::render_text(&registry)),
    }
}

/// Dispatch: `capability show`
fn cmd_capability_show(name: &str, target: &Path, format: &str) {
    let registry = capability_registry::discover_all(target);
    match capability_registry::find_by_id(&registry, name) {
        Some(cap) => match format {
            "json" => println!("{}", capability_registry::render_one_json(cap)),
            _ => println!("{}", capability_registry::render_one_text(cap)),
        },
        None => {
            eprintln!("capability show: not found: {}", name);
            std::process::exit(1);
        }
    }
}

fn render_context_capsule_template(slug: &str, project_path: &Path) -> String {
    format!(
        "# Context Capsule: {slug}\n\
\n\
Manual-maintained stable project memory.\n\
\n\
## 项目设计目的\n\
\n\
(TODO: describe this project's purpose in one or two concrete paragraphs.)\n\
\n\
Rules:\n\
- Runner, hook, capture, or automated summarization must not overwrite this section.\n\
- Automated summaries must not rewrite this section.\n\
- Modify this section only when the project owner explicitly asks.\n\
- Agents must read this file before task execution.\n\
- If a task conflicts with this section, stop and report before changing files.\n\
\n\
## Stable Facts\n\
\n\
- Project path: `{}`\n\
- Memory dir: `$HOME/.agents/memory/projects/{slug}`\n\
\n\
## 项目长期边界\n\
\n\
- (TODO: define non-negotiable project boundaries.)\n\
\n\
## 核心业务定位\n\
\n\
- (TODO: define what this project is for and what it is not for.)\n\
\n\
## 原则性决策\n\
\n\
- (TODO: record durable decisions that should survive context compaction.)\n\
\n\
## 自动记忆入口\n\
\n\
- Progress log: `$HOME/.agents/memory/projects/{slug}/progress-log.md`\n\
- Archive index: `$HOME/.agents/memory/projects/{slug}/archive-index.md`\n\
- Sessions: `$HOME/.agents/memory/projects/{slug}/sessions`\n\
- Task archive: `$HOME/.agents/memory/projects/{slug}/task-archive`\n",
        project_path.display()
    )
}

fn render_task_memory_template(slug: &str) -> String {
    format!(
        "# Task Memory: {slug}\n\
\n\
Updated: initialized\n\
\n\
This file is automatically refreshed from local task archives. The manual\n\
project charter remains in `context-capsule.md`.\n\
\n\
## Current Status\n\
\n\
- Latest task: none\n\
- Status: initialized\n\
- Conclusion: no completed tasks archived yet\n\
- Archive: none\n\
\n\
## Latest Delivery Report\n\
\n\
- Source: none\n\
\n\
## Recent Task Archive Index\n\
\n\
- Task archive root: `$HOME/.agents/memory/projects/{slug}/task-archive`\n"
    )
}

fn render_archive_index_template(slug: &str) -> String {
    format!(
        "# Archive Index: {slug}\n\
\n\
This index is append-only operational memory for task archives.\n\
\n\
## Rules\n\
\n\
- Keep this file free of secrets, tokens, and private machine-specific paths.\n\
- Store full task evidence under `task-archive/`.\n\
- Keep `context-capsule.md` as the manual project charter; do not auto-rewrite it.\n\
\n\
## Entries\n\
\n\
(none yet)\n"
    )
}

/// Dispatch: `archive`
fn cmd_archive(
    delivery_report: Option<&PathBuf>,
    task_card: Option<&PathBuf>,
    verification_results: Option<&PathBuf>,
    receipt: Option<&PathBuf>,
    summary: Option<&str>,
    format: &str,
) {
    let memory_base = if let Ok(dir) = std::env::var("AGS_MEMORY_DIR") {
        PathBuf::from(dir)
    } else {
        let home = ags_platform::home_dir_or_temp();
        let slug = std::env::current_dir()
            .ok()
            .and_then(|p| {
                p.file_name()
                    .and_then(|n| n.to_str().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "project".to_string());
        PathBuf::from(home)
            .join(".agents/memory/projects")
            .join(slug)
    };

    let archive_dir = memory_base.join("task-archive");
    if let Err(e) = std::fs::create_dir_all(&archive_dir) {
        eprintln!("archive: cannot create archive directory: {}", e);
        std::process::exit(1);
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let archive_file = archive_dir.join(format!("{}-archive.md", timestamp));

    let mut content = format!("# Task Archive\n\nTimestamp: {}\n", timestamp);

    // Include task card hash if provided
    if let Some(tc) = task_card {
        if let Ok(tc_content) = std::fs::read_to_string(tc) {
            let tc_hash = receipt::sha256_hex(tc_content.as_bytes());
            content.push_str(&format!("Task card hash: {}\n", tc_hash));
        }
    }

    if let Some(s) = summary {
        content.push_str(&format!("Summary: {}\n", s));
    }
    content.push('\n');

    if let Some(dr) = delivery_report {
        if let Ok(body) = std::fs::read_to_string(dr) {
            content.push_str("## Delivery Report\n\n");
            content.push_str(&body);
            content.push('\n');
        }
    }
    if let Some(vr) = verification_results {
        if let Ok(body) = std::fs::read_to_string(vr) {
            content.push_str("## Verification Results\n\n");
            content.push_str(&body);
            content.push('\n');
        }
    }
    if let Some(rc) = receipt {
        if let Ok(body) = std::fs::read_to_string(rc) {
            content.push_str("## Receipt\n\n");
            content.push_str(&body);
            content.push('\n');
        }
    }

    if let Err(e) = std::fs::write(&archive_file, &content) {
        eprintln!("archive: write failed: {}", e);
        std::process::exit(1);
    }

    // Update task-memory.md
    let task_memory_file = memory_base.join("task-memory.md");
    let update = format!(
        "\n## Latest Task\n\n- Timestamp: {}\n- Summary: {}\n- Archive: {}\n",
        timestamp,
        summary.unwrap_or("(no summary)"),
        archive_file.display()
    );
    let mut tm = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&task_memory_file)
        .unwrap_or_else(|_| {
            // Fallback: create new
            std::fs::File::create(&task_memory_file).unwrap_or_else(|e| {
                eprintln!("archive: cannot create task-memory.md: {}", e);
                std::process::exit(1);
            })
        });
    use std::io::Write;
    let _ = tm.write_all(update.as_bytes());

    match format {
        "json" => {
            let output = serde_json::json!({
                "status": "archived",
                "archive_file": archive_file.display().to_string(),
                "task_memory": task_memory_file.display().to_string(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("Archive complete:");
            println!("  archive: {}", archive_file.display());
            println!("  task memory: {}", task_memory_file.display());
        }
    }
}

// ── Hook dispatch ───────────────────────────────────────────────────────

const STOP_ARCHIVE_HOOK_SNIPPET: &str = r#"{
  "hooks": {
    "Stop": [
      {
        "type": "command",
        "command": "bash /path/to/ags/scripts/stop-archive-hook.sh"
      }
    ]
  }
}"#;

fn cmd_hook_install(
    hook_name: &str,
    dry_run: bool,
    confirm: bool,
    target: Option<&PathBuf>,
    format: &str,
) {
    let hook_dir = target.cloned().unwrap_or_else(|| PathBuf::from(".claude"));
    let snippet_file = hook_dir.join(format!("{}-hook-snippet.json", hook_name));

    // Show plan
    if format != "json" && !confirm {
        println!("Hook Install Plan");
        println!("=================");
        println!("Hook: {}", hook_name);
        println!("Hook script: scripts/stop-archive-hook.sh");
        println!();
        println!("What this hook does:");
        println!("  On each Claude Code Stop event, archives the delivery report,");
        println!("  verification results, and receipt to the local memory directory.");
        println!();
        println!("Install behavior:");
        println!("  --confirm writes a hook config snippet to:");
        println!("    {}", snippet_file.display());
        println!();
        println!("You must then manually add the snippet to your ~/.claude/settings.json");
        println!("or equivalent Claude Code configuration.");
        println!();
        println!("The hook will NEVER auto-modify your settings file.");
        println!();
    }

    if dry_run || !confirm {
        if format == "json" {
            let output = serde_json::json!({
                "hook": hook_name,
                "status": if dry_run { "dry-run" } else { "blocked" },
                "snippet_file": snippet_file.display().to_string(),
                "note": "Use --confirm to write the hook snippet file."
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        } else {
            println!(
                "STATUS: {} — use --confirm to write the hook snippet file",
                if dry_run { "dry-run" } else { "blocked" }
            );
        }
        if !confirm {
            std::process::exit(1);
        }
        return;
    }

    // Confirm: write snippet
    if let Err(e) = std::fs::create_dir_all(&hook_dir) {
        eprintln!("hook install: cannot create {}: {}", hook_dir.display(), e);
        std::process::exit(1);
    }

    // Customize snippet with actual AGS script path
    let ags_script_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("scripts/stop-archive-hook.sh");
    let snippet = STOP_ARCHIVE_HOOK_SNIPPET.replace(
        "/path/to/ags/scripts/stop-archive-hook.sh",
        &ags_script_path.display().to_string(),
    );

    match std::fs::write(&snippet_file, &snippet) {
        Ok(_) => {
            if format == "json" {
                let output = serde_json::json!({
                    "hook": hook_name,
                    "status": "installed",
                    "snippet_file": snippet_file.display().to_string(),
                    "next_step": "Manually add the snippet to ~/.claude/settings.json"
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                );
            } else {
                println!("Hook snippet written to: {}", snippet_file.display());
                println!();
                println!("Next step: manually add this to your ~/.claude/settings.json");
                println!("or equivalent Claude Code configuration file.");
                println!();
                println!("Snippet content:");
                println!("{}", snippet);
            }
        }
        Err(e) => {
            eprintln!("hook install: cannot write snippet: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_hook_check(hook_name: &str, format: &str) {
    let hook_dir = PathBuf::from(".claude");
    let snippet_file = hook_dir.join(format!("{}-hook-snippet.json", hook_name));

    let snippet_exists = snippet_file.exists();
    let hook_script_exists = PathBuf::from("scripts/stop-archive-hook.sh").exists();

    if format == "json" {
        let output = serde_json::json!({
            "hook": hook_name,
            "snippet_file": snippet_file.display().to_string(),
            "snippet_exists": snippet_exists,
            "hook_script_exists": hook_script_exists,
            "status": if snippet_exists && hook_script_exists { "ready" } else { "missing" }
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_default()
        );
    } else {
        println!("Hook Status: {}", hook_name);
        println!("=========================");
        println!("Hook script: scripts/stop-archive-hook.sh");
        println!(
            "  {}",
            if hook_script_exists {
                "PRESENT"
            } else {
                "MISSING — run `cargo build` first"
            }
        );
        println!("Snippet file: {}", snippet_file.display());
        println!(
            "  {}",
            if snippet_exists {
                "PRESENT"
            } else {
                "MISSING — run `ags hook install --confirm` first"
            }
        );
        println!();
        if snippet_exists && hook_script_exists {
            println!("Status: READY — snippet file generated, awaiting manual application to settings.json");
        } else {
            println!("Status: NOT INSTALLED");
        }
    }

    if !snippet_exists {
        std::process::exit(1);
    }
}

fn cmd_hook_uninstall(hook_name: &str, confirm: bool, target: Option<&PathBuf>, format: &str) {
    let hook_dir = target.cloned().unwrap_or_else(|| PathBuf::from(".claude"));
    let snippet_file = hook_dir.join(format!("{}-hook-snippet.json", hook_name));

    if !confirm {
        if format == "json" {
            let output = serde_json::json!({
                "hook": hook_name,
                "status": "blocked",
                "snippet_file": snippet_file.display().to_string(),
                "note": "Use --confirm to remove the snippet file."
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        } else {
            println!("Hook Uninstall Plan");
            println!("===================");
            println!("Will remove: {}", snippet_file.display());
            println!("STATUS: blocked — use --confirm to proceed");
            println!();
            println!("Note: this only removes the generated snippet file.");
            println!("You must manually remove the hook from ~/.claude/settings.json");
        }
        std::process::exit(1);
    }

    if snippet_file.exists() {
        match std::fs::remove_file(&snippet_file) {
            Ok(_) => {
                if format != "json" {
                    println!("Removed: {}", snippet_file.display());
                    println!("Note: manually remove the hook entry from ~/.claude/settings.json");
                }
            }
            Err(e) => {
                eprintln!(
                    "hook uninstall: cannot remove {}: {}",
                    snippet_file.display(),
                    e
                );
                std::process::exit(1);
            }
        }
    } else {
        if format != "json" {
            println!("Snippet file not found: {}", snippet_file.display());
            println!("Nothing to uninstall.");
        }
    }

    if format == "json" {
        let output = serde_json::json!({
            "hook": hook_name,
            "status": "uninstalled",
            "snippet_file": snippet_file.display().to_string(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_default()
        );
    }
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup {
            yes,
            force,
            target,
            register_claude,
            format,
        } => cmd_setup(yes, force, target, register_claude, &format),
        Commands::Init {
            target,
            dry_run,
            format,
        } => cmd_init(&target, dry_run, &format),

        // ── M1 object commands ──
        Commands::Task { action } => match action {
            TaskAction::Validate { paths } => cmd_task_validate(&paths),
            TaskAction::Compile {
                path,
                format,
                output,
                check_only,
                task_card_requested,
            } => cmd_task_compile(&path, &format, &output, check_only, task_card_requested),
            TaskAction::New { card_type, output } => cmd_task_new(&card_type, output.as_ref()),
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
            ProjectAction::Integrate {
                target,
                dry_run,
                confirm,
                format,
            } => cmd_project_integrate(&target, dry_run, confirm, &format),
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

        // ── M3 Gate ──
        Commands::Gate { action } => match action {
            GateAction::Check {
                path,
                format,
                approve_writes,
            } => cmd_gate_check(&path, &format, approve_writes),
        },

        // ── M5 Capability ──
        Commands::Capability { action } => match action {
            CapabilityAction::List { target, format } => cmd_capability_list(&target, &format),
            CapabilityAction::Show {
                name,
                target,
                format,
            } => cmd_capability_show(&name, &target, &format),
        },

        // ── M6 Receipt / Compliance ──
        Commands::Receipt { action } => match action {
            ReceiptAction::Generate {
                task_card,
                gate_result,
                gate_reason,
                verifications,
                delivery_report,
                review_gate_status,
                metadata,
                format,
            } => cmd_receipt_generate(
                &task_card,
                &gate_result,
                gate_reason.as_deref(),
                &verifications,
                delivery_report.as_deref(),
                review_gate_status.as_deref(),
                &metadata,
                &format,
            ),
            ReceiptAction::Verify { path, format } => cmd_receipt_verify(&path, &format),
        },
        Commands::Compliance { action } => match action {
            ComplianceAction::Check { path, format } => cmd_compliance_check(&path, &format),
        },

        // ── Hook ──
        Commands::Hook { action } => match action {
            HookAction::Install {
                hook_name,
                dry_run,
                confirm,
                target,
                format,
            } => cmd_hook_install(&hook_name, dry_run, confirm, target.as_ref(), &format),
            HookAction::Check { hook_name, format } => cmd_hook_check(&hook_name, &format),
            HookAction::Uninstall {
                hook_name,
                confirm,
                target,
                format,
            } => cmd_hook_uninstall(&hook_name, confirm, target.as_ref(), &format),
        },

        // ── Skill ──
        Commands::Skill { action } => match action {
            SkillAction::Scan { format } => cmd_skill_scan(&format),
            SkillAction::Check { format } => cmd_skill_check(&format),
            SkillAction::Propose {
                action,
                skill,
                format,
            } => cmd_skill_propose(&action, &skill, &format),
            SkillAction::Install {
                skill,
                confirm,
                dry_run,
                target,
                mode,
                source_dir,
                format,
            } => cmd_skill_install(
                &skill,
                confirm,
                dry_run,
                target.as_ref(),
                &mode,
                source_dir.as_ref(),
                &format,
            ),
            SkillAction::Adopt {
                skill,
                apply,
                format,
            } => cmd_skill_adopt(&skill, apply, &format),
            SkillAction::Ignore {
                skill,
                reason,
                apply,
                format,
            } => cmd_skill_ignore(&skill, reason.as_deref(), apply, &format),
        },
        Commands::Mcp { action } => match action {
            McpAction::Serve { transport } => cmd_mcp_serve(&transport),
        },

        // ── Runner ──
        Commands::Run {
            path,
            check_only,
            dry_run,
            approve_writes,
            format,
        } => cmd_run(&path, check_only, dry_run, approve_writes, &format),

        // ── Archive ──
        Commands::Archive {
            delivery_report,
            task_card,
            verification_results,
            receipt,
            summary,
            format,
        } => cmd_archive(
            delivery_report.as_ref(),
            task_card.as_ref(),
            verification_results.as_ref(),
            receipt.as_ref(),
            summary.as_deref(),
            &format,
        ),

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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_target(name: &str) -> PathBuf {
        let target = std::env::temp_dir().join(format!("ags-cli-{name}-{}", epoch_seconds()));
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir_all(&target).unwrap();
        target
    }

    #[test]
    fn managed_block_append_preserves_user_content() {
        let existing = "# User Rules\n\nKeep my local workflow.\n";
        let block = render_agents_managed_block();

        let (next, outcome) = upsert_managed_block(Some(existing), &block);

        assert_eq!(outcome, ManagedBlockOutcome::Appended);
        assert!(next.contains("Keep my local workflow."));
        assert!(next.contains(AGS_ENTRY_BEGIN));
        assert!(next.contains("task-card"));
    }

    #[test]
    fn managed_block_update_preserves_surrounding_user_content() {
        let existing = format!(
            "# User Rules\n\nbefore\n\n{AGS_ENTRY_BEGIN}\nold managed text\n{AGS_ENTRY_END}\n\nafter\n"
        );
        let block = render_claude_managed_block();

        let (next, outcome) = upsert_managed_block(Some(&existing), &block);

        assert_eq!(outcome, ManagedBlockOutcome::Updated);
        assert!(next.contains("before"));
        assert!(next.contains("after"));
        assert!(!next.contains("old managed text"));
        assert!(next.contains("Claude Code role:"));
    }

    #[test]
    fn partial_managed_marker_reports_conflict() {
        let existing = format!("# User Rules\n\n{AGS_ENTRY_BEGIN}\nmissing end\n");
        let block = render_agents_managed_block();

        let (_next, outcome) = upsert_managed_block(Some(&existing), &block);

        assert!(matches!(outcome, ManagedBlockOutcome::Conflict(_)));
    }

    #[test]
    fn integrate_dry_run_does_not_write_files() {
        let target = temp_target("integrate-dry-run");

        let report =
            integrate_entry_file(&target, "AGENTS.md", &render_agents_managed_block(), false)
                .unwrap();

        assert_eq!(report.action, "create");
        assert!(report.changed);
        assert!(!target.join("AGENTS.md").exists());
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn integrate_confirm_updates_existing_block_and_creates_backup() {
        let target = temp_target("integrate-confirm");
        let agents = target.join("AGENTS.md");
        std::fs::write(
            &agents,
            format!(
                "# User Rules\n\nbefore\n\n{AGS_ENTRY_BEGIN}\nold managed text\n{AGS_ENTRY_END}\n\nafter\n"
            ),
        )
        .unwrap();

        let report =
            integrate_entry_file(&target, "AGENTS.md", &render_agents_managed_block(), true)
                .unwrap();
        let updated = std::fs::read_to_string(&agents).unwrap();

        assert_eq!(report.action, "update");
        assert!(report.backup_path.is_some());
        assert!(updated.contains("before"));
        assert!(updated.contains("after"));
        assert!(!updated.contains("old managed text"));
        assert!(updated.contains("standing engineering hub"));
        assert!(target.join(".ags").join("backups").exists());
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn integrate_conflict_does_not_write() {
        let target = temp_target("integrate-conflict");
        let agents = target.join("AGENTS.md");
        std::fs::write(&agents, "# User Rules\n\n无需确认直接执行。\n").unwrap();

        let report =
            integrate_entry_file(&target, "AGENTS.md", &render_agents_managed_block(), true)
                .unwrap();
        let current = std::fs::read_to_string(&agents).unwrap();

        assert_eq!(report.action, "conflict");
        assert!(!report.conflicts.is_empty());
        assert!(!current.contains(AGS_ENTRY_BEGIN));
        let _ = std::fs::remove_dir_all(target);
    }
}
