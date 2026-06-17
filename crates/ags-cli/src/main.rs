//! Agent Governance Suite unified CLI — binary entry point.
//!
//! AGS exposes a small human-facing facade:
//!
//! - `ags setup`   Global runtime setup so AGS is visible to host agents.
//! - `ags init`    Project onboarding into AGS governance.
//! - `ags doctor`  Health checks and safe repair suggestions.
//! - `ags skill`   Third-party skill & MCP management console: unified
//!   inventory, host visibility, and confirmation-protected propose/apply.
//! - `ags help`    Operator guidance.
//!
//! Kernel operations such as task validation, policy resolution, gates,
//! receipts, compliance, preflight, and release checks remain available to
//! AGS MCP, CI, and compatibility callers, but are hidden from the human CLI
//! command surface.

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── CLI root ──────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "ags",
    about = "Agent Governance Suite CLI",
    after_help = "Common flow:\n  ags setup --yes      Initialize the global AGS runtime\n  ags init             Onboard the current project\n  ags doctor           Diagnose AGS health\n  ags skill            Review local skills",
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
    /// Compile a task intent into a canonical task card (M4).
    ///
    /// Reads a flexible intent file (or stdin with "-") and deterministically
    /// compiles it into the canonical task-card skeleton (the classic fixed
    /// skeleton in protocol/task-card-template.md; the compact format has been
    /// removed).  This is a rule engine only — no AI calls, no free-form
    /// prompt generation.
    ///
    /// Slot filling uses project context (CLAUDE.md, WORKSPACE.md, protocol
    /// files, known workspace identity, and local memory paths).  Slots that
    /// cannot be filled are reported as missing and the command exits 1.
    Compile {
        /// Intent file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Output mode: `card` prints only the compiled task card (pipeable to
        /// `ags task validate -`); `report` prints the full compile report.
        /// Default: `report`.
        #[arg(long, default_value = "report", value_parser = ["card", "report"])]
        output: String,
        /// Check only: report if compilation is possible and what is missing,
        /// but do not output an executable task card.
        #[arg(long, default_value_t = false)]
        check_only: bool,
        /// Task card explicitly requested by the user.
        ///
        /// This is the hard gate between "solution OK" and task card generation.
        /// Without this flag, the compiler produces a diagnostic report only —
        /// it will NOT output an executable task card.  Set this flag only after
        /// the user has explicitly issued a task-card instruction ("生成任务卡",
        /// "按这个方案出任务卡", "交给 Claude Code 执行", etc.).
        ///
        /// Without --task-card-requested, the report will show
        /// executable_allowed=false with block_reason=task_card_not_requested.
        #[arg(long, default_value_t = false)]
        task_card_requested: bool,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Resolve execution policy for a validated task card (read-only).
    ///
    /// Validates the task card first.  If validation fails, prints errors
    /// to stderr and exits with 1.  On success, outputs the resolved
    /// execution policy in the requested format (text or json).
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

/// Runner-facing gate operations (M3).
#[derive(Subcommand)]
enum GateAction {
    /// Run the gate check and output a runner-level decision.
    ///
    /// Outputs decision: allow|confirm|stop with embedded resolved policy.
    /// On validation failure, outputs structured decision=stop JSON with
    /// error details — never just a raw exit code.
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

    /// Entry intent gate: classify a user request for prompt / task-card intent.
    ///
    /// Deterministic (prompt-request-classifier). Decision `require_task_card`
    /// when intent is detected — the host MUST route through preflight →
    /// `task compile --task-card-requested` → `gate output`, and the foreground
    /// answer MUST be a canonical `## 任务卡`. Otherwise `allow`. Runs AGS session
    /// preflight as a fail-closed precondition unless `--no-preflight`; if
    /// preflight reports should_stop, decision = `stop`.
    PromptRequest {
        /// User request text (use "-" for stdin).
        request: String,
        /// Target repository path for the preflight precondition.
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Skip the preflight precondition (pure classification only).
        #[arg(long, default_value_t = false)]
        no_preflight: bool,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Frontstage output-shape gate: verify a candidate foreground answer is a
    /// canonical task card.
    ///
    /// Decision `allow` iff the first non-empty line is `## 任务卡` AND the content
    /// passes the canonical validator; otherwise `stop` with block_reason
    /// `bad_output_shape` or `validation_failed`, plus a `governance_miss` event
    /// (AGS writes no file — the host persists the sample if it wants it).
    Output {
        /// Candidate output file (use "-" for stdin).
        path: String,
        /// Original user request, to correlate the governance_miss (optional).
        #[arg(long)]
        for_request: Option<String>,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
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

#[derive(Subcommand)]
enum SyncAction {
    /// Multi-project protocol drift checker (read-only).
    ///
    /// Compares protocol files between source and one or more targets
    /// at the markdown section/rule level, distinguishing dangerous drift
    /// from legal differences (e.g. public-full sanitized adjustments).
    Check {
        /// Source suite root (default: current directory)
        #[arg(long, default_value = ".")]
        source: PathBuf,

        /// Target name=path pairs, e.g. "stable=/path/to/stable" "public=/path/to/public"
        #[arg(long = "targets", value_name = "NAME=PATH", num_args = 1.., value_parser = parse_target)]
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

// ── M2 Object Command Sub-enums ───────────────────────────────────────────

#[derive(Subcommand)]
enum ProjectAction {
    /// Detect project identity and AGS integration status (read-only).
    ///
    /// Identifies whether the target repo is an AGS development suite,
    /// an AGS-integrated project, or not integrated. Reports workspace
    /// role, protocol file inventory, memory paths, and integration gaps.
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
    ///
    /// Reports which protocol files are present or missing, the task-card
    /// validator entry point, risk boundaries, protected paths, and
    /// review/verify/receipt requirements for the target repository.
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
    ///
    /// Generates instructions tailored to known agents, with a generic
    /// governed-host fallback for any other non-empty agent identifier.
    Instructions {
        /// Agent identifier: codex, claude-code, cursor, tencent-agent, workbuddy, codebuddy-code, cowork, or another host id
        #[arg(long = "for", value_name = "AGENT")]
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
///
/// Aggregates project detection, protocol status, agent instructions, and
/// memory path discovery into a single preflight report. This is the default
/// wake-up entry point for agents — it does NOT depend on skill governance
/// or any third-party configuration.
#[derive(Subcommand)]
enum SessionAction {
    /// Run aggregated session preflight for an agent (kernel activation entry point).
    ///
    /// Combines `project detect`, `protocol status`, and `agent instructions`
    /// into a single read-only report. Reports project identity, protocol
    /// status, memory capsule/task-memory paths, stop conditions, warnings,
    /// failures, and recommended next steps.
    Preflight {
        /// Agent identifier: codex, claude-code, cursor, tencent-agent, workbuddy, codebuddy-code, cowork, or another host id
        #[arg(long = "for", value_name = "AGENT")]
        for_agent: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Verification operations — structured verification entry point.
///
/// Runs scoped verification checks with stable `CheckItem` model output.
/// `local` focuses on in-repo checks (fmt, test, build, fixtures, YAML,
/// preflight). `full` adds drift checks against stable and public targets.
/// `release` focuses on public-full sanitized boundary checks.
#[derive(Subcommand)]
enum VerifyAction {
    /// Run verification checks for the given scope.
    ///
    /// Scope determines which checks run:
    ///   local   — fmt, test, build, fixtures, YAML, preflight
    ///   full    — local + drift checks (stable, public)
    ///   release — release-focused boundary checks
    Run {
        /// Verification scope: local, full, or release
        #[arg(long, default_value = "local", value_parser = ["local", "full", "release"])]
        scope: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Target repository path, or private runtime home with --profile private.
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Classify the change lane for a git diff range (diff-aware verification).
    ///
    /// Maps the changed files in `--range` to a change lane and the
    /// minimal-sufficient verification profile. `--range` is required and never
    /// defaulted — pass the commit range actually under review (e.g.
    /// `<a1-head>..HEAD`, or `cached` / `staged` for the index).
    Lane {
        /// Git diff range, or `cached` / `staged` for the index.
        #[arg(long)]
        range: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Target repository path.
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
}

// ── MCP Server ─────────────────────────────────────────────────────────────

/// MCP server operations — run AGS as an MCP server.
///
/// First version supports stdio transport only. The server exposes
/// AGS governance tools, resources, and prompts for MCP hosts
/// (Tencent Agent, Codex, Cursor, Claude Code) to call as a global
/// governance capability.
///
/// AGS MCP and EvoMap MCP are parallel peers. AGS MCP does NOT
/// proxy, wrap, or broker EvoMap MCP calls.
#[derive(Subcommand)]
enum McpAction {
    /// Start the AGS MCP server on stdio.
    ///
    /// Reads line-delimited JSON-RPC 2.0 messages from stdin and writes
    /// responses to stdout. Stderr is reserved for server logging.
    /// Supports: initialize, tools/list, tools/call, resources/list,
    /// resources/read, prompts/list, prompts/get.
    Serve {
        /// Transport protocol — only "stdio" is supported in v1.
        #[arg(long, default_value = "stdio", value_parser = ["stdio"])]
        transport: String,
    },
}

/// `ags hooks` — manage repo-owned git hooks (opt-in, explicit confirmation).
#[derive(Subcommand)]
enum HooksAction {
    /// Install the AGS pre-push verification hook from templates/hooks/.
    ///
    /// Without --confirm this only prints the install plan (source template,
    /// destination .git/hooks/pre-push) and writes NOTHING. With --confirm it
    /// copies the template into .git/hooks/pre-push and marks it executable.
    /// Never installs silently. Uninstall by deleting .git/hooks/pre-push.
    Install {
        /// Actually write .git/hooks/pre-push (otherwise dry-run plan only).
        #[arg(long)]
        confirm: bool,
    },
}

// ── Release / Rollback actions ─────────────────────────────────────────────

/// Release verification operations — dry-run only, no apply to stable/public.
#[derive(Subcommand)]
enum ReleaseAction {
    /// Verify release readiness against a target.
    ///
    /// Checks drift, boundary, and allowlist compliance against the specified
    /// target (stable or public-full sanitized). Read-only, no files are written.
    Verify {
        /// Target: stable or public-full
        #[arg(long, default_value = "stable", value_parser = ["stable", "public", "public-core", "public-full", "public-full-sanitized"])]
        target: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan a release package — lists what files WOULD be included.
    ///
    /// Public profiles include the public Rust workspace and governance
    /// runtime, while excluding build output, local/private runtime state, real
    /// memory, preinstalled skill packs, local agent config, and EvoMap/GEP
    /// runtime surfaces.
    /// `private-full` includes everything. Dry-run only, nothing is written.
    Package {
        /// Package profile: public-full or private-full
        #[arg(long, default_value = "public-full", value_parser = ["public-full", "public-core", "private-full"])]
        profile: String,
        /// Dry-run: list files but do not write any package.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Rollback operations — plan only, no apply.
#[derive(Subcommand)]
enum RollbackAction {
    /// Plan a rollback — maps what would change without applying.
    ///
    /// Dry-run only: output a structured rollback plan with affected files,
    /// current state, and rollback target. Does not modify any files.
    Plan {
        /// Rollback profile. `private` plans rollback of the local AGS runtime home.
        #[arg(long, value_parser = ["private"])]
        profile: Option<String>,
        /// Target runtime home (default: $AGS_HOME or ~/.ags/runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Skill governance operations — read-only inventory and proposal.
///
/// Reads governance/skill-adoption-log.yaml, governance/skill-ignore-list.yaml,
/// and manifests/suite.yaml. All operations are read-only — scan/check/propose
/// only. Adopt/apply/rollback writes are not implemented; if any write subcommand
/// is added later it must be a dry-run stub with human confirm.
#[derive(Subcommand)]
enum SkillAction {
    /// Scan the suite manifest and governance files for skill status.
    ///
    /// Reports available, missing, disabled, and degraded skills with
    /// profile information (required/optional/personal).
    #[command(hide = true)]
    Scan {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Validate governance YAML files for schema compliance and consistency.
    ///
    /// Checks parseability, cross-references adoption log with manifest,
    /// and reports schema version consistency across files.
    #[command(hide = true)]
    Check {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Propose a management action on a capability — dry-run unless `--apply`.
    ///
    /// Actions: adopt, update, remove, uninstall, repair, verify. Without
    /// `--apply` nothing is written and no external installer runs. With
    /// `--apply` only AGS-owned host entry files are written (with backup);
    /// external installers/registrars (npx skills, lark-cli, claude mcp) are
    /// advised, never executed.
    #[command(hide = true)]
    Propose {
        /// Action: adopt, update, remove, uninstall, repair, or verify
        #[arg(long, value_parser = ["adopt", "update", "remove", "uninstall", "repair", "verify"])]
        action: String,
        /// Capability name to act on (skill / MCP / CLI-backed)
        #[arg(long = "skill")]
        skill: String,
        /// Confirm and perform AGS-owned writes. Without it, dry-run only.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify host visibility for a host (read-only).
    ///
    /// Claude Code and Codex: check `~/.claude/skills` / `~/.codex/skills`
    /// `SKILL.md` (symlink-aware) and `claude mcp list` / `codex mcp list`.
    /// Cursor is reserved (unsupported in this version; model fields are
    /// stable). Degrades, never panics, when a host CLI is unavailable.
    #[command(hide = true)]
    Verify {
        /// Host to verify: claude-code | codex (cursor reserved)
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (use as a post-apply
        /// gate). Without it, verify is informational and exits 0.
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Inventory skill assets on disk (global-skills/ and skill-packs/).
    ///
    /// Read-only scan of each SKILL.md front-matter; never reads secrets,
    /// tokens, credentials, or runtime files. Use --write to emit a Markdown
    /// report to governance/skills-inventory.md.
    #[command(hide = true)]
    Inventory {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Also write a Markdown report to governance/skills-inventory.md
        #[arg(long)]
        write: bool,
    },
}

// ── Top-level Commands ────────────────────────────────────────────────────

#[derive(Subcommand)]
enum Commands {
    /// Global setup: make AGS visible to host agents on this machine.
    Setup {
        /// Target runtime home (default: $AGS_HOME or ~/.ags/runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Include GEP/EvoMap planner recall MCP snippets.
        #[arg(long)]
        with_evomap: bool,
        /// Write setup files. Without --yes, setup prints a plan only.
        #[arg(long)]
        yes: bool,
        /// Overwrite differing files after writing .bak.<timestamp> backups.
        #[arg(long)]
        force: bool,
        /// Register AGS MCP servers in Claude Code user config after setup.
        #[arg(long)]
        register_claude: bool,
        /// Print plan only, even if --yes is omitted.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Onboard the current project into AGS governance.
    Init {
        /// Target project directory (default: current directory).
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Stable project slug for local memory paths.
        #[arg(long)]
        slug: Option<String>,
        /// Print the onboarding plan without writing files.
        #[arg(long)]
        dry_run: bool,
        /// Governance overlay mode: `local` (default) git-ignores AGS files via
        /// `.git/info/exclude`; `shared`/`tracked` keep them committed.
        #[arg(long, default_value = "local", value_parser = ["local", "shared", "tracked"])]
        mode: String,
        /// Untrack already-tracked AGS-owned overlay files via
        /// `git rm --cached` (local mode only; keeps the working copy).
        #[arg(long)]
        migrate_tracked_overlay: bool,
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Plan private AGS runtime installation. Read-only.
    #[command(hide = true)]
    Plan {
        /// Installation profile. Only `private` is currently supported.
        #[arg(long, value_parser = ["private"])]
        profile: String,
        /// Target runtime home (default: $AGS_HOME or ~/.ags/runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Include GEP/EvoMap planner recall MCP snippets.
        #[arg(long)]
        with_evomap: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Apply private AGS runtime installation.
    #[command(hide = true)]
    Apply {
        /// Installation profile. Only `private` is currently supported.
        #[arg(long, value_parser = ["private"])]
        profile: String,
        /// Target runtime home (default: $AGS_HOME or ~/.ags/runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Required confirmation for write-mode install.
        #[arg(long)]
        yes: bool,
        /// Overwrite differing files after writing .bak.<timestamp> backups.
        #[arg(long)]
        force: bool,
        /// Include GEP/EvoMap planner recall MCP snippets.
        #[arg(long)]
        with_evomap: bool,
        /// Register AGS MCP servers in Claude Code user config after apply.
        #[arg(long)]
        register_claude: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Task card operations
    #[command(hide = true)]
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Execution policy operations
    #[command(hide = true)]
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Workflow sync operations
    #[command(hide = true)]
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Diagnose AGS health. Use --fix for safe repair actions.
    Doctor {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Perform safe repair actions (default: read-only diagnosis only).
        #[arg(long)]
        fix: bool,
        /// Backward-compatible alias for --fix.
        #[arg(long, hide = true)]
        repair: bool,
        /// Dry-run: show what would be repaired without executing.
        #[arg(long)]
        dry_run: bool,
        /// Target directory (default: current directory).
        #[arg(long, default_value = ".")]
        target: PathBuf,
    },
    /// Bootstrap operations — plan, dry-run, and apply to a target.
    ///
    /// --dry-run checks the current workspace (Rust toolchain + structure).
    /// --apply writes bootstrap payload to a target directory.
    /// --apply REQUIRES --target; the target MUST be a tempdir or
    /// non-A/S/B directory.  Writing to A/S/B/B1/A1 or any suite root
    /// containing WORKSPACE.md is rejected.
    #[command(hide = true)]
    Bootstrap {
        /// Perform a dry run (no files are written).
        #[arg(long)]
        dry_run: bool,
        /// Apply bootstrap: write bootstrap payload to target directory.
        /// Requires --target.
        #[arg(long)]
        apply: bool,
        /// Target directory for bootstrap operations.
        /// Required with --apply; optional with --dry-run (default: current dir).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Gate operations (runner-facing, M3)
    #[command(hide = true)]
    Gate {
        #[command(subcommand)]
        action: GateAction,
    },

    // ── M2 Agent Awareness commands ───────────────────────────────────
    /// Project discovery and AGS integration detection (M2)
    #[command(hide = true)]
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Protocol file status and governance requirements (M2)
    #[command(hide = true)]
    Protocol {
        #[command(subcommand)]
        action: ProtocolAction,
    },
    /// Export agent-specific project instructions (M2)
    #[command(hide = true)]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },

    // ── M5 Capability Registry ───────────────────────────────────────
    /// Capability discovery and registry operations (M5)
    #[command(hide = true)]
    Capability {
        #[command(subcommand)]
        action: CapabilityAction,
    },

    // ── M6 Receipt / Compliance ──────────────────────────────────────
    /// Receipt generation and verification operations (M6)
    #[command(hide = true)]
    Receipt {
        #[command(subcommand)]
        action: ReceiptAction,
    },
    /// Compliance checking against policy gates (M6)
    #[command(hide = true)]
    Compliance {
        #[command(subcommand)]
        action: ComplianceAction,
    },

    // ── Session operations (M2 — kernel activation) ──────────────────
    /// Session preflight — aggregated agent wake-up check (M2)
    #[command(hide = true)]
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    // ── Skill governance operations ───────────────────────────────
    /// Review local skills and update advice.
    Skill {
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Show safe update guidance. Skill writes still require explicit user approval.
        #[arg(long)]
        fix: bool,
        #[command(subcommand)]
        action: Option<SkillAction>,
    },

    // ── Release / Rollback operations ──────────────────────────────
    /// Release verification and packaging — dry-run only
    #[command(hide = true)]
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
    /// Rollback planning — dry-run only, no apply
    #[command(hide = true)]
    Rollback {
        #[command(subcommand)]
        action: RollbackAction,
    },

    // ── MCP operations ─────────────────────────────────────────
    /// Start AGS MCP server — expose governance tools/resources/prompts
    /// to MCP hosts (Tencent Agent, Codex, Cursor, Claude Code). V1 supports
    /// stdio transport only. AGS MCP and EvoMap MCP are parallel peers.
    #[command(hide = true)]
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Manage repo-owned git hooks (opt-in; explicit --confirm required, never
    /// installs silently).
    #[command(hide = true)]
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    // ── Runner operations ──────────────────────────────────────────
    /// Run a task card through the gate-first execution pipeline.
    ///
    /// Flow: validate → gate → policy → adapter resolve → launch plan.
    /// The runner ONLY consumes resolved execution policy — it never reads
    /// raw task-card fields to decide permissions, parallelism, or launch args.
    ///
    /// --check-only stops after gate check. --dry-run outputs the full launch
    /// plan without launching. Without flags, outputs a plan for shell-wrapper
    /// dispatch.
    #[command(hide = true)]
    Run {
        /// Task card file (use "-" for stdin)
        path: String,

        /// Stop after gate check; exit 0 if allowed/confirm, 1 if stop.
        #[arg(long, default_value_t = false)]
        check_only: bool,

        /// Full pipeline, output structured launch plan, do not execute.
        #[arg(long, default_value_t = false)]
        dry_run: bool,

        /// Pass write approval to the policy resolver for Heavy tasks.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,

        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    // ── Verify operations ────────────────────────────────────────────
    /// Run scoped verification checks — structured, machine-readable reports
    #[command(hide = true)]
    Verify {
        /// Verification scope: local, full, or release
        #[arg(long, default_value = "local", value_parser = ["local", "full", "release"])]
        scope: String,
        /// Verification profile. `private` verifies the local AGS runtime home.
        #[arg(long, value_parser = ["private"])]
        profile: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Target repository path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Require GEP/EvoMap planner recall MCP snippets and host visibility.
        #[arg(long)]
        with_evomap: bool,
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
        #[arg(long = "targets", value_name = "NAME=PATH", num_args = 1.., value_parser = parse_target)]
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

// ── Shared dispatch functions (used by both M1 and M0 commands) ───────────

fn guard_writable_target(command: &str, target: &Path) {
    let target_path = guard_path(target);
    let protected_roots = [
        "/Volumes/Projects/example-private-suite",
        "/Volumes/Projects/remotes/example-private-suite.git",
        "/Volumes/Projects/example-stable-suite",
        "/Volumes/AI Project/ai-dev-env-bootstrap",
        "/Volumes/Projects/remotes/example-public-suite.git",
    ];

    for protected in &protected_roots {
        let protected_path = guard_path(Path::new(protected));
        if target_path == protected_path || target_path.starts_with(&protected_path) {
            eprintln!(
                "{command}: refused — target is a protected suite path: {}",
                target.display()
            );
            eprintln!("Write-mode operations must target a tempdir or non-A/S/B directory.");
            std::process::exit(1);
        }
    }

    if target_path.join("WORKSPACE.md").exists()
        || target_path.join("AGENT_SUITE_PROTOCOL.md").exists()
    {
        eprintln!(
            "{command}: refused — target appears to be a suite root: {}",
            target.display()
        );
        eprintln!("Write-mode operations must target a tempdir or non-A/S/B directory.");
        std::process::exit(1);
    }
}

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

fn sanitize_name(path: &str) -> String {
    path.trim_matches('/')
        .replace(['/', '\\', '.'], "-")
        .trim_matches('-')
        .to_string()
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
        "ags bootstrap --apply: refused — source is not a complete private suite root: {}",
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

// ── Private runtime install profile ───────────────────────────────────────

const PRIVATE_INSTALL_SCHEMA: &str = "2.4-private-install";
const PROJECT_INIT_SCHEMA: &str = "2.4-project-init";

#[derive(Debug, Clone)]
struct InstallFile {
    path: PathBuf,
    description: String,
    content: String,
    mode: Option<u32>,
}

#[derive(Debug, Clone)]
struct PrivateInstallPlan {
    profile: String,
    source_root: PathBuf,
    target: PathBuf,
    with_evomap: bool,
    files: Vec<InstallFile>,
    cleanup_dirs: Vec<PathBuf>,
}

fn default_private_runtime_home() -> PathBuf {
    if let Some(path) = std::env::var_os("AGS_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = ags_platform::home_dir() {
        return home.join(".ags").join("runtime");
    }
    PathBuf::from(".ags").join("runtime")
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

fn private_install_target(target: Option<PathBuf>) -> PathBuf {
    target.unwrap_or_else(default_private_runtime_home)
}

fn source_root_or_exit(command: &str) -> PathBuf {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ensure_bootstrap_source_repo(&root);
    if !root.join("crates/ags-cli/Cargo.toml").exists() {
        eprintln!("{command}: refused — run from the AGS private suite root.");
        std::process::exit(1);
    }
    root
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn claude_ags_command_content() -> String {
    format!(
        r#"---
description: AGS one-command setup, project onboarding, and governance
argument-hint: [setup|init|preflight|doctor|verify|request...]
---

# AGS

This is the post-install AGS operator surface. Route by the first token in `$ARGUMENTS`.

## `/ags setup`

Initialize this machine into AGS with one user command. Run these steps without asking for another confirmation unless credentials, sudo, or destructive replacement is required:

```bash
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v ags >/dev/null 2>&1; then
  echo "AGS CLI is not on PATH. Run the AGS one-line installer first, then retry /ags setup." >&2
  exit 127
fi

if ! command -v gep-mcp-server >/dev/null 2>&1; then
  if command -v npm >/dev/null 2>&1; then
    npm install -g @evomap/gep-mcp-server
  else
    echo "npm is not available; continuing with AGS core setup and reporting EvoMap recall as unavailable." >&2
  fi
fi

if command -v gep-mcp-server >/dev/null 2>&1; then
  ags setup --with-evomap --yes --force --register-claude
  ags verify --profile private --with-evomap
else
  ags setup --yes --force --register-claude
  ags verify --profile private
fi

claude mcp list
```

Expected result: `ags`, `/ags`, and Claude Code MCP registration are ready on this machine.

## `/ags init`

Onboard the current repository into AGS governance with one user command:

```bash
ags init --target .
ags session preflight --for claude-code --target .
```

Aliases: `/ags onboard`, `/ags manage`, `/ags 纳管`.

## Other routes

- Empty or `preflight`: report the AGS preflight result and next allowed actions.
- `doctor`: run `ags doctor --target .` and summarize the findings.
- `verify`: run `ags verify --scope local --target .` and summarize the check results.
- Any other text: treat it as the user request. Prefer MCP `ags_preflight` first; if MCP is unavailable, run `ags session preflight --for claude-code --target .`. Complete AGS solution formation and do not generate an executable task card until the user explicitly asks for one.

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
            "初始化本机 AGS 环境：运行 `ags setup --with-evomap --yes --force --register-claude`，然后用 `ags verify --profile private --with-evomap` 校验",
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
            "管理第三方技能：运行 `ags skill` 查看概览，或运行 `ags skill --fix`、`ags skill scan`、`ags skill check`、`ags skill propose --action adopt --skill <name>` 生成纳管建议",
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

这是 Codex 顶层 AGS 命令技能，用来把明确的 AGS 操作路由到已安装的 `ags` CLI 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先运行 AGS preflight：

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

fn private_install_plan(
    source_root: &Path,
    target: &Path,
    with_evomap: bool,
) -> PrivateInstallPlan {
    let ags_mcp_json = r#"{
  "mcpServers": {
    "ags": {
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "AGS_RUNTIME_HOME": "__TARGET__"
      }
    },
    "codegraph": {
      "command": "codegraph",
      "args": ["serve", "--mcp"]
    }
  },
  "initialization_gate": {
    "mandatory_first_tool": "ags_preflight",
    "failed_preflight_opens_gate": false
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let codex_snippet = r#"# AGS MCP host initialization adapter
# Merge this snippet into ~/.codex/config.toml after review.
[mcp_servers.ags]
command = "ags"
args = ["mcp", "serve", "--transport", "stdio"]

[mcp_servers.ags.env]
AGS_RUNTIME_HOME = "__TARGET__"

[mcp_servers.codegraph]
command = "codegraph"
args = ["serve", "--mcp"]
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let claude_snippet = r#"{
  "mcpServers": {
    "ags": {
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "env": {
        "AGS_RUNTIME_HOME": "__TARGET__"
      }
    },
    "codegraph": {
      "command": "codegraph",
      "args": ["serve", "--mcp"]
    }
  },
  "hooks": {
    "Stop": [
      {
        "command": "node __TARGET__/hooks/claude-code-executor-stop.js",
        "timeout": 8
      }
    ]
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let gep_mcp_json = r#"{
  "mcpServers": {
    "gep": {
      "command": "gep-mcp-server",
      "env": {
        "EVOMAP_HUB_URL": "https://evomap.ai",
        "GEP_ASSETS_DIR": "__TARGET__/evomap/gep-assets",
        "GEP_MEMORY_DIR": "__TARGET__/evomap/evolution-memory"
      }
    },
    "evolver_proxy": {
      "command": "__TARGET__/bin/evolver-proxy-mcp"
    }
  },
  "boundary": {
    "role": "planner_advisory_only",
    "ags_authority": "lifecycle_gates_task_level_permission_review_verify_release",
    "evomap_authority": "solution_formation_method_recall_only"
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let claude_evomap_snippet = r#"{
  "mcpServers": {
    "gep": {
      "command": "gep-mcp-server",
      "env": {
        "EVOMAP_HUB_URL": "https://evomap.ai",
        "GEP_ASSETS_DIR": "__TARGET__/evomap/gep-assets",
        "GEP_MEMORY_DIR": "__TARGET__/evomap/evolution-memory"
      }
    },
    "evolver_proxy": {
      "command": "__TARGET__/bin/evolver-proxy-mcp"
    }
  },
  "usage": {
    "phase": "solution_formation_only",
    "rule": "Call AGS first for preflight and gates. Call GEP/EvoMap separately only for advisory recall."
  }
}
"#
    .replace("__TARGET__", &target.to_string_lossy());

    // Tencent Agent is the platform family; WorkBuddy and CodeBuddy-Code are
    // host clients. These snippets are host-platform MCP registrations for AGS,
    // not task-card runtime adapters and not execution-policy authority.
    let host_platform_mcp_snippet = |client_note: &str| -> String {
        format!(
            r#"{{
  "mcpServers": {{
    "ags": {{
      "role": "host_initialization_adapter",
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "mandatory_first_tool": "ags_preflight",
      "_comment": "{client_note}"
    }}
  }}
}}
"#
        )
    };
    let tencent_agent_snippet = host_platform_mcp_snippet(
        "Tencent Agent platform MCP registration for AGS. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients sharing this AGS MCP entry.",
    );
    let workbuddy_snippet = host_platform_mcp_snippet(
        "WorkBuddy (Tencent Agent host client) platform MCP registration for AGS.",
    );
    let codebuddy_code_snippet = host_platform_mcp_snippet(
        "CodeBuddy-Code (Tencent Agent host client) platform MCP registration for AGS.",
    );

    let profile = std::fs::read_to_string(
        source_root.join("manifests/templates/runtime-profiles.template.yaml"),
    )
    .unwrap_or_default()
    .replace("http://127.0.0.1:PORT", "http://127.0.0.1:19821")
    .replace(
        "path/to/evolver-token.txt",
        &target.join("secrets/evolver-token.txt").to_string_lossy(),
    )
    .replace(
        "node .claude/hooks/evolver-session-end.js",
        &format!(
            "node {}",
            target
                .join("hooks/claude-code-executor-stop.js")
                .to_string_lossy()
        ),
    );

    let claude_hook = std::fs::read_to_string(
        source_root.join("manifests/templates/hooks/claude-code-executor-stop.template.js"),
    )
    .unwrap_or_default();
    let codex_hook = std::fs::read_to_string(
        source_root.join("manifests/templates/hooks/codex-planner-recall.template.json"),
    )
    .unwrap_or_default();

    let launcher = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nexport AGS_RUNTIME_HOME={}\nexec ags mcp serve --transport stdio\n",
        shell_quote(target)
    );

    let evolver_proxy_launcher = r#"#!/usr/bin/env bash
set -euo pipefail

export AGS_RUNTIME_HOME="${AGS_RUNTIME_HOME:-__TARGET__}"

if [[ -x "$HOME/.codex/bin/ensure-evolver-proxy" ]]; then
  "$HOME/.codex/bin/ensure-evolver-proxy" >/dev/null || true
fi

if [[ -n "${EVOLVER_PROXY_MCP:-}" ]]; then
  if [[ -x "$EVOLVER_PROXY_MCP" ]]; then
    exec "$EVOLVER_PROXY_MCP" "$@"
  fi
  if [[ -f "$EVOLVER_PROXY_MCP" ]]; then
    exec node "$EVOLVER_PROXY_MCP" "$@"
  fi
  echo "EVOLVER_PROXY_MCP is set but not executable/readable: $EVOLVER_PROXY_MCP" >&2
  exit 127
fi

candidates=(
  "$AGS_RUNTIME_HOME/evomap/evolver/mcp/evolver-proxy.mjs"
  "$HOME/plugins/evolver/mcp/evolver-proxy.mjs"
  "$HOME/.evolver/mcp/evolver-proxy.mjs"
  "$HOME/.local/share/evolver/mcp/evolver-proxy.mjs"
)

for candidate in "${candidates[@]}"; do
  if [[ -f "$candidate" ]]; then
    exec node "$candidate" "$@"
  fi
done

cat >&2 <<'EOF'
AGS managed-device runtime requires Evolver Proxy, but no local proxy entrypoint was found.

Expected one of:
  - $EVOLVER_PROXY_MCP
  - $AGS_RUNTIME_HOME/evomap/evolver/mcp/evolver-proxy.mjs
  - $HOME/plugins/evolver/mcp/evolver-proxy.mjs
  - $HOME/.evolver/mcp/evolver-proxy.mjs
  - $HOME/.local/share/evolver/mcp/evolver-proxy.mjs

Install or sync the Evolver runtime for this device, then rerun:
  claude mcp remove evolver_proxy -s user
  claude mcp add -s user evolver_proxy -- "$AGS_RUNTIME_HOME/bin/evolver-proxy-mcp"
  ags doctor --target "$AGS_RUNTIME_HOME"
EOF
exit 127
"#
    .replace("__TARGET__", &target.to_string_lossy());

    let manifest = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": "private",
        "source_root": source_root.to_string_lossy(),
        "target": target.to_string_lossy(),
        "mcp": {
            "server": "ags",
            "command": "ags mcp serve --transport stdio",
            "mandatory_first_tool": "ags_preflight"
        },
        "evomap": {
            "boundary": "parallel_peer_not_brokered",
            "with_evomap": with_evomap,
            "mcp": if with_evomap { serde_json::json!({
                "servers": ["gep", "evolver_proxy"],
                "phase": "solution_formation_only",
                "managed_device_runtime": true,
                "claude_code_global_hints": [
                    "claude mcp add -s user gep -- gep-mcp-server",
                    format!("claude mcp add -s user evolver_proxy -- {}", target.join("bin/evolver-proxy-mcp").to_string_lossy())
                ]
            }) } else { serde_json::json!({
                "status": "not_requested",
                "next_action": "rerun ags setup --with-evomap --yes to include managed-device EvoMap runtime"
            }) },
            "runtime_profile": "manifests/runtime-profiles.yaml",
            "token_file_expected": "secrets/evolver-token.txt"
        },
        "host_snippets": if with_evomap { serde_json::json!([
            "hosts/codex.config.snippet.toml",
            "hosts/claude-code.mcp.snippet.json",
            "hosts/claude-code.evomap-mcp.snippet.json",
            "hosts/tencent-agent.mcp.snippet.json",
            "hosts/workbuddy.mcp.snippet.json",
            "hosts/codebuddy-code.mcp.snippet.json"
        ]) } else { serde_json::json!([
            "hosts/codex.config.snippet.toml",
            "hosts/claude-code.mcp.snippet.json",
            "hosts/tencent-agent.mcp.snippet.json",
            "hosts/workbuddy.mcp.snippet.json",
            "hosts/codebuddy-code.mcp.snippet.json"
        ]) },
        "host_commands": {
            "claude_code": {
                "slash_command": "/ags",
                "path": claude_ags_command_path().to_string_lossy()
            },
            "codex": {
                "command_skills": codex_ags_command_skill_specs()
                    .iter()
                    .map(|(name, _, _, _, _)| codex_ags_named_skill_path(name).to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
                "retired_visible_skills": retired_codex_ags_skill_dirs()
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
            }
        },
        "created_by": "ags setup",
    });

    let readme = format!(
        "# AGS Private Runtime\n\n\
This directory was generated by `ags setup`.\n\n\
## Commands\n\n\
- MCP server: `ags mcp serve --transport stdio`\n\
- Doctor: `ags doctor`\n\
- Runtime check: `ags doctor --target {}`\n\n\
## Host snippets\n\n\
Review files in `hosts/` before merging them into host-specific global config.\n\
AGS scenarios must call `ags_preflight` before any other AGS tool.\n\n\
## Claude Code slash command\n\n\
The one-line installer seeds `/ags`; `ags setup --yes` refreshes it at `~/.claude/commands/ags.md`.\n\
Use `/ags setup` to initialize this machine and `/ags init` to onboard the current project.\n\
Diagnostics remain available as `/ags preflight` and `/ags doctor`; verification gates drive `ags verify` internally.\n\n\
## Codex skills\n\n\
`ags setup --yes` installs visible top-level command skills: `$ags-setup`, `$ags-init`, `$ags-skill`, and `$ags-doctor`.\n\
Retired visible skills (`$ags`, `$ags-preflight`, `$ags-verify`) are removed from the Codex skill list during setup.\n\
`ags verify` remains a kernel/CI verification command and is not installed as a visible Codex skill.\n\
Each command skill routes through AGS preflight before acting.\n\n\
## EvoMap boundary\n\n\
EvoMap remains a parallel peer. AGS MCP does not proxy EvoMap MCP calls.\n\
Install managed-device EvoMap runtime explicitly with `ags setup --with-evomap --yes`.\n\
Place the local bearer token, if needed, at `secrets/evolver-token.txt` with mode 0600.\n",
        target.display()
    );

    let mut files = vec![
        InstallFile {
            path: target.join("install-manifest.json"),
            description: "machine-readable private runtime install manifest".to_string(),
            content: serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n",
            mode: None,
        },
        InstallFile {
            path: target.join("README.md"),
            description: "operator notes for this private runtime home".to_string(),
            content: readme,
            mode: None,
        },
        InstallFile {
            path: target.join("mcp/ags.mcp.json"),
            description: "generic MCP registration snippet for AGS host adapter".to_string(),
            content: ags_mcp_json,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/codex.config.snippet.toml"),
            description: "Codex MCP config snippet".to_string(),
            content: codex_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/claude-code.mcp.snippet.json"),
            description: "Claude Code MCP and Stop hook snippet".to_string(),
            content: claude_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/tencent-agent.mcp.snippet.json"),
            description: "Tencent Agent platform MCP registration snippet for AGS".to_string(),
            content: tencent_agent_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/workbuddy.mcp.snippet.json"),
            description: "WorkBuddy platform MCP registration snippet for AGS".to_string(),
            content: workbuddy_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("hosts/codebuddy-code.mcp.snippet.json"),
            description: "CodeBuddy-Code platform MCP registration snippet for AGS".to_string(),
            content: codebuddy_code_snippet,
            mode: None,
        },
        InstallFile {
            path: target.join("manifests/runtime-profiles.yaml"),
            description: "private EvoMap runtime profile with local-safe defaults".to_string(),
            content: profile,
            mode: None,
        },
        InstallFile {
            path: target.join("hooks/claude-code-executor-stop.js"),
            description: "Claude Code executor Stop hook".to_string(),
            content: claude_hook,
            mode: Some(0o755),
        },
        InstallFile {
            path: target.join("hooks/codex-planner-recall.json"),
            description: "Codex/Cursor planner advisory recall hook template".to_string(),
            content: codex_hook,
            mode: None,
        },
        InstallFile {
            path: target.join("bin/ags-mcp-stdio.sh"),
            description: "portable launcher for AGS MCP stdio server".to_string(),
            content: launcher,
            mode: Some(0o755),
        },
        InstallFile {
            path: target.join("secrets/README.md"),
            description: "private token placement note; no secret is generated".to_string(),
            content: "Place evolver-token.txt here if EvoMap proxy auth is required. Do not commit this directory.\n".to_string(),
            mode: None,
        },
        InstallFile {
            path: claude_ags_command_path(),
            description: "Claude Code user slash command for AGS governance".to_string(),
            content: claude_ags_command_content(),
            mode: None,
        },
        InstallFile {
            path: target.join("project-templates/scripts/validate.sh"),
            description: "portable project task-card validator wrapper".to_string(),
            content: portable_validate_script(),
            mode: Some(0o755),
        },
    ];

    for (name, display_name, short_description, default_prompt, summary) in
        codex_ags_command_skill_specs()
    {
        files.push(InstallFile {
            path: codex_ags_named_skill_path(name),
            description: format!("Codex AGS command skill: {name}"),
            content: codex_ags_command_skill_content(name, display_name, summary),
            mode: None,
        });
        files.push(InstallFile {
            path: codex_ags_named_skill_agent_metadata_path(name),
            description: format!("Codex AGS command skill UI metadata: {name}"),
            content: codex_ags_command_skill_agent_metadata_content(
                display_name,
                short_description,
                default_prompt,
            ),
            mode: None,
        });
    }

    for name in project_protocol_files() {
        let src = source_root.join("protocol").join(name);
        if let Ok(content) = std::fs::read_to_string(&src) {
            files.push(InstallFile {
                path: target.join("project-templates/protocol").join(name),
                description: format!("project onboarding protocol template: protocol/{name}"),
                content,
                mode: None,
            });
        }
    }

    if with_evomap {
        files.push(InstallFile {
            path: target.join("mcp/gep.mcp.json"),
            description: "generic MCP registration snippet for GEP/EvoMap planner recall"
                .to_string(),
            content: gep_mcp_json,
            mode: None,
        });
        files.push(InstallFile {
            path: target.join("hosts/claude-code.evomap-mcp.snippet.json"),
            description: "Claude Code GEP/EvoMap MCP planner recall snippet".to_string(),
            content: claude_evomap_snippet,
            mode: None,
        });
        files.push(InstallFile {
            path: target.join("bin/evolver-proxy-mcp"),
            description: "AGS managed-device Evolver Proxy MCP launcher".to_string(),
            content: evolver_proxy_launcher,
            mode: Some(0o755),
        });
        files.push(InstallFile {
            path: target.join("evomap/README.md"),
            description: "EvoMap/GEP optional advisory recall notes".to_string(),
            content: "Managed-device EvoMap workspace. AGS remains the governance authority; GEP provides advisory recall and Evolver Proxy provides the device runtime bridge.\n".to_string(),
            mode: None,
        });
    }

    PrivateInstallPlan {
        profile: "private".to_string(),
        source_root: source_root.to_path_buf(),
        target: target.to_path_buf(),
        with_evomap,
        files,
        cleanup_dirs: retired_codex_ags_skill_dirs(),
    }
}

fn project_protocol_files() -> &'static [&'static str] {
    &[
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
        "task-routing.md",
        "project-profile.md",
        "context-memory.md",
    ]
}

fn portable_validate_script() -> String {
    "#!/usr/bin/env bash\n# AGS portable task-card validator wrapper.\nset -euo pipefail\nexec ags task validate \"$@\"\n".to_string()
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn default_project_slug(target: &Path) -> String {
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("project");
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "project".to_string()
    } else {
        out
    }
}

fn home_dir() -> PathBuf {
    ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn project_memory_dir(slug: &str) -> PathBuf {
    home_dir()
        .join(".agents")
        .join("memory")
        .join("projects")
        .join(slug)
}

fn project_template_protocol_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let suite_protocol = cwd.join("protocol");
    if suite_protocol.join("agent-task-protocol.md").exists() {
        return Some(suite_protocol);
    }

    if let Some(runtime_home) = std::env::var_os("AGS_RUNTIME_HOME").map(PathBuf::from) {
        let dir = runtime_home.join("project-templates/protocol");
        if dir.join("agent-task-protocol.md").exists() {
            return Some(dir);
        }
    }

    let dir = default_private_runtime_home().join("project-templates/protocol");
    if dir.join("agent-task-protocol.md").exists() {
        Some(dir)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct ProjectInitPlan {
    target: PathBuf,
    slug: String,
    memory_dir: PathBuf,
    files: Vec<InstallFile>,
    append_files: Vec<InstallFile>,
    directories: Vec<PathBuf>,
    warnings: Vec<String>,
}

fn project_init_plan(target: &Path, slug: Option<String>) -> ProjectInitPlan {
    let canonical = guard_path(target);
    let slug = slug.unwrap_or_else(|| default_project_slug(&canonical));
    let memory_dir = project_memory_dir(&slug);
    let protocol_dir = project_template_protocol_dir();
    let mut files = Vec::new();
    let mut append_files = Vec::new();
    let mut directories = vec![
        canonical.join("config"),
        canonical.join("protocol"),
        canonical.join("scripts"),
        memory_dir.join("task-archive"),
        memory_dir.join("sessions"),
    ];
    let mut warnings = Vec::new();

    let ags_block = format!(
        "\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}.\n\n- Run `ags doctor --target .` to diagnose local governance health.\n- AGS MCP hosts must call `ags_preflight` before other AGS tools.\n- CLI fallback: `ags session preflight --for <agent-id> --target .`.\n- Known agents get tailored instructions; unknown non-empty agent ids use the generic governed-host profile.\n- Protocol entry points: `AGENT_SUITE_PROTOCOL.md`, `CLAUDE.md`, `protocol/agent-task-protocol.md`, and `protocol/task-routing.md`.\n- Task cards must be validated with the task-card-validator via `bash scripts/validate.sh <task-card>`.\n"
    );

    files.push(InstallFile {
        path: canonical.join("AGENTS.md"),
        description: "agent entrypoint with AGS governance reference".to_string(),
        content: format!("# AGENTS.md\n\n@CLAUDE.md\n{ags_block}"),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join("AGENTS.md"),
        description: "append AGS governance block to existing AGENTS.md".to_string(),
        content: ags_block.clone(),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("CLAUDE.md"),
        description: "Claude Code AGS execution protocol entrypoint".to_string(),
        content: format!(
            "# CLAUDE.md\n\nThis project is governed by Agent Governance Suite {AGS_VERSION}.\n\nBefore task execution, run AGS preflight through MCP (`ags_preflight`) or CLI fallback:\n\n```bash\nags session preflight --for claude-code --target .\n```\n\nDo not classify tasks from raw requests. Follow solution formation, user confirmation, task-card request gate, execution contract, routing, gate, verification, and receipt rules from `protocol/agent-task-protocol.md`.\n"
        ),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join("CLAUDE.md"),
        description: "append AGS execution protocol block to existing CLAUDE.md".to_string(),
        content: format!("\n## Agent Governance Suite\n\nThis project is governed by AGS {AGS_VERSION}. Run `ags_preflight` through MCP or `ags session preflight --for claude-code --target .` before execution. Follow `protocol/agent-task-protocol.md` and `protocol/task-routing.md`.\n"),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "ignore AGS/GEP local runtime data".to_string(),
        content: "# AGS/GEP local runtime data\nassets/gep/\n".to_string(),
        mode: None,
    });
    append_files.push(InstallFile {
        path: canonical.join(".gitignore"),
        description: "append AGS/GEP local runtime ignore rules".to_string(),
        content: "\n# AGS/GEP local runtime data\nassets/gep/\n".to_string(),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("AGENT_SUITE_PROTOCOL.md"),
        description: "project-local AGS protocol pointer".to_string(),
        content: format!("# AGENT_SUITE_PROTOCOL.md\n\nThis project is integrated with Agent Governance Suite {AGS_VERSION}.\n\nCanonical governance entry points:\n\n- `AGENTS.md`\n- `CLAUDE.md`\n- `protocol/agent-task-protocol.md`\n- `protocol/task-routing.md`\n- `config/agent-project-profile.yaml`\n\nHosts must call AGS preflight before AGS-governed work.\n"),
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("WORKSPACE.md"),
        description: "project-local AGS workspace marker".to_string(),
        content: format!(
            "# WORKSPACE.md\n\n| Code | Role | Path |\n|---|---|---|\n| P | AGS-integrated project | {} |\n\nThis file marks the repository as an AGS-managed project, not an AGS suite root.\n",
            canonical.display()
        ),
        mode: None,
    });

    let profile = format!(
        "schema_version: 1\nproject:\n  name: {}\n  slug: {}\n  type: {}\n  primary_languages: []\n  primary_runtime: {}\n\ndefaults:\n  executor: {}\n  runtime_adapter: {}\n  execution_surface: {}\n  permission_mode_by_level:\n    light: execute-and-verify\n    medium: edit-with-confirmation\n    heavy: plan-only\n  parallelism: none\n\nverification:\n  default_commands:\n    - ags doctor --target .\n  smoke_commands: []\n  expensive_commands: []\n  evidence_required:\n    - command\n    - exit_code\n\nrisk:\n  high_risk_paths:\n    - AGENTS.md\n    - CLAUDE.md\n    - AGENT_SUITE_PROTOCOL.md\n    - config/agent-project-profile.yaml\n    - protocol/\n  protected_paths:\n    - $HOME/.agents/memory/projects/{}/context-capsule.md\n  destructive_actions_require_confirmation: true\n  heavy_triggers:\n    - protocol changes\n    - hook installation\n    - production wiring\n  stop_conditions:\n    - Do not overwrite user-owned files without explicit confirmation.\n\nworkflow:\n  governance_docs:\n    - AGENTS.md\n    - CLAUDE.md\n    - AGENT_SUITE_PROTOCOL.md\n    - protocol/agent-task-protocol.md\n    - protocol/task-routing.md\n  context_memory_capsule: {}\n  task_memory: {}\n  task_archive: {}\n  default_review_policy: Codex review before release\n  delivery_report: protocol/agent-task-protocol.md\n\nuser_preferences:\n  interaction_style: {}\n  ask_before:\n    - destructive commands\n    - hook installation\n    - dependency installation\n  do_not_do:\n    - overwrite project memory design purpose automatically\n",
        yaml_string(
            canonical
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("project")
        ),
        yaml_string(&slug),
        yaml_string("ags-integrated-project"),
        yaml_string("project-defined"),
        yaml_string("codex"),
        yaml_string("ags-mcp-or-cli-fallback"),
        yaml_string("local-workspace"),
        slug,
        yaml_string(&memory_dir.join("context-capsule.md").to_string_lossy()),
        yaml_string(&memory_dir.join("task-memory.md").to_string_lossy()),
        yaml_string(&memory_dir.join("task-archive").to_string_lossy()),
        yaml_string("concise, evidence-first, ask before high-risk writes"),
    );
    files.push(InstallFile {
        path: canonical.join("config/agent-project-profile.yaml"),
        description: "AGS project profile".to_string(),
        content: profile,
        mode: None,
    });

    files.push(InstallFile {
        path: canonical.join("scripts/validate.sh"),
        description: "portable project task-card validator wrapper".to_string(),
        content: portable_validate_script(),
        mode: Some(0o755),
    });

    files.push(InstallFile {
        path: memory_dir.join("context-capsule.md"),
        description: "manual project memory capsule".to_string(),
        content: format!(
            "# Context Capsule: {slug}\n\nManual-maintained stable project memory.\n\n## 项目设计目的\n\nTODO: describe this project's purpose. This section is human-maintained and must not be overwritten by automated capture.\n\n## Stable Facts\n\n- Project path: `{}`\n- Memory dir: `{}`\n\n## 自动记忆入口\n\n- Task memory: `{}`\n- Task archive: `{}`\n- Sessions: `{}`\n",
            canonical.display(),
            memory_dir.display(),
            memory_dir.join("task-memory.md").display(),
            memory_dir.join("task-archive").display(),
            memory_dir.join("sessions").display(),
        ),
        mode: None,
    });
    files.push(InstallFile {
        path: memory_dir.join("task-memory.md"),
        description: "task continuity memory entrypoint".to_string(),
        content: format!(
            "# Task Memory: {slug}\n\nNo AGS task archives have been captured yet.\n\nThe manual project charter remains in `context-capsule.md`.\n"
        ),
        mode: None,
    });

    if let Some(protocol_dir) = protocol_dir {
        for name in project_protocol_files() {
            let src = protocol_dir.join(name);
            match std::fs::read_to_string(&src) {
                Ok(content) => files.push(InstallFile {
                    path: canonical.join("protocol").join(name),
                    description: format!("AGS protocol file: protocol/{name}"),
                    content,
                    mode: None,
                }),
                Err(e) => warnings.push(format!(
                    "cannot read protocol template {}: {}",
                    src.display(),
                    e
                )),
            }
        }
    } else {
        warnings.push(
            "no AGS protocol templates found; run `ags setup --yes` or invoke init from the AGS suite root"
                .to_string(),
        );
    }

    directories.sort();
    directories.dedup();

    ProjectInitPlan {
        target: canonical,
        slug,
        memory_dir,
        files,
        append_files,
        directories,
        warnings,
    }
}

fn project_file_status(file: &InstallFile, append_candidates: &[InstallFile]) -> &'static str {
    if !file.path.exists() {
        return "would-create";
    }
    if append_candidates
        .iter()
        .any(|candidate| candidate.path == file.path)
    {
        if let Ok(existing) = std::fs::read_to_string(&file.path) {
            if existing.contains("Agent Governance Suite")
                || existing.contains(&format!("AGS {AGS_VERSION}"))
            {
                "exists"
            } else {
                "would-append"
            }
        } else {
            "exists"
        }
    } else {
        "exists"
    }
}

fn render_project_init_text(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let mut lines = vec![
        format!("AGS Project Init Plan {}", PROJECT_INIT_SCHEMA),
        format!("Target: {}", plan.target.display()),
        format!("Slug:   {}", plan.slug),
        format!("Memory: {}", plan.memory_dir.display()),
        format!("Mode:   {}", if dry_run { "dry-run" } else { "apply" }),
        String::new(),
        "Directories:".to_string(),
    ];
    for dir in &plan.directories {
        let status = if dir.exists() {
            "exists"
        } else {
            "would-create"
        };
        lines.push(format!("  - [{status}] {}", dir.display()));
    }
    lines.push(String::new());
    lines.push("Files:".to_string());
    for file in &plan.files {
        lines.push(format!(
            "  - [{}] {} — {}",
            project_file_status(file, &plan.append_files),
            file.path.display(),
            file.description
        ));
    }
    if !plan.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for warning in &plan.warnings {
            lines.push(format!("  ! {warning}"));
        }
    }
    lines.join("\n")
}

fn render_project_init_json(plan: &ProjectInitPlan, dry_run: bool) -> String {
    let directories: Vec<_> = plan
        .directories
        .iter()
        .map(|dir| {
            serde_json::json!({
                "path": dir.to_string_lossy(),
                "status": if dir.exists() { "exists" } else { "would-create" },
            })
        })
        .collect();
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path.to_string_lossy(),
                "description": file.description,
                "status": project_file_status(file, &plan.append_files),
                "mode": file.mode.map(|m| format!("{m:o}")),
            })
        })
        .collect();
    let output = serde_json::json!({
        "schema_version": PROJECT_INIT_SCHEMA,
        "target": plan.target.to_string_lossy(),
        "slug": plan.slug,
        "memory_dir": plan.memory_dir.to_string_lossy(),
        "dry_run": dry_run,
        "directories": directories,
        "files": files,
        "warnings": plan.warnings,
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}

fn write_project_init_file(
    file: &InstallFile,
    append_candidates: &[InstallFile],
) -> suite_doctor::Finding {
    if let Some(parent) = file.path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return suite_doctor::Finding::fail(
                format!(
                    "project-init-{}",
                    sanitize_name(&file.path.to_string_lossy())
                ),
                format!("cannot create directory {}", parent.display()),
                e.to_string(),
            );
        }
    }

    if file.path.exists() {
        if let Some(append) = append_candidates
            .iter()
            .find(|candidate| candidate.path == file.path)
        {
            match std::fs::read_to_string(&file.path) {
                Ok(existing) if existing.contains(append.content.trim()) => {
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(existing)
                    if existing.contains("Agent Governance Suite")
                        || existing.contains(&format!("AGS {AGS_VERSION}")) =>
                {
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("unchanged: {}", file.path.display()),
                    );
                }
                Ok(_) => {
                    if let Err(e) = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&file.path)
                        .and_then(|mut f| {
                            use std::io::Write;
                            f.write_all(append.content.as_bytes())
                        })
                    {
                        return suite_doctor::Finding::fail(
                            format!(
                                "project-init-{}",
                                sanitize_name(&file.path.to_string_lossy())
                            ),
                            format!("append failed: {}", file.path.display()),
                            e.to_string(),
                        );
                    }
                    return suite_doctor::Finding::pass(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("appended AGS block: {}", file.path.display()),
                    );
                }
                Err(e) => {
                    return suite_doctor::Finding::fail(
                        format!(
                            "project-init-{}",
                            sanitize_name(&file.path.to_string_lossy())
                        ),
                        format!("read failed: {}", file.path.display()),
                        e.to_string(),
                    );
                }
            }
        }

        return suite_doctor::Finding::pass(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("kept existing: {}", file.path.display()),
        );
    }

    if let Err(e) = std::fs::write(&file.path, &file.content) {
        return suite_doctor::Finding::fail(
            format!(
                "project-init-{}",
                sanitize_name(&file.path.to_string_lossy())
            ),
            format!("write failed: {}", file.path.display()),
            e.to_string(),
        );
    }

    #[cfg(unix)]
    if let Some(mode) = file.mode {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&file.path) {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(&file.path, perms);
        }
    }

    suite_doctor::Finding::pass(
        format!(
            "project-init-{}",
            sanitize_name(&file.path.to_string_lossy())
        ),
        format!("written: {}", file.path.display()),
    )
}

fn cmd_project_init(
    target: &Path,
    slug: Option<String>,
    dry_run: bool,
    format: &str,
    mode: OverlayMode,
    migrate: bool,
) {
    if !target.exists() {
        eprintln!("ags init: target does not exist — {}", target.display());
        std::process::exit(1);
    }
    let plan = project_init_plan(target, slug);
    let overlay = compute_overlay_plan(&plan.target, &plan.files, mode, migrate);
    if dry_run {
        match format {
            "json" => {
                let mut value: serde_json::Value =
                    serde_json::from_str(&render_project_init_json(&plan, true))
                        .unwrap_or_default();
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("overlay".to_string(), overlay_json(&overlay));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            }
            _ => {
                println!("{}", render_project_init_text(&plan, true));
                println!();
                println!("{}", render_overlay_text(&overlay));
            }
        }
        return;
    }

    let mut report = suite_doctor::HealthReport::new("project-init");
    for dir in &plan.directories {
        match std::fs::create_dir_all(dir) {
            Ok(_) => report.add(suite_doctor::Finding::pass(
                format!("project-init-dir-{}", sanitize_name(&dir.to_string_lossy())),
                format!("directory ready: {}", dir.display()),
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                format!("project-init-dir-{}", sanitize_name(&dir.to_string_lossy())),
                format!("cannot create directory: {}", dir.display()),
                e.to_string(),
            )),
        }
    }
    for file in &plan.files {
        report.add(write_project_init_file(file, &plan.append_files));
    }
    for warning in &plan.warnings {
        report.add(suite_doctor::Finding::warn(
            format!("project-init-warning-{}", sanitize_name(warning)),
            warning,
            "project init completed with a warning",
        ));
    }
    for finding in apply_overlay(&overlay) {
        report.add(finding);
    }

    let preflight = project_discovery::run_session_preflight(
        &plan.target,
        &project_discovery::AgentType::Codex,
    );
    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PROJECT_INIT_SCHEMA,
                "plan": serde_json::from_str::<serde_json::Value>(&render_project_init_json(&plan, false)).unwrap_or_default(),
                "overlay": overlay_json(&overlay),
                "report": report,
                "preflight": preflight,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", render_project_init_text(&plan, false));
            println!();
            println!("{}", render_overlay_text(&overlay));
            println!();
            println!("{}", suite_doctor::render_text(&report));
            println!();
            println!(
                "{}",
                project_discovery::render_session_preflight_text(&preflight)
            );
        }
    }
    if !report.passed() || preflight.exit_code != 0 {
        std::process::exit(1);
    }
}

// ── Local governance overlay (.git/info/exclude management) ─────────────────
//
// `ags init` defaults to a `local` overlay: the AGS governance files it writes
// into a repository are added to `.git/info/exclude` so they are git-ignored
// locally and never show up as committable changes. `--mode shared|tracked`
// opts into a committed overlay (no exclude). `--migrate-tracked-overlay`
// untracks already-tracked AGS-owned files via `git rm --cached` (keeping the
// working copy). Shared files the repository may own (AGENTS.md / CLAUDE.md /
// .gitignore) are never auto-untracked.

const OVERLAY_BLOCK_BEGIN: &str = "# >>> AGS local governance overlay (managed by `ags init`) >>>";
const OVERLAY_BLOCK_END: &str = "# <<< AGS local governance overlay (managed by `ags init`) <<<";

/// Shared, repo-owned append targets that AGS never auto-untracks.
const OVERLAY_SHARED_TARGETS: [&str; 3] = ["/AGENTS.md", "/CLAUDE.md", "/.gitignore"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OverlayMode {
    /// Default: AGS files are added to `.git/info/exclude` (local, uncommitted).
    Local,
    /// Opt-in: AGS files are left tracked/committed (shared with the repo).
    Shared,
}

impl OverlayMode {
    fn parse(value: &str) -> OverlayMode {
        match value {
            "shared" | "tracked" => OverlayMode::Shared,
            _ => OverlayMode::Local,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            OverlayMode::Local => "local",
            OverlayMode::Shared => "shared",
        }
    }
}

/// Repo-root-anchored gitignore entries for every AGS overlay file that lives
/// inside the target repository. Memory-capsule files (under `$HOME`) and any
/// path outside the target are skipped. Result is sorted and de-duplicated.
fn overlay_exclude_entries(target: &Path, files: &[InstallFile]) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for file in files {
        if let Ok(rel) = file.path.strip_prefix(target) {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if !rel.is_empty() {
                entries.push(format!("/{rel}"));
            }
        }
    }
    entries.sort();
    entries.dedup();
    entries
}

/// Overlay entries that AGS exclusively owns and may safely untrack. The shared
/// append targets are never auto-untracked because the repository may own them.
fn overlay_migratable_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| !OVERLAY_SHARED_TARGETS.contains(&e.as_str()))
        .cloned()
        .collect()
}

/// Result of merging the AGS-managed overlay block into a `.git/info/exclude`
/// body.
struct OverlayExcludeMerge {
    content: String,
    /// True when the existing body contained an unpaired/malformed AGS marker
    /// (a begin without a matching end, or a stray end). In that case the
    /// existing content is preserved verbatim and a fresh block is appended;
    /// no user lines are ever deleted.
    had_malformed_markers: bool,
}

/// Insert or replace the AGS-managed overlay block in an existing
/// `.git/info/exclude` body. Only a **well-formed** managed block — a `BEGIN`
/// line followed by a matching `END` line with no intervening `BEGIN` — is
/// stripped and replaced. Unpaired markers (a begin without an end, or a stray
/// end) are treated as ordinary content and preserved, so user ignore lines
/// after a truncated block are never silently dropped; a fresh block is
/// appended instead and `had_malformed_markers` is set. Idempotent:
/// re-running with the same entries yields byte-identical output, even when
/// stray markers remain.
fn merge_overlay_exclude(existing: &str, entries: &[String]) -> OverlayExcludeMerge {
    let lines: Vec<&str> = existing.lines().collect();
    let mut marker_depth = 0usize;
    let mut had_malformed_markers = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == OVERLAY_BLOCK_BEGIN {
            if marker_depth != 0 {
                had_malformed_markers = true;
                break;
            }
            marker_depth = 1;
        } else if trimmed == OVERLAY_BLOCK_END {
            if marker_depth == 0 {
                had_malformed_markers = true;
                break;
            }
            marker_depth = 0;
        }
    }
    if marker_depth != 0 {
        had_malformed_markers = true;
    }

    if had_malformed_markers {
        let mut content = existing.trim_end_matches('\n').to_string();
        if !entries.is_empty() && !ends_with_overlay_block(&content, entries) {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            push_overlay_block(&mut content, entries);
        } else if !content.is_empty() {
            content.push('\n');
        }
        return OverlayExcludeMerge {
            content,
            had_malformed_markers: true,
        };
    }

    let mut kept: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == OVERLAY_BLOCK_BEGIN {
            let mut j = i + 1;
            while j < lines.len() {
                let inner = lines[j].trim();
                if inner == OVERLAY_BLOCK_END {
                    break;
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        kept.push(lines[i]);
        i += 1;
    }
    while matches!(kept.last(), Some(l) if l.trim().is_empty()) {
        kept.pop();
    }

    let mut content = String::new();
    if !kept.is_empty() {
        content.push_str(&kept.join("\n"));
        content.push('\n');
    }
    if !entries.is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        push_overlay_block(&mut content, entries);
    }
    OverlayExcludeMerge {
        content,
        had_malformed_markers: false,
    }
}

fn push_overlay_block(content: &mut String, entries: &[String]) {
    content.push_str(OVERLAY_BLOCK_BEGIN);
    content.push('\n');
    for entry in entries {
        content.push_str(entry);
        content.push('\n');
    }
    content.push_str(OVERLAY_BLOCK_END);
    content.push('\n');
}

fn ends_with_overlay_block(content: &str, entries: &[String]) -> bool {
    let mut expected = String::new();
    push_overlay_block(&mut expected, entries);
    content.trim_end_matches('\n') == expected.trim_end_matches('\n')
        || content
            .trim_end_matches('\n')
            .ends_with(&format!("\n\n{}", expected.trim_end_matches('\n')))
}

fn git_command(target: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(target);
    cmd
}

fn git_is_repo(target: &Path) -> bool {
    git_command(target)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

fn git_info_exclude_path(target: &Path) -> Option<PathBuf> {
    let out = git_command(target)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(&raw);
    Some(if path.is_absolute() {
        path
    } else {
        target.join(path)
    })
}

fn git_tracked_set(target: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(out) = git_command(target).args(["ls-files"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let line = line.trim();
                if !line.is_empty() {
                    set.insert(line.to_string());
                }
            }
        }
    }
    set
}

fn git_rm_cached(target: &Path, rel: &str) -> Result<(), String> {
    let out = git_command(target)
        .args(["rm", "--cached", "--quiet", "--", rel])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

struct OverlayPlan {
    target: PathBuf,
    mode: OverlayMode,
    migrate: bool,
    is_git_repo: bool,
    exclude_path: Option<PathBuf>,
    entries: Vec<String>,
    tracked_migratable: Vec<String>,
    tracked_shared: Vec<String>,
    warnings: Vec<String>,
}

/// Resolve the local overlay plan for the given target and AGS install files.
/// Read-only: it queries git state but performs no writes.
fn compute_overlay_plan(
    target: &Path,
    files: &[InstallFile],
    mode: OverlayMode,
    migrate: bool,
) -> OverlayPlan {
    let entries = overlay_exclude_entries(target, files);
    let mut warnings = Vec::new();
    let is_git_repo = git_is_repo(target);

    if mode == OverlayMode::Shared {
        return OverlayPlan {
            target: target.to_path_buf(),
            mode,
            migrate,
            is_git_repo,
            exclude_path: None,
            entries,
            tracked_migratable: Vec::new(),
            tracked_shared: Vec::new(),
            warnings,
        };
    }

    if !is_git_repo {
        warnings.push(
            "target is not a git repository; cannot write a local overlay to .git/info/exclude. Run `git init` first or use --mode shared."
                .to_string(),
        );
        return OverlayPlan {
            target: target.to_path_buf(),
            mode,
            migrate,
            is_git_repo,
            exclude_path: None,
            entries,
            tracked_migratable: Vec::new(),
            tracked_shared: Vec::new(),
            warnings,
        };
    }

    let exclude_path = git_info_exclude_path(target);
    let tracked = git_tracked_set(target);
    let tracked_migratable: Vec<String> = overlay_migratable_entries(&entries)
        .into_iter()
        .filter(|e| tracked.contains(e.trim_start_matches('/')))
        .collect();
    let tracked_shared: Vec<String> = entries
        .iter()
        .filter(|e| {
            OVERLAY_SHARED_TARGETS.contains(&e.as_str())
                && tracked.contains(e.trim_start_matches('/'))
        })
        .cloned()
        .collect();

    if !migrate && !tracked_migratable.is_empty() {
        warnings.push(format!(
            "{} AGS overlay file(s) are tracked by git and will stay visible until migrated. Re-run with `--migrate-tracked-overlay` to untrack them via `git rm --cached`.",
            tracked_migratable.len()
        ));
    }
    if !tracked_shared.is_empty() {
        warnings.push(format!(
            "{} shared file(s) ({}) are tracked; AGS appended its governance block and they will show as modifications. Local overlay never auto-untracks shared files.",
            tracked_shared.len(),
            tracked_shared.join(", ")
        ));
    }

    OverlayPlan {
        target: target.to_path_buf(),
        mode,
        migrate,
        is_git_repo,
        exclude_path,
        entries,
        tracked_migratable,
        tracked_shared,
        warnings,
    }
}

/// Apply the local overlay: migrate tracked AGS-owned files (when requested),
/// then write the managed block into `.git/info/exclude`. Returns findings.
fn apply_overlay(plan: &OverlayPlan) -> Vec<suite_doctor::Finding> {
    use suite_doctor::Finding;
    let mut findings = Vec::new();

    if plan.mode == OverlayMode::Shared {
        findings.push(Finding::info(
            "overlay-mode",
            "overlay mode: shared — AGS governance files are left tracked/committed",
        ));
        return findings;
    }

    if !plan.is_git_repo {
        for warning in &plan.warnings {
            findings.push(Finding::warn(
                "overlay-no-git",
                warning.clone(),
                "AGS local overlay not applied",
            ));
        }
        return findings;
    }

    if plan.migrate {
        for entry in &plan.tracked_migratable {
            let rel = entry.trim_start_matches('/');
            match git_rm_cached(&plan.target, rel) {
                Ok(()) => findings.push(Finding::pass(
                    format!("overlay-migrate-{}", sanitize_name(rel)),
                    format!("untracked via git rm --cached (working copy kept): {rel}"),
                )),
                Err(e) => findings.push(Finding::fail(
                    format!("overlay-migrate-{}", sanitize_name(rel)),
                    format!("failed to untrack {rel}"),
                    e,
                )),
            }
        }
    }

    let Some(exclude_path) = &plan.exclude_path else {
        findings.push(Finding::warn(
            "overlay-exclude",
            "could not resolve .git/info/exclude path",
            "AGS local overlay not written",
        ));
        return findings;
    };

    let existing = std::fs::read_to_string(exclude_path).unwrap_or_default();
    let merge = merge_overlay_exclude(&existing, &plan.entries);
    if merge.had_malformed_markers {
        findings.push(Finding::warn(
            "overlay-exclude-malformed",
            format!(
                "unpaired AGS overlay marker(s) in {}; preserved existing content and appended a fresh managed block (no user lines deleted) — remove stray markers manually",
                exclude_path.display()
            ),
            "malformed managed block detected",
        ));
    }
    if merge.content == existing {
        findings.push(Finding::pass(
            "overlay-exclude",
            format!(
                "unchanged: {} ({} overlay entries)",
                exclude_path.display(),
                plan.entries.len()
            ),
        ));
    } else {
        if let Some(parent) = exclude_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(exclude_path, &merge.content) {
            Ok(()) => findings.push(Finding::pass(
                "overlay-exclude",
                format!(
                    "wrote {} overlay entries to {}",
                    plan.entries.len(),
                    exclude_path.display()
                ),
            )),
            Err(e) => findings.push(Finding::fail(
                "overlay-exclude",
                format!("failed to write {}", exclude_path.display()),
                e.to_string(),
            )),
        }
    }

    for warning in &plan.warnings {
        findings.push(Finding::warn(
            "overlay-note",
            warning.clone(),
            "review overlay state",
        ));
    }
    findings
}

fn render_overlay_text(plan: &OverlayPlan) -> String {
    let mut lines = vec![
        "Overlay:".to_string(),
        format!("  Mode:    {}", plan.mode.as_str()),
        format!("  Git:     {}", if plan.is_git_repo { "yes" } else { "no" }),
    ];
    if plan.mode == OverlayMode::Local && plan.is_git_repo {
        if let Some(path) = &plan.exclude_path {
            lines.push(format!("  Exclude: {}", path.display()));
        }
        lines.push(format!(
            "  Entries: {} overlay path(s) git-ignored locally",
            plan.entries.len()
        ));
        for entry in &plan.entries {
            lines.push(format!("    - {entry}"));
        }
        if plan.migrate && !plan.tracked_migratable.is_empty() {
            lines.push(format!(
                "  Migrate: {} tracked AGS file(s) via git rm --cached",
                plan.tracked_migratable.len()
            ));
            for entry in &plan.tracked_migratable {
                lines.push(format!("    - {entry}"));
            }
        }
    } else if plan.mode == OverlayMode::Shared {
        lines.push("  AGS governance files are tracked/committed (shared).".to_string());
    }
    for warning in &plan.warnings {
        lines.push(format!("  ! {warning}"));
    }
    lines.join("\n")
}

fn overlay_json(plan: &OverlayPlan) -> serde_json::Value {
    serde_json::json!({
        "mode": plan.mode.as_str(),
        "is_git_repo": plan.is_git_repo,
        "migrate": plan.migrate,
        "exclude_path": plan.exclude_path.as_ref().map(|p| p.to_string_lossy()),
        "entries": plan.entries,
        "tracked_migratable": plan.tracked_migratable,
        "tracked_shared": plan.tracked_shared,
        "warnings": plan.warnings,
    })
}

fn install_file_status(file: &InstallFile) -> &'static str {
    match std::fs::read(&file.path) {
        Ok(existing) if existing == file.content.as_bytes() => "unchanged",
        Ok(_) => "would-replace",
        Err(_) => "would-create",
    }
}

fn render_private_plan_json(plan: &PrivateInstallPlan) -> String {
    let files: Vec<_> = plan
        .files
        .iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path.to_string_lossy(),
                "description": file.description,
                "mode": file.mode.map(|m| format!("{m:o}")),
                "status": install_file_status(file),
            })
        })
        .collect();
    let cleanup_dirs: Vec<_> = plan
        .cleanup_dirs
        .iter()
        .map(|path| {
            serde_json::json!({
                "path": path.to_string_lossy(),
                "status": if path.exists() { "would-remove" } else { "absent" },
            })
        })
        .collect();

    let output = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": plan.profile,
        "source_root": plan.source_root.to_string_lossy(),
        "target": plan.target.to_string_lossy(),
        "with_evomap": plan.with_evomap,
        "write_mode": "plan-only",
        "files": files,
        "cleanup_dirs": cleanup_dirs,
        "host_config_policy": "MCP snippets are generated only; Claude Code /ags command and Codex AGS command skills are installed on apply",
        "evomap_boundary": "parallel_peer_not_brokered",
    });
    serde_json::to_string_pretty(&output).unwrap_or_default()
}

fn render_private_plan_text(plan: &PrivateInstallPlan) -> String {
    let mut lines = vec![
        format!(
            "AGS Private Runtime Install Plan {}",
            PRIVATE_INSTALL_SCHEMA
        ),
        format!("Profile: {}", plan.profile),
        format!("Source:  {}", plan.source_root.display()),
        format!("Target:  {}", plan.target.display()),
        format!(
            "EvoMap:  {}",
            if plan.with_evomap {
                "included (managed-device runtime)"
            } else {
                "not included (add --with-evomap to include managed-device runtime)"
            }
        ),
        "Mode:    plan-only".to_string(),
        String::new(),
        "Files:".to_string(),
    ];
    for (i, file) in plan.files.iter().enumerate() {
        let mode = file
            .mode
            .map(|m| format!(" mode={m:o}"))
            .unwrap_or_default();
        lines.push(format!(
            "  {}. [{}{}] {} — {}",
            i + 1,
            install_file_status(file),
            mode,
            file.path.display(),
            file.description
        ));
    }
    if !plan.cleanup_dirs.is_empty() {
        lines.push(String::new());
        lines.push("Cleanup:".to_string());
        for (i, dir) in plan.cleanup_dirs.iter().enumerate() {
            let status = if dir.exists() {
                "would-remove"
            } else {
                "absent"
            };
            lines.push(format!("  {}. [{}] {}", i + 1, status, dir.display()));
        }
    }
    lines.push(String::new());
    lines.push(
        "Host config policy: MCP snippets only; Claude Code /ags command and Codex AGS command skills are installed on apply."
            .to_string(),
    );
    lines.push("EvoMap boundary: parallel peer, not proxied by AGS MCP.".to_string());
    if plan.with_evomap {
        lines.push("Apply with: ags setup --with-evomap --yes".to_string());
        lines.push(
            "One-command Claude Code initialization: /ags setup (runs setup with Claude MCP registration)"
                .to_string(),
        );
    } else {
        lines.push("Apply with: ags setup --yes".to_string());
        lines.push("Optional managed-device EvoMap runtime: ags setup --with-evomap".to_string());
    }
    lines.join("\n")
}

fn cleanup_install_dir(path: &Path) -> suite_doctor::Finding {
    if !path.exists() {
        return suite_doctor::Finding::pass(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("absent: {}", path.display()),
        );
    }
    match std::fs::remove_dir_all(path) {
        Ok(()) => suite_doctor::Finding::pass(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("removed: {}", path.display()),
        ),
        Err(e) => suite_doctor::Finding::fail(
            format!("cleanup-{}", sanitize_name(&path.to_string_lossy())),
            format!("remove failed: {}", path.display()),
            e.to_string(),
        ),
    }
}

fn cmd_private_plan(profile: &str, target: Option<PathBuf>, format: &str, with_evomap: bool) {
    if profile != "private" {
        eprintln!("ags plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    let plan = private_install_plan(&source_root, &target, with_evomap);
    match format {
        "json" => println!("{}", render_private_plan_json(&plan)),
        _ => println!("{}", render_private_plan_text(&plan)),
    }
}

fn write_install_file(file: &InstallFile, force: bool, backup_stamp: u64) -> suite_doctor::Finding {
    if let Some(parent) = file.path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return suite_doctor::Finding::fail(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("cannot create directory {}", parent.display()),
                e.to_string(),
            );
        }
    }

    match std::fs::read(&file.path) {
        Ok(existing) if existing == file.content.as_bytes() => {
            return suite_doctor::Finding::pass(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("unchanged: {}", file.path.display()),
            );
        }
        Ok(_) if !force => {
            return suite_doctor::Finding::fail(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("exists with different content: {}", file.path.display()),
                "Review `ags setup`, then rerun setup with --force --yes if replacement is intended.",
            );
        }
        Ok(_) => {
            let backup = file.path.with_extension(format!(
                "{}.bak.{backup_stamp}",
                file.path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
            ));
            if let Err(e) = std::fs::copy(&file.path, &backup) {
                return suite_doctor::Finding::fail(
                    format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                    format!("backup failed for {}", file.path.display()),
                    e.to_string(),
                );
            }
        }
        Err(_) => {}
    }

    if let Err(e) = std::fs::write(&file.path, &file.content) {
        return suite_doctor::Finding::fail(
            format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
            format!("write failed: {}", file.path.display()),
            e.to_string(),
        );
    }

    #[cfg(unix)]
    if let Some(mode) = file.mode {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&file.path) {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(&file.path, perms);
        }
    }

    suite_doctor::Finding::pass(
        format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
        format!("written: {}", file.path.display()),
    )
}

fn run_claude_mcp_command(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("claude")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined.trim().to_string())
    } else {
        Err(combined.trim().to_string())
    }
}

fn register_claude_mcp_server(
    report: &mut suite_doctor::HealthReport,
    server: &str,
    command: String,
    args: &[&str],
) {
    let remove_args = vec![
        "mcp".to_string(),
        "remove".to_string(),
        server.to_string(),
        "-s".to_string(),
        "user".to_string(),
    ];
    let _ = run_claude_mcp_command(&remove_args);

    let mut add_args = vec![
        "mcp".to_string(),
        "add".to_string(),
        "-s".to_string(),
        "user".to_string(),
        server.to_string(),
        "--".to_string(),
        command.clone(),
    ];
    add_args.extend(args.iter().map(|arg| (*arg).to_string()));

    match run_claude_mcp_command(&add_args) {
        Ok(output) => {
            let mut finding = suite_doctor::Finding::pass(
                format!("install-claude-mcp-register-{server}"),
                format!("Claude Code MCP registered {server}: {command}"),
            );
            finding.detail = if output.trim().is_empty() {
                None
            } else {
                Some(output)
            };
            report.add(finding);
        }
        Err(e) => report.add(suite_doctor::Finding::fail(
            format!("install-claude-mcp-register-{server}"),
            format!("failed to register Claude Code MCP {server}"),
            e,
        )),
    }
}

fn add_claude_registration_checks(
    report: &mut suite_doctor::HealthReport,
    target: &Path,
    with_evomap: bool,
) {
    match command_in_path("claude") {
        Ok(path) => report.add(suite_doctor::Finding::pass(
            "install-claude-code-cli",
            format!("Claude Code CLI available at {path}"),
        )),
        Err(e) => {
            report.add(suite_doctor::Finding::fail(
                "install-claude-code-cli",
                "Claude Code CLI is required for --register-claude",
                e,
            ));
            return;
        }
    }

    match command_in_path("ags") {
        Ok(ags_path) => register_claude_mcp_server(
            report,
            "ags",
            ags_path,
            &["mcp", "serve", "--transport", "stdio"],
        ),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-ags",
            "cannot register AGS MCP because `ags` is not on PATH",
            e,
        )),
    }

    match command_in_path("codegraph") {
        Ok(codegraph_path) => {
            register_claude_mcp_server(report, "codegraph", codegraph_path, &["serve", "--mcp"])
        }
        Err(e) => report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-codegraph",
            "cannot register codegraph MCP because `codegraph` is not on PATH",
            format!("install codegraph first, then rerun setup. {e}"),
        )),
    }

    if !with_evomap {
        return;
    }

    match command_in_path("gep-mcp-server") {
        Ok(gep_path) => register_claude_mcp_server(report, "gep", gep_path, &[]),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-gep",
            "cannot register GEP MCP because `gep-mcp-server` is not on PATH",
            format!("install @evomap/gep-mcp-server first, then rerun setup. {e}"),
        )),
    }

    let evolver_proxy = target.join("bin/evolver-proxy-mcp");
    if evolver_proxy.exists() {
        register_claude_mcp_server(
            report,
            "evolver_proxy",
            evolver_proxy.to_string_lossy().to_string(),
            &[],
        );
    } else {
        report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-evolver_proxy",
            "cannot register evolver_proxy because AGS managed wrapper is missing",
            format!("missing: {}", evolver_proxy.display()),
        ));
    }
}

fn cmd_private_apply(
    profile: &str,
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    format: &str,
    with_evomap: bool,
    register_claude: bool,
) {
    if profile != "private" {
        eprintln!("ags apply: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    if !yes {
        eprintln!("ags setup: --yes is required for write mode.");
        eprintln!("Review `ags setup` first.");
        std::process::exit(2);
    }

    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    guard_writable_target("ags setup", &target);
    let plan = private_install_plan(&source_root, &target, with_evomap);
    let plan_text_before_apply = render_private_plan_text(&plan);
    let backup_stamp = unix_timestamp();
    let mut report = suite_doctor::HealthReport::new("private-install-apply");

    for file in &plan.files {
        report.add(write_install_file(file, force, backup_stamp));
    }
    for dir in &plan.cleanup_dirs {
        report.add(cleanup_install_dir(dir));
    }
    if register_claude {
        add_claude_registration_checks(&mut report, &target, with_evomap);
    }

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PRIVATE_INSTALL_SCHEMA,
                "profile": profile,
                "target": target.to_string_lossy(),
                "with_evomap": with_evomap,
                "register_claude": register_claude,
                "force": force,
                "report": report,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", plan_text_before_apply);
            println!();
            println!("{}", suite_doctor::render_text(&report));
        }
    }
    std::process::exit(report.exit_code());
}

fn cmd_setup(
    target: Option<PathBuf>,
    with_evomap: bool,
    yes: bool,
    force: bool,
    register_claude: bool,
    dry_run: bool,
    format: &str,
) {
    if yes && !dry_run {
        cmd_private_apply(
            "private",
            target.clone(),
            true,
            force,
            format,
            with_evomap,
            register_claude,
        );
    }
    cmd_private_plan("private", target, format, with_evomap);
}

fn json_file_ok(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str::<serde_json::Value>(&text)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn text_file_contains_no_secret_markers(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if has_token_like_secret(&text, "Bearer ", 20) {
        return Err("contains token-like Bearer secret".to_string());
    }
    if has_token_like_secret(&text, "sk-", 20) {
        return Err("contains token-like sk secret".to_string());
    }
    Ok(())
}

fn has_token_like_secret(text: &str, prefix: &str, min_tail: usize) -> bool {
    let mut start = 0;
    while let Some(offset) = text[start..].find(prefix) {
        let tail_start = start + offset + prefix.len();
        let tail = &text[tail_start..];
        let token_len = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if token_len >= min_tail {
            return true;
        }
        start = tail_start;
    }
    false
}

fn mcp_smoke_current_exe() -> Result<(), String> {
    use std::io::Write;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .args(["mcp", "serve", "--transport", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ags-install-verify\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"ags_solution_check\",\"arguments\":{\"summary\":\"before preflight\"}}}\n"
    );
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains("\"id\":1") || !stdout.contains("\"result\"") {
        return Err("initialize response missing".to_string());
    }
    if !stdout.contains("\"id\":2") || !stdout.contains("AGS Initialization Gate") {
        return Err("preflight gate error response missing".to_string());
    }
    Ok(())
}

fn command_in_path(command: &str) -> Result<String, String> {
    // Cross-platform PATH lookup (replaces shelling out to `which`, which is
    // absent on native Windows). On Windows this also honours `%PATHEXT%`.
    match ags_platform::find_in_path(command) {
        Some(path) => Ok(path.display().to_string()),
        None => Err(format!("{command} not found in PATH")),
    }
}

fn claude_mcp_list_line(server: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "list"])
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{server}:")))
            .map(|line| line.trim().to_string()))
    } else {
        Err(combined.trim().to_string())
    }
}

fn claude_mcp_get(server: &str) -> Result<String, String> {
    let output = std::process::Command::new("claude")
        .args(["mcp", "get", server])
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined)
    } else {
        Err(combined.trim().to_string())
    }
}

fn shell_syntax_ok(path: &Path) -> Result<(), String> {
    // `bash -n` validates a Unix shell script; on native Windows bash may be
    // absent. Skip (treat as ok) rather than hard-failing — the checked
    // artifact is a Unix shell script that only runs where bash exists.
    if !ags_platform::is_on_path("bash") {
        return Ok(());
    }
    let output = std::process::Command::new("bash")
        .arg("-n")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn add_evomap_advisory_checks(
    report: &mut suite_doctor::HealthReport,
    target: &Path,
    required: bool,
) {
    let evomap_files = [
        "mcp/gep.mcp.json",
        "hosts/claude-code.evomap-mcp.snippet.json",
        "bin/evolver-proxy-mcp",
        "evomap/README.md",
    ];

    for rel in evomap_files {
        let path = target.join(rel);
        if path.exists() {
            report.add(suite_doctor::Finding::pass(
                format!("private-install-evomap-present-{}", sanitize_name(rel)),
                format!("present: {rel}"),
            ));
        } else if required {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-evomap-present-{}", sanitize_name(rel)),
                format!("missing: {rel}"),
                "rerun `ags setup --with-evomap --yes`",
            ));
        } else {
            report.add(suite_doctor::Finding::warn(
                format!("private-install-evomap-present-{}", sanitize_name(rel)),
                format!("optional EvoMap planner recall snippet not installed: {rel}"),
                "run `ags setup --with-evomap` and apply after review with --yes",
            ));
        }
    }

    let proxy_launcher = target.join("bin/evolver-proxy-mcp");
    if proxy_launcher.exists() {
        match shell_syntax_ok(&proxy_launcher) {
            Ok(()) => report.add(suite_doctor::Finding::pass(
                "private-install-evomap-proxy-wrapper-syntax",
                "evolver-proxy-mcp bash syntax OK",
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-evomap-proxy-wrapper-syntax",
                "evolver-proxy-mcp bash syntax failed",
                e,
            )),
        }
        match text_file_contains_no_secret_markers(&proxy_launcher) {
            Ok(()) => report.add(suite_doctor::Finding::pass(
                "private-install-evomap-proxy-wrapper-secret-scan",
                "secret marker scan OK: bin/evolver-proxy-mcp",
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-evomap-proxy-wrapper-secret-scan",
                "secret marker scan failed: bin/evolver-proxy-mcp",
                e,
            )),
        }
    } else if required {
        report.add(suite_doctor::Finding::fail(
            "private-install-evomap-proxy-wrapper-syntax",
            "evolver-proxy-mcp wrapper missing",
            "rerun `ags setup --with-evomap --yes`",
        ));
        report.add(suite_doctor::Finding::fail(
            "private-install-evomap-proxy-wrapper-secret-scan",
            "evolver-proxy-mcp wrapper missing",
            "rerun `ags setup --with-evomap --yes`",
        ));
    }

    for rel in [
        "mcp/gep.mcp.json",
        "hosts/claude-code.evomap-mcp.snippet.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match json_file_ok(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-evomap-json-{}", sanitize_name(rel)),
                    format!("valid JSON: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-evomap-json-{}", sanitize_name(rel)),
                    format!("invalid JSON: {rel}"),
                    e,
                )),
            }
            match text_file_contains_no_secret_markers(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-evomap-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan OK: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-evomap-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan failed: {rel}"),
                    e,
                )),
            }
        }
    }

    match command_in_path("gep-mcp-server") {
        Ok(path) => report.add(suite_doctor::Finding::pass(
            "private-install-evomap-gep-mcp-server",
            format!("gep-mcp-server available at {path}"),
        )),
        Err(e) if required => report.add(suite_doctor::Finding::fail(
            "private-install-evomap-gep-mcp-server",
            "gep-mcp-server is not available on PATH",
            format!("install @evomap/gep-mcp-server or provide a reviewed wrapper. {e}"),
        )),
        Err(e) => report.add(suite_doctor::Finding::warn(
            "private-install-evomap-gep-mcp-server",
            "optional gep-mcp-server is not available on PATH",
            format!("install before enabling EvoMap planner recall. {e}"),
        )),
    }

    for server in ["gep", "evolver_proxy"] {
        match claude_mcp_list_line(server) {
            Ok(Some(line)) if line.contains("Connected") => report.add(suite_doctor::Finding::pass(
                format!("private-install-claude-code-{server}-global"),
                format!("Claude Code global MCP includes connected {server}"),
            )),
            Ok(Some(line)) if required => report.add(suite_doctor::Finding::fail(
                format!("private-install-claude-code-{server}-global"),
                format!("Claude Code global MCP {server} is configured but not connected"),
                line,
            )),
            Ok(Some(line)) => report.add(suite_doctor::Finding::warn(
                format!("private-install-claude-code-{server}-global"),
                format!("Claude Code global MCP optional {server} is configured but not connected"),
                line,
            )),
            Ok(None) if required => report.add(suite_doctor::Finding::fail(
                format!("private-install-claude-code-{server}-global"),
                format!("Claude Code global MCP does not include {server}"),
                format!(
                    "run `claude mcp add -s user {server} -- <reviewed-command>` after reviewing the snippet"
                ),
            )),
            Ok(None) => report.add(suite_doctor::Finding::warn(
                format!("private-install-claude-code-{server}-global"),
                format!("Claude Code global MCP does not include optional {server}"),
                format!(
                    "install {server} only if planner recall should be globally available"
                ),
            )),
            Err(e) if required => report.add(suite_doctor::Finding::fail(
                format!("private-install-claude-code-{server}-global"),
                format!("cannot verify Claude Code global MCP {server} entry"),
                e,
            )),
            Err(e) => report.add(suite_doctor::Finding::warn(
                format!("private-install-claude-code-{server}-global"),
                format!("cannot verify optional Claude Code global MCP {server} entry"),
                e,
            )),
        }
    }

    let expected_proxy_command = target.join("bin/evolver-proxy-mcp");
    let expected_proxy_command = expected_proxy_command.to_string_lossy();
    match claude_mcp_get("evolver_proxy") {
        Ok(detail) if detail.contains(expected_proxy_command.as_ref()) => {
            report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-evolver_proxy-command",
                "Claude Code evolver_proxy uses AGS managed wrapper",
            ));
        }
        Ok(detail) if required => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-evolver_proxy-command",
            "Claude Code evolver_proxy does not use AGS managed wrapper",
            format!("expected command: {expected_proxy_command}\n{detail}"),
        )),
        Ok(detail) => report.add(suite_doctor::Finding::warn(
            "private-install-claude-code-evolver_proxy-command",
            "optional Claude Code evolver_proxy does not use AGS managed wrapper",
            detail,
        )),
        Err(e) if required => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-evolver_proxy-command",
            "cannot inspect Claude Code evolver_proxy command",
            e,
        )),
        Err(e) => report.add(suite_doctor::Finding::warn(
            "private-install-claude-code-evolver_proxy-command",
            "cannot inspect optional Claude Code evolver_proxy command",
            e,
        )),
    }
}

fn add_codegraph_claude_checks(report: &mut suite_doctor::HealthReport) {
    match claude_mcp_list_line("codegraph") {
        Ok(Some(line)) if line.contains("Connected") => report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP includes connected codegraph",
        )),
        Ok(Some(line)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP codegraph is configured but not connected",
            line,
        )),
        Ok(None) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "Claude Code global MCP does not include codegraph",
            "run `claude mcp add -s user codegraph -- codegraph serve --mcp`",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-global",
            "cannot verify Claude Code global MCP codegraph entry",
            e,
        )),
    }

    match claude_mcp_get("codegraph") {
        Ok(detail) if detail.contains("codegraph") && detail.contains("serve --mcp") => {
            report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-codegraph-command",
                "Claude Code codegraph MCP uses `codegraph serve --mcp`",
            ));
        }
        Ok(detail) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-command",
            "Claude Code codegraph MCP does not use `codegraph serve --mcp`",
            detail,
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-codegraph-command",
            "cannot inspect Claude Code codegraph MCP command",
            e,
        )),
    }

    match command_in_path("codegraph") {
        Ok(path) => report.add(suite_doctor::Finding::pass(
            "private-install-codegraph-cli",
            format!("codegraph CLI available at {path}"),
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-codegraph-cli",
            "codegraph CLI is not available on PATH",
            format!("install codegraph before relying on code intelligence. {e}"),
        )),
    }
}

fn cmd_private_verify(profile: &str, target: Option<PathBuf>, format: &str, with_evomap: bool) {
    if profile != "private" {
        eprintln!("ags verify: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let mut report = suite_doctor::HealthReport::new("private-install-verify");

    let required = [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
        "bin/ags-mcp-stdio.sh",
    ];

    for rel in required {
        let path = target.join(rel);
        if path.exists() {
            report.add(suite_doctor::Finding::pass(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("present: {rel}"),
            ));
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-present-{}", sanitize_name(rel)),
                format!("missing: {rel}"),
                path.display().to_string(),
            ));
        }
    }

    let claude_command_path = claude_ags_command_path();
    if claude_command_path.exists() {
        report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-slash-command-present",
            format!("present: {}", claude_command_path.display()),
        ));
        match std::fs::read_to_string(&claude_command_path) {
            Ok(content) if content.contains("ags_preflight") && content.contains(AGS_VERSION) => {
                report.add(suite_doctor::Finding::pass(
                    "private-install-claude-code-slash-command-content",
                    "Claude Code /ags command references AGS preflight and current version",
                ));
            }
            Ok(_) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-content",
                "Claude Code /ags command content is stale",
                format!(
                    "expected ags_preflight and version {AGS_VERSION} in {}",
                    claude_command_path.display()
                ),
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-content",
                "cannot read Claude Code /ags command",
                e.to_string(),
            )),
        }
        match text_file_contains_no_secret_markers(&claude_command_path) {
            Ok(()) => report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan OK: Claude Code /ags command",
            )),
            Err(e) => report.add(suite_doctor::Finding::fail(
                "private-install-claude-code-slash-command-secret-scan",
                "secret marker scan failed: Claude Code /ags command",
                e,
            )),
        }
    } else {
        report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-slash-command-present",
            "missing Claude Code /ags command",
            format!(
                "rerun `ags setup --yes` to create {}",
                claude_command_path.display()
            ),
        ));
    }

    for retired_dir in retired_codex_ags_skill_dirs() {
        let check_suffix = sanitize_name(&retired_dir.to_string_lossy());
        if retired_dir.exists() {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                "retired Codex AGS visible skill still exists",
                format!(
                    "rerun `ags setup --yes --force` to remove {}",
                    retired_dir.display()
                ),
            ));
        } else {
            report.add(suite_doctor::Finding::pass(
                format!("private-install-retired-codex-skill-{check_suffix}"),
                format!(
                    "retired Codex AGS visible skill absent: {}",
                    retired_dir.display()
                ),
            ));
        }
    }

    for (name, display_name, _, _, summary) in codex_ags_command_skill_specs() {
        let skill_path = codex_ags_named_skill_path(name);
        let check_suffix = sanitize_name(name);
        if skill_path.exists() {
            match std::fs::read_to_string(&skill_path) {
                Ok(content)
                    if content.contains(&format!("name: \"{name}\""))
                        && content.contains("ags session preflight --for codex")
                        && content.contains(AGS_VERSION) =>
                {
                    report.add(suite_doctor::Finding::pass(
                        format!("private-install-codex-command-skill-{check_suffix}"),
                        format!("Codex command skill present: {name}"),
                    ));
                }
                Ok(_) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("Codex command skill content is stale: {name}"),
                    format!("expected {display_name}, {summary}, and version {AGS_VERSION}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-{check_suffix}"),
                    format!("cannot read Codex command skill: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-codex-command-skill-{check_suffix}"),
                format!("missing Codex command skill: {name}"),
                skill_path.display().to_string(),
            ));
        }

        let metadata_path = codex_ags_named_skill_agent_metadata_path(name);
        if metadata_path.exists() {
            match std::fs::read_to_string(&metadata_path) {
                Ok(content) if content.contains(&format!("display_name: \"{display_name}\"")) => {
                    report.add(suite_doctor::Finding::pass(
                        format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                        format!("Codex command skill metadata present: {name}"),
                    ));
                }
                Ok(_) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("Codex command skill metadata is stale: {name}"),
                    metadata_path.display().to_string(),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                    format!("cannot read Codex command skill metadata: {name}"),
                    e.to_string(),
                )),
            }
        } else {
            report.add(suite_doctor::Finding::fail(
                format!("private-install-codex-command-skill-metadata-{check_suffix}"),
                format!("missing Codex command skill metadata: {name}"),
                metadata_path.display().to_string(),
            ));
        }
    }

    match claude_mcp_list_line("ags") {
        Ok(Some(line)) if line.contains("Connected") => report.add(suite_doctor::Finding::pass(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP includes connected ags",
        )),
        Ok(Some(line)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP ags is configured but not connected",
            line,
        )),
        Ok(None) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "Claude Code global MCP does not include ags",
            "run `/ags setup` or `ags setup --yes --register-claude`",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-global",
            "cannot verify Claude Code global MCP ags entry",
            e,
        )),
    }

    match (claude_mcp_get("ags"), command_in_path("ags")) {
        (Ok(detail), Ok(ags_path)) if detail.contains(&ags_path) => {
            report.add(suite_doctor::Finding::pass(
                "private-install-claude-code-ags-command",
                "Claude Code ags MCP uses installed AGS binary",
            ));
        }
        (Ok(detail), Ok(ags_path)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "Claude Code ags MCP does not use the installed AGS binary",
            format!("expected command: {ags_path}\n{detail}"),
        )),
        (Ok(detail), Err(e)) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "cannot confirm installed AGS binary path",
            format!("{e}\n{detail}"),
        )),
        (Err(e), _) => report.add(suite_doctor::Finding::fail(
            "private-install-claude-code-ags-command",
            "cannot inspect Claude Code ags MCP command",
            e,
        )),
    }

    add_codegraph_claude_checks(&mut report);

    for rel in [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "hooks/codex-planner-recall.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match json_file_ok(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-json-{}", sanitize_name(rel)),
                    format!("valid JSON: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-json-{}", sanitize_name(rel)),
                    format!("invalid JSON: {rel}"),
                    e,
                )),
            }
        }
    }

    for rel in [
        "install-manifest.json",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
    ] {
        let path = target.join(rel);
        if path.exists() {
            match text_file_contains_no_secret_markers(&path) {
                Ok(()) => report.add(suite_doctor::Finding::pass(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan OK: {rel}"),
                )),
                Err(e) => report.add(suite_doctor::Finding::fail(
                    format!("private-install-secret-scan-{}", sanitize_name(rel)),
                    format!("secret marker scan failed: {rel}"),
                    e,
                )),
            }
        }
    }

    match std::process::Command::new("node")
        .arg("--check")
        .arg(target.join("hooks/claude-code-executor-stop.js"))
        .output()
    {
        Ok(output) if output.status.success() => report.add(suite_doctor::Finding::pass(
            "private-install-node-check",
            "node --check hook OK",
        )),
        Ok(output) => report.add(suite_doctor::Finding::fail(
            "private-install-node-check",
            "node --check hook failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Err(e) => report.add(suite_doctor::Finding::warn(
            "private-install-node-check",
            "node unavailable; skipped hook syntax check",
            e.to_string(),
        )),
    }

    match mcp_smoke_current_exe() {
        Ok(()) => report.add(suite_doctor::Finding::pass(
            "private-install-mcp-smoke",
            "ags mcp serve stdio smoke OK",
        )),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "private-install-mcp-smoke",
            "ags mcp serve stdio smoke failed",
            e,
        )),
    }

    add_evomap_advisory_checks(&mut report, &target, with_evomap);

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PRIVATE_INSTALL_SCHEMA,
                "profile": profile,
                "target": target.to_string_lossy(),
                "with_evomap": with_evomap,
                "report": report,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => println!("{}", suite_doctor::render_text(&report)),
    }
    std::process::exit(report.exit_code());
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

/// Shared dispatch: `task compile` (M4)
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
        eprintln!(
            "  The user must explicitly issue a task-card instruction before an executable card can be generated."
        );
        eprintln!("  Use --task-card-requested after receiving: \"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.");
        std::process::exit(1);
    }

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    // Read input
    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    // Determine project root (current directory)
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Compile
    let (compiled_card, report) =
        task_compiler::compile(&content, &project_root, check_only, task_card_requested);

    // Validate the compiled card using the canonical validator
    let (validation_passed, validation_errors) = if !report.missing_slots.is_empty() {
        // Can't validate — missing slots
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

    // Build final report with actual validation results
    // Preserve gate fields from the compiler; override validation from the
    // canonical validator (which only runs meaningfully when executable_allowed).
    let final_report = task_compiler::CompileReport {
        schema_version: report.schema_version,
        compiled_task_card: report.compiled_task_card,
        slot_sources: report.slot_sources,
        missing_slots: report.missing_slots,
        assumptions: report.assumptions,
        validation_passed: if report.executable_allowed {
            validation_passed
        } else {
            report.validation_passed
        },
        validation_errors: if report.executable_allowed {
            validation_errors
        } else {
            report.validation_errors
        },
        check_only,
        task_card_requested: report.task_card_requested,
        executable_allowed: report.executable_allowed,
        block_reason: report.block_reason,
    };

    // check_only mode is inherently diagnostic — succeed if slots filled
    // regular mode requires executable_allowed AND validation_passed
    let success = if final_report.check_only {
        final_report.missing_slots.is_empty()
    } else {
        final_report.executable_allowed && final_report.validation_passed
    };

    // Card output is intended for direct piping into `ags task validate -`.
    // Never write a partial or invalid card to stdout.
    if output == "card" && !success {
        if !final_report.missing_slots.is_empty() {
            eprintln!(
                "{}: COMPILATION INCOMPLETE — {} missing slot(s)",
                display_path,
                final_report.missing_slots.len()
            );
            for slot in &final_report.missing_slots {
                eprintln!("  - {}", slot);
            }
        } else {
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &final_report.validation_errors {
                eprintln!("  - {}", err);
            }
        }
        std::process::exit(1);
    }

    // Output
    if output == "card" {
        // Plain card output — directly pipeable to `ags task validate -`
        match format {
            "json" => {
                // JSON card-only: wrap in a minimal object for machine consumers
                let card_json = serde_json::json!({
                    "compiled_task_card": final_report.compiled_task_card,
                });
                if let Ok(json) = serde_json::to_string_pretty(&card_json) {
                    println!("{}", json);
                }
            }
            _ => {
                // Plain text card output — first line is ## 任务卡
                print!("{}", task_compiler::render_card_text(&final_report));
            }
        }
    } else {
        // Full report output
        match format {
            "json" => {
                println!("{}", task_compiler::render_report_json(&final_report));
            }
            _ => {
                println!("{}", task_compiler::render_report_text(&final_report));
            }
        }
    }

    // Exit code
    if success {
        // Success — exit 0
    } else if !final_report.missing_slots.is_empty() {
        eprintln!(
            "{}: COMPILATION INCOMPLETE — {} missing slot(s)",
            display_path,
            final_report.missing_slots.len()
        );
        for slot in &final_report.missing_slots {
            eprintln!("  - {}", slot);
        }
        std::process::exit(1);
    } else {
        eprintln!("{}: VALIDATION FAILED", display_path);
        for err in &final_report.validation_errors {
            eprintln!("  - {}", err);
        }
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

    // Read input (file or stdin)
    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
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

/// Shared helper: read a task card (file or stdin) and validate+parse it.
/// Returns (content, parsed_fields, display_path) or exits on failure.
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
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
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

/// Shared dispatch: `gate check` — always outputs structured JSON even on
/// validation failure (decision=stop with error details).
fn cmd_gate_check(path: &str, format: &str, approve_writes: bool) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            let err_output = execution_policy::gate_check_failed(
                "read_error",
                vec![format!("Failed to read stdin: {}", e)],
            );
            output_gate_result(&err_output, &display_path, format);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                let err_output = execution_policy::gate_check_failed(
                    "read_error",
                    vec![format!("Failed to read {}: {}", display_path, e)],
                );
                output_gate_result(&err_output, &display_path, format);
                std::process::exit(1);
            }
        }
    };

    // Validate
    let card = match task_card_validator::parse_validated(&content) {
        Ok(c) => c,
        Err(errors) => {
            let err_output =
                execution_policy::gate_check_failed("validation_failed", errors.clone());
            output_gate_result(&err_output, &display_path, format);
            // Write validation errors to stderr for visibility
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
            std::process::exit(1);
        }
    };

    // Resolve and gate check
    let input = build_policy_input(&card.fields, approve_writes);
    let output = execution_policy::gate_check(&input);

    match format {
        "json" => match serde_json::to_string_pretty(&output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
                std::process::exit(1);
            }
        },
        _ => {
            println!("{}", format_gate_check_text(&output, &display_path));
        }
    }

    if output.decision == execution_policy::GateDecision::Stop {
        std::process::exit(1);
    }
}

/// Shared dispatch: `gate prompt-request` — deterministic entry intent gate.
fn cmd_gate_prompt_request(request_arg: &str, target: &Path, no_preflight: bool, format: &str) {
    use std::io::Read;

    let request = if request_arg == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("gate prompt-request: 读取失败 — {}", e);
            std::process::exit(1);
        }
        buf
    } else {
        request_arg.to_string()
    };

    let classification = prompt_request_classifier::classify(&request);

    // Value Route (效价比路由): minimal execution-path form for this request.
    // Advisory and deterministic. At the entry gate there is no task-card
    // instruction yet and no triviality assessment, so both context flags are
    // false. It shapes path form only — never task level, permission, or gates.
    let value_route = prompt_request_classifier::derive_value_route(&classification, false, false);

    // Fail-closed precondition: project must be AGS-healthy (preflight should not
    // stop) before we declare an executable routing requirement.
    let (preflight_ran, preflight_should_stop, preflight_status) = if no_preflight {
        (false, false, "skipped".to_string())
    } else {
        match project_discovery::AgentType::from_str("claude-code") {
            Ok(agent) => {
                let pf = project_discovery::run_session_preflight(target, &agent);
                (true, pf.should_stop, format!("{:?}", pf.overall_status))
            }
            Err(_) => (false, false, "skipped".to_string()),
        }
    };

    let (decision, block_reason): (&str, Option<&str>) = if preflight_should_stop {
        ("stop", Some("preflight_failed"))
    } else if classification.is_task_card_request {
        ("require_task_card", None)
    } else if classification.detected_advisory_intent && !classification.mutation_allowed {
        ("advisory_no_mutation", Some("advisory_intent_no_mutation"))
    } else {
        ("allow", None)
    };

    let next_step = match decision {
        "stop" => {
            "AGS preflight reports should_stop — resolve project/protocol health before generating any task card."
        }
        "require_task_card" => {
            "Task-card/prompt request detected. Route through AGS preflight → `ags task compile --task-card-requested` → `ags gate output`; the foreground answer MUST be a canonical `## 任务卡`."
        }
        "advisory_no_mutation" => {
            "Advisory/consultation intent detected. Host may perform preflight, read-only retrieval, diagnosis, solution formation, and risk explanation, but must NOT perform write-type tool calls, dependency installs, or implementation. Explicit execution authorization required to clear this block."
        }
        _ => "No task-card/prompt request detected. An ordinary prose answer is allowed.",
    };

    match format {
        "json" => {
            let mut out = serde_json::json!({
                "gate": "prompt_request",
                "decision": decision,
                "block_reason": block_reason,
                "is_task_card_request": classification.is_task_card_request,
                "detected_advisory_intent": classification.detected_advisory_intent,
                "mutation_allowed": classification.mutation_allowed,
                "classification": serde_json::to_value(&classification)
                    .unwrap_or(serde_json::Value::Null),
                "preflight": {
                    "ran": preflight_ran,
                    "should_stop": preflight_should_stop,
                    "status": preflight_status,
                },
                "value_route": serde_json::to_value(&value_route)
                    .unwrap_or(serde_json::Value::Null),
                "next_step": next_step,
            });
            if !classification.advisory_override_triggers.is_empty() {
                out["advisory_override_triggers"] =
                    serde_json::to_value(&classification.advisory_override_triggers)
                        .unwrap_or(serde_json::Value::Null);
            }
            match serde_json::to_string_pretty(&out) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Gate: prompt-request");
            println!("Decision: {}", decision);
            println!("Detected kind: {}", classification.kind.as_str());
            println!("Task-card request: {}", classification.is_task_card_request);
            if classification.detected_advisory_intent {
                println!(
                    "Advisory intent: detected (mutation_allowed={})",
                    classification.mutation_allowed
                );
            }
            if !classification.matched_triggers.is_empty() {
                println!(
                    "Matched triggers: {}",
                    classification.matched_triggers.join(", ")
                );
            }
            if !classification.advisory_override_triggers.is_empty() {
                println!(
                    "Override triggers: {}",
                    classification.advisory_override_triggers.join(", ")
                );
            }
            if preflight_ran {
                println!(
                    "Preflight: status={} should_stop={}",
                    preflight_status, preflight_should_stop
                );
            }
            if let Some(r) = block_reason {
                println!("Block reason: {}", r);
            }
            println!(
                "Value route: {} (user confirmation: {})",
                value_route.recommended_path.as_str(),
                if value_route.requires_user_confirmation {
                    "required"
                } else {
                    "not required"
                }
            );
            println!("Next: {}", next_step);
        }
    }

    if decision == "stop" {
        std::process::exit(1);
    }
}

/// Shared dispatch: `gate output` — frontstage output-shape gate.
fn cmd_gate_output(path: &str, for_request: Option<&str>, format: &str) {
    use std::io::Read;

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    // Distinguish a bad foreground shape (not even a `## 任务卡`) from a card that
    // claims to be one but fails the canonical validator. Both are blocked; the
    // block_reason differs so governance_miss samples are actionable.
    let shape_ok = task_card_validator::output_is_canonical_header(&content);
    let (decision, block_reason, stage, validation_errors): (
        &str,
        Option<&str>,
        &str,
        Vec<String>,
    ) = if !shape_ok {
        ("stop", Some("bad_output_shape"), "output_shape", Vec::new())
    } else {
        let errs = task_card_validator::validate(&content);
        if errs.is_empty() {
            ("allow", None, "", Vec::new())
        } else {
            ("stop", Some("validation_failed"), "validate", errs)
        }
    };

    let governance_miss = block_reason.map(|reason| {
        prompt_request_classifier::GovernanceMiss::new(reason, stage, &content, for_request)
    });

    match format {
        "json" => {
            let out = serde_json::json!({
                "gate": "output",
                "decision": decision,
                "block_reason": block_reason,
                "validation_errors": validation_errors,
                "governance_miss": governance_miss
                    .as_ref()
                    .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null)),
            });
            match serde_json::to_string_pretty(&out) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Gate: output");
            println!("Path: {}", display_path);
            println!("Decision: {}", decision);
            if let Some(r) = block_reason {
                println!("Block reason: {}", r);
            }
            for e in &validation_errors {
                println!("  - {}", e);
            }
            if let Some(m) = &governance_miss {
                println!(
                    "governance_miss: detected_kind={} reason={} stage={}",
                    m.detected_kind, m.blocked_reason, m.stage
                );
            }
        }
    }

    if decision == "stop" {
        std::process::exit(1);
    }
}

/// Output a gate result (GateCheckOutput or GateErrorOutput) in the requested format.
fn output_gate_result(
    error_output: &execution_policy::GateErrorOutput,
    display_path: &str,
    format: &str,
) {
    match format {
        "json" => match serde_json::to_string_pretty(error_output) {
            Ok(json) => println!("{}", json),
            Err(e) => {
                eprintln!("JSON serialization error: {}", e);
            }
        },
        _ => {
            println!("Gate Decision: stop");
            println!("Path: {}", display_path);
            println!("Error: {}", error_output.error_kind);
            for (i, err) in error_output.errors.iter().enumerate() {
                println!("  {}. {}", i + 1, err);
            }
        }
    }
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

/// Format a GateCheckOutput as human-readable text.
fn format_gate_check_text(
    output: &execution_policy::GateCheckOutput,
    display_path: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Gate Decision: {}", output.decision));
    lines.push(format!("Task card:     {}", display_path));
    lines.push(format!("Schema:        {}", output.schema_version));
    lines.push(String::new());
    lines.push(format_policy_text(&output.resolved_policy));
    lines.join("\n")
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

    // Backward compat: --target adds a single target
    if let Some(target_root) = target {
        all_targets.push((target_name, target_root));
    }

    // Default: if no targets specified, use stable as default
    if all_targets.is_empty() {
        all_targets.push((
            "stable".to_string(),
            PathBuf::from(workflow_sync_check::DEFAULT_STABLE_ROOT),
        ));
    }

    let target_configs: Vec<workflow_sync_check::TargetConfig> = all_targets
        .into_iter()
        .map(|(name, root)| {
            let kind = match name.as_str() {
                "stable" => workflow_sync_check::ProjectKind::Stable,
                "public"
                | "public-core"
                | "public-core-only"
                | "public-full"
                | "public-full-sanitized" => workflow_sync_check::ProjectKind::PublicCoreOnly,
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

    // Print plan first
    if format != "json" {
        println!("{}", bootstrap_dry_run::render_plan_text(&plan));
    }

    // Execute plan
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

// ── M5 dispatch functions ─────────────────────────────────────────────────

/// Shared dispatch: `capability list`
fn cmd_capability_list(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "capability list: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let registry = capability_registry::discover_all(target);
    match format {
        "json" => println!("{}", capability_registry::render_json(&registry)),
        _ => println!("{}", capability_registry::render_text(&registry)),
    }
}

/// Shared dispatch: `capability show`
fn cmd_capability_show(name: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "capability show: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let registry = capability_registry::discover_all(target);
    match capability_registry::find_by_id(&registry, name) {
        Some(cap) => match format {
            "json" => println!("{}", capability_registry::render_one_json(cap)),
            _ => println!("{}", capability_registry::render_one_text(cap)),
        },
        None => {
            eprintln!("capability show: capability not found — {}", name);
            std::process::exit(1);
        }
    }
}

// ── M6 dispatch functions ─────────────────────────────────────────────────

/// Shared dispatch: `receipt generate`
fn cmd_receipt_generate(
    task_card: &str,
    gate_result: &str,
    gate_reason: Option<&str>,
    verifications: &[String],
    delivery_report: Option<&str>,
    format: &str,
) {
    use std::io::Read;

    // Read task card content
    let display_path = if task_card == "-" {
        "(stdin)".to_string()
    } else {
        task_card.to_string()
    };

    let content = if task_card == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("receipt generate: 读取失败 — {}", e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(task_card) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("receipt generate: cannot read task card — {}", e);
                std::process::exit(1);
            }
        }
    };

    // Compute task card hash
    let task_card_hash = receipt::sha256_hex(content.as_bytes());

    // Parse verification results
    let mut verification_results = Vec::new();
    for v in verifications {
        if let Some((cmd, code_str)) = v.rsplit_once(':') {
            let exit_code: i32 = match code_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    eprintln!(
                        "receipt generate: invalid verification format '{}' — expected CMD:EXIT_CODE",
                        v
                    );
                    std::process::exit(2);
                }
            };
            verification_results.push(receipt::VerificationResult {
                command: cmd.to_string(),
                exit_code,
                output_hash: String::new(), // no real output to hash
            });
        } else {
            eprintln!(
                "receipt generate: invalid verification format '{}' — expected CMD:EXIT_CODE",
                v
            );
            std::process::exit(2);
        }
    }

    // Compute delivery report hash if provided
    let delivery_hash = match delivery_report {
        Some(p) => match receipt::hash_file(std::path::Path::new(p)) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("receipt generate: cannot hash delivery report — {}", e);
                std::process::exit(1);
            }
        },
        None => None,
    };

    // Derive receipt_id from first 12 chars of task card hash
    let receipt_id = format!(
        "receipt-{}",
        &task_card_hash[..12.min(task_card_hash.len())]
    );

    let receipt = receipt::Receipt {
        schema_version: "2.0-m6".to_string(),
        receipt_id,
        timestamp: format!("unix-{}", {
            use std::time::SystemTime;
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        }),
        task_card_hash,
        task_card_path: if task_card == "-" {
            None
        } else {
            Some(display_path)
        },
        gate_result: receipt::GateResult {
            decision: gate_result.to_string(),
            reason: gate_reason.map(|s| s.to_string()),
        },
        verification_results,
        delivery_report_hash: delivery_hash,
        exit_code: None,
    };

    match format {
        "json" => println!("{}", receipt::render_receipt_json(&receipt)),
        _ => {
            // Text format: print JSON because text receipt is just the JSON body
            println!("{}", receipt::render_receipt_json(&receipt));
        }
    }
}

/// Shared dispatch: `receipt verify`
fn cmd_receipt_verify(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("receipt verify: cannot read receipt — {}", e);
            std::process::exit(1);
        }
    };

    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("receipt verify: invalid receipt JSON — {}", e);
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

/// Shared dispatch: `compliance check`
fn cmd_compliance_check(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("compliance check: cannot read receipt — {}", e);
            std::process::exit(1);
        }
    };

    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("compliance check: invalid receipt JSON — {}", e);
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

// ── Release dispatch ───────────────────────────────────────────────────────

/// Shared dispatch: `release verify`
fn cmd_release_verify(target: &str, format: &str) {
    let target_root = match target {
        "stable" => PathBuf::from("/Volumes/Projects/example-stable-suite"),
        "public" | "public-core" | "public-full" | "public-full-sanitized" => {
            PathBuf::from("/Volumes/AI Project/ai-dev-env-bootstrap")
        }
        _ => unreachable!("clap guards target values"),
    };

    let target_config = workflow_sync_check::TargetConfig {
        root: target_root.clone(),
        name: target.to_string(),
        kind: match target {
            "stable" => workflow_sync_check::ProjectKind::Stable,
            "public" | "public-core" | "public-full" | "public-full-sanitized" => {
                workflow_sync_check::ProjectKind::PublicCoreOnly
            }
            _ => unreachable!(),
        },
    };

    let options = workflow_sync_check::CheckOptions {
        source_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        source_name: "private".to_string(),
        targets: vec![target_config],
        allowlist_path: None,
    };

    let report_format = match format {
        "json" => workflow_sync_check::ReportFormat::Json,
        _ => workflow_sync_check::ReportFormat::Text,
    };

    let ok = workflow_sync_check::run_cli(options, report_format);
    if !ok {
        std::process::exit(1);
    }
}

fn matches_path_boundary(relative: &str, boundary: &str) -> bool {
    let relative = relative.trim_start_matches("./").replace('\\', "/");
    let boundary = boundary.trim_start_matches("./").replace('\\', "/");

    if boundary.ends_with('/') {
        let dir = boundary.trim_end_matches('/');
        relative == dir || relative.starts_with(&boundary)
    } else {
        relative == boundary
    }
}

fn is_public_release_profile(profile: &str) -> bool {
    profile == "public-full" || profile == "public-core"
}

fn public_release_forbidden_patterns() -> Vec<&'static str> {
    workflow_sync_check::manifest::PUBLIC_FORBIDDEN_PAYLOAD
        .iter()
        .copied()
        .chain([
            "proposals/",
            "graphify-out/",
            "governance/skill-adoption-log.yaml",
            "governance/skill-ignore-list.yaml",
            "governance/backups/",
            ".claude/",
            ".codegraph/",
        ])
        .collect()
}

fn walk_release_files(root: &Path, prefix: &str, files: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(root.join(prefix)) {
        for entry in entries.flatten() {
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(&entry.path())
                .to_string_lossy()
                .to_string();
            if entry.path().is_dir() {
                if rel == ".git" || rel == "target" || rel.starts_with("target/") {
                    continue;
                }
                walk_release_files(root, &rel, files);
            } else {
                files.push(rel);
            }
        }
    }
}

fn release_package_plan(
    source_root: &Path,
    profile: &str,
    dry_run: bool,
) -> (serde_json::Value, bool) {
    let public_full_forbidden_patterns = public_release_forbidden_patterns();
    let mut included: Vec<String> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut exclusion_reasons: Vec<(String, String)> = Vec::new();

    let mut all_files: Vec<String> = Vec::new();
    walk_release_files(source_root, "", &mut all_files);
    all_files.sort();

    if is_public_release_profile(profile) {
        for f in &all_files {
            let forbidden_reason = public_full_forbidden_patterns
                .iter()
                .find(|pat| matches_path_boundary(f, pat))
                .map(|pat| format!("matches forbidden pattern: {}", pat));

            if let Some(reason) = forbidden_reason {
                excluded.push(f.clone());
                exclusion_reasons.push((f.clone(), reason));
                continue;
            }

            included.push(f.clone());
        }
    } else {
        for f in &all_files {
            included.push(f.clone());
        }
    }

    let forbidden_included: Vec<String> = included
        .iter()
        .filter(|file| {
            public_full_forbidden_patterns
                .iter()
                .any(|pat| matches_path_boundary(file, pat))
        })
        .cloned()
        .collect();

    let plan = serde_json::json!({
        "schema_version": "2.0-release",
        "profile": profile,
        "dry_run": dry_run,
        "source_root": source_root.to_string_lossy(),
        "summary": {
            "total_files": all_files.len(),
            "included": included.len(),
            "excluded": excluded.len(),
        },
        "included_files": included,
        "forbidden_included": forbidden_included,
        "excluded_files": excluded.iter().map(|f| {
            let empty_reason = String::new();
            let reason = exclusion_reasons
                .iter()
                .find(|(name, _)| name == f)
                .map(|(_, r)| r)
                .unwrap_or(&empty_reason);
            serde_json::json!({"file": f, "reason": reason})
        }).collect::<Vec<_>>(),
    });

    let has_forbidden_included = plan["forbidden_included"]
        .as_array()
        .map(|items| !items.is_empty())
        .unwrap_or(false);

    (plan, has_forbidden_included)
}

fn render_release_package_plan_text(plan: &serde_json::Value) {
    println!("Release Package Plan");
    println!("====================");
    println!("Schema:    {}", plan["schema_version"]);
    println!("Profile:   {}", plan["profile"]);
    println!("Dry run:   {}", plan["dry_run"]);
    println!("Source:    {}", plan["source_root"]);
    println!();
    println!(
        "Files:     {} total, {} included, {} excluded",
        plan["summary"]["total_files"], plan["summary"]["included"], plan["summary"]["excluded"]
    );
    println!();
    println!("Included:");
    if let Some(files) = plan["included_files"].as_array() {
        for file in files.iter().filter_map(|value| value.as_str()) {
            println!("  + {}", file);
        }
    }
    if let Some(files) = plan["forbidden_included"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Forbidden included:");
            for file in files.iter().filter_map(|value| value.as_str()) {
                println!("  ! {}", file);
            }
        }
    }
    if let Some(files) = plan["excluded_files"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Excluded:");
            for entry in files {
                let file = entry["file"].as_str().unwrap_or("");
                let reason = entry["reason"].as_str().unwrap_or("");
                println!("  - {}  ({})", file, reason);
            }
        }
    }
    println!();
    println!("Verdict: DRY-RUN — no files written. Ready for review.");
}

/// Shared dispatch: `release package`
fn cmd_release_package(profile: &str, dry_run: bool, format: &str) {
    if !dry_run {
        eprintln!("release package: --dry-run is required for now. Apply not yet implemented.");
        std::process::exit(2);
    }

    let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (plan, has_forbidden_included) = release_package_plan(&source_root, profile, dry_run);

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        _ => render_release_package_plan_text(&plan),
    }

    if has_forbidden_included {
        std::process::exit(1);
    }
}

/// Shared dispatch: `rollback plan`
fn cmd_rollback_plan(format: &str) {
    let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let plan = serde_json::json!({
        "schema_version": "2.0-rollback",
        "source_root": source_root.to_string_lossy(),
        "rollback_type": "plan-only",
        "applied": false,
        "note": "Rollback plan is read-only. No files are modified. This is a planning stub — real rollback requires human confirmation and explicit task-card authorization.",
        "affected_scope": {
            "protocol_files": "Would revert to last known stable state",
            "scripts": "Would revert to last known stable state",
            "governance": "Would revert skill adoption/ignore lists to last checkpoint",
        },
        "stopped_because": [
            "rollback apply not yet implemented",
            "requires stable/public state synchronization",
            "requires human confirmation",
        ],
        "next_steps": [
            "Review this plan with Codex",
            "Confirm rollback scope with task-card authorization",
            "Run ags release verify --target stable to check current drift",
        ],
    });

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        _ => {
            println!("Rollback Plan");
            println!("=============");
            println!("Schema:        {}", plan["schema_version"]);
            println!("Source:        {}", plan["source_root"]);
            println!("Type:          {}", plan["rollback_type"]);
            println!("Applied:       {}", plan["applied"]);
            println!();
            println!("Note: {}", plan["note"]);
            println!();
            println!("Affected scope:");
            if let Some(scope) = plan["affected_scope"].as_object() {
                for (k, v) in scope {
                    println!("  {}: {}", k, v);
                }
            }
            println!();
            println!("Stopped because:");
            if let Some(reasons) = plan["stopped_because"].as_array() {
                for r in reasons {
                    println!("  - {}", r.as_str().unwrap_or("?"));
                }
            }
            println!();
            println!("Next steps:");
            if let Some(steps) = plan["next_steps"].as_array() {
                for s in steps {
                    println!("  - {}", s.as_str().unwrap_or("?"));
                }
            }
            println!();
            println!("Verdict: PLAN-ONLY — no rollback applied. Human confirmation required.");
        }
    }
}

fn cmd_private_rollback_plan(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags rollback plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let files = [
        "install-manifest.json",
        "README.md",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
        "bin/ags-mcp-stdio.sh",
        "secrets/README.md",
    ];
    let mut entries: Vec<_> = files
        .iter()
        .map(|rel| {
            let path = target.join(rel);
            serde_json::json!({
                "path": path.to_string_lossy(),
                "exists": path.exists(),
                "backup_candidates": backup_candidates(&path),
            })
        })
        .collect();
    let claude_command_path = claude_ags_command_path();
    entries.push(serde_json::json!({
        "path": claude_command_path.to_string_lossy(),
        "exists": claude_command_path.exists(),
        "backup_candidates": backup_candidates(&claude_command_path),
    }));

    let plan = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": "private",
        "target": target.to_string_lossy(),
        "rollback_type": "plan-only",
        "applied": false,
        "note": "Rollback apply is intentionally not implemented. Review backup candidates and remove or restore files manually with explicit authorization.",
        "files": entries,
    });

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&plan).unwrap_or_default()
        ),
        _ => {
            println!("AGS Private Runtime Rollback Plan");
            println!("=================================");
            println!("Schema:  {}", PRIVATE_INSTALL_SCHEMA);
            println!("Profile: private");
            println!("Target:  {}", target.display());
            println!("Applied: false");
            println!();
            println!("Files:");
            if let Some(files) = plan["files"].as_array() {
                for file in files {
                    println!(
                        "  - {} (exists: {})",
                        file["path"].as_str().unwrap_or("?"),
                        file["exists"]
                    );
                    if let Some(backups) = file["backup_candidates"].as_array() {
                        for backup in backups {
                            println!("      backup: {}", backup.as_str().unwrap_or("?"));
                        }
                    }
                }
            }
            println!();
            println!("Verdict: PLAN-ONLY — no files modified.");
        }
    }
}

fn backup_candidates(path: &Path) -> Vec<String> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{file_name}.");
    let mut backups = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.contains(".bak.") {
                backups.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    backups.sort();
    backups
}

// ── Skill dispatch ─────────────────────────────────────────────────────────

/// Shared dispatch: `skill scan`
fn cmd_skill_scan(format: &str) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let result = skill_governance::scan_skills(&root);

    match format {
        "json" => println!("{}", skill_governance::render_scan_json(&result)),
        _ => println!("{}", skill_governance::render_scan_text(&result)),
    }
}

/// Shared dispatch: `skill check`
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

/// Shared dispatch: `skill propose` — management console proposal.
///
/// Dry-run by default. `--apply` performs only AGS-owned host-entry writes
/// (with backup) through the console's single mutation guard; external
/// installers/registrars are advised, never executed.
fn cmd_skill_propose(action: &str, skill_name: &str, apply: bool, format: &str) {
    use skill_governance::console;
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let Some(parsed) = console::ConsoleAction::from_str(action) else {
        eprintln!("skill propose: unknown action '{action}'");
        std::process::exit(2);
    };
    let ctx = console::ConsoleContext::system(root);
    let result = console::propose_action(&ctx, parsed, skill_name, apply);

    match format {
        "json" => println!("{}", console::render_proposal_json(&result)),
        _ => println!("{}", console::render_proposal_text(&result)),
    }

    // Exit nonzero when an `--apply` could not actually be carried out by AGS:
    // blocked, a write failed, or the action is advised-only (AGS performed
    // nothing and the user must run the advised command). A clean dry-run, a
    // successful apply, and a genuine no-op all exit 0.
    let apply_unfulfilled = apply && matches!(result.apply_status.as_str(), "advised-only");
    if !result.blocked_reasons.is_empty() || !result.apply_errors.is_empty() || apply_unfulfilled {
        std::process::exit(1);
    }
}

/// Shared dispatch: `skill verify --host <host>` — read-only host visibility.
///
/// Informational by default (exit 0). With `--strict` it acts as a post-apply
/// gate: exit nonzero unless status is "ok" (i.e. every expected capability is
/// visible).
fn cmd_skill_verify(host: &str, strict: bool, format: &str) {
    use skill_governance::console;
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ctx = console::ConsoleContext::system(root);
    let result = console::verify_host(&ctx, host);
    let status = result.status.clone();

    match format {
        "json" => println!("{}", console::render_verify_json(&result)),
        _ => println!("{}", console::render_verify_text(&result)),
    }

    if strict && status != "ok" {
        std::process::exit(1);
    }
}

/// Shared dispatch: `skill inventory`
fn cmd_skill_inventory(format: &str, write: bool) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let result = skill_governance::scan_skill_inventory(&root);

    match format {
        "json" => println!("{}", skill_governance::render_inventory_json(&result)),
        _ => println!("{}", skill_governance::render_inventory_text(&result)),
    }

    if write {
        let report_dir = root.join("governance");
        let report_path = report_dir.join("skills-inventory.md");
        let markdown = skill_governance::render_inventory_markdown(&result);
        match std::fs::create_dir_all(&report_dir)
            .and_then(|_| std::fs::write(&report_path, markdown))
        {
            Ok(_) => println!("\nWrote {}", report_path.display()),
            Err(e) => {
                eprintln!("Failed to write {}: {e}", report_path.display());
                std::process::exit(1);
            }
        }
    }
}

/// `ags hooks install` — install the repo-owned pre-push verification hook.
///
/// Default is a DRY-RUN plan (writes nothing). `--confirm` copies
/// templates/hooks/pre-push.verify.sh into .git/hooks/pre-push and marks it
/// executable on Unix. Never installs silently; uninstall by deleting the file.
fn cmd_hooks_install(confirm: bool) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let template = root.join("templates/hooks/pre-push.verify.sh");
    let git_hooks_dir = root.join(".git/hooks");
    let dest = git_hooks_dir.join("pre-push");

    if !template.is_file() {
        eprintln!("Template not found: {}", template.display());
        eprintln!("Run `ags hooks install` from the repository root.");
        std::process::exit(1);
    }

    println!("AGS pre-push hook installer");
    println!("  source:      {}", template.display());
    println!("  destination: {}", dest.display());

    if !confirm {
        println!();
        println!("DRY-RUN — nothing was written.");
        if dest.exists() {
            println!(
                "Note: {} already exists; --confirm would overwrite it.",
                dest.display()
            );
        }
        println!("Re-run with --confirm to install:  ags hooks install --confirm");
        println!("Uninstall later with:              rm {}", dest.display());
        return;
    }

    if !git_hooks_dir.is_dir() {
        eprintln!(
            "Not a git working tree (missing {}).",
            git_hooks_dir.display()
        );
        std::process::exit(1);
    }

    let contents = match std::fs::read_to_string(&template) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read template: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(&dest, &contents) {
        eprintln!("Failed to write {}: {e}", dest.display());
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }
    }
    println!();
    println!("Installed pre-push hook → {}", dest.display());
    println!("Skip once with:  git push --no-verify");
    println!("Uninstall with:  rm {}", dest.display());
}

fn cmd_skill_overview(format: &str, fix: bool) {
    use skill_governance::console;
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let scan = skill_governance::scan_skills(&root);
    let check = skill_governance::check_skills(&root);
    // Unified management-console inventory: skills + MCPs + suite interface +
    // CLI-backed, with canonical body status + per-host thin-index visibility
    // across Claude Code and Codex. Read-only.
    let ctx = console::ConsoleContext::system(root);
    let inventory = console::build_inventory(&ctx, &["claude-code", "codex"]);

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": "2.6.0-skill-console-overview",
                "inventory": inventory,
                "scan": scan,
                "check": check,
                "fix_requested": fix,
                "update_policy": "no_silent_writes_user_confirmation_required",
                "next_steps": if fix {
                    serde_json::json!([
                        "Review the inventory: managed_status, host_visibility, health_status, risk_notes.",
                        "Dry-run a change: `ags skill propose --action <adopt|update|remove|uninstall|repair|verify> --skill <name>`.",
                        "Confirm with `--apply` (writes only AGS-owned host entries; never runs external installers).",
                        "After apply, restart the host and run `ags skill verify --host claude-code`."
                    ])
                } else {
                    serde_json::json!([
                        "Run `ags skill verify --host claude-code` to check host visibility.",
                        "Run `ags skill --fix` for update guidance. No files are modified by overview."
                    ])
                }
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", console::render_inventory_text(&inventory));
            println!();
            println!("{}", skill_governance::render_scan_text(&scan));
            println!();
            println!("{}", skill_governance::render_check_text(&check));
            println!();
            if fix {
                println!("Skill Update Guidance");
                println!("=====================");
                println!("No skill files were modified.");
                println!("Review the inventory above, then use:");
                println!("  ags skill propose --action adopt --skill <name>          # dry-run");
                println!("  ags skill propose --action adopt --skill <name> --apply  # confirm");
                println!("  ags skill verify  --host claude-code                     # post-restart check");
                println!(
                    "Apply writes only AGS-owned host entries (with backup) and never runs external installers."
                );
            } else {
                println!(
                    "Next: `ags skill verify --host claude-code` for host visibility, or `ags skill --fix` for update guidance. No files were modified."
                );
            }
        }
    }

    if !check.passed && !fix {
        std::process::exit(1);
    }
}

// ── Run dispatch ───────────────────────────────────────────────────────────

/// Shared dispatch: `run`
fn cmd_run(path: &str, check_only: bool, dry_run: bool, approve_writes: bool, format: &str) {
    let plan = runner::run_task_card(path, check_only, dry_run, approve_writes);

    match format {
        "json" => println!("{}", runner::render_json(&plan)),
        _ => println!("{}", runner::render_text(&plan)),
    }

    // Exit code: 0 if gate allows/confirms, 1 if stop or validation failed
    let should_exit_1 = plan.gate_decision == "stop" || !plan.validation_passed;
    if check_only && should_exit_1 {
        std::process::exit(1);
    }
    if !check_only && should_exit_1 {
        std::process::exit(1);
    }
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

/// `ags verify lane` — classify the change lane for a git diff range.
///
/// Deterministic, read-only. `range` is the commit range under review (e.g.
/// `<a1-head>..HEAD`), or `cached` / `staged` for the index. The push gate uses
/// this to route hygiene changes onto a minimal path; it never defaults the
/// range so a multi-commit push is not misjudged by a `HEAD~1` assumption.
fn cmd_verify_lane(range: &str, format: &str, target: &Path) {
    if !target.exists() {
        eprintln!("verify lane: target does not exist — {}", target.display());
        std::process::exit(1);
    }

    let range_norm = if range == "cached" || range == "staged" {
        format!("--{}", range)
    } else {
        range.to_string()
    };

    match ags_verify::classify_from_git_range(target, &range_norm) {
        Ok(classification) => match format {
            "json" => match serde_json::to_string_pretty(&classification) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("verify lane: JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            },
            _ => {
                let components: Vec<&str> = classification
                    .components
                    .iter()
                    .map(|c| c.as_str())
                    .collect();
                println!("Lane: {}", classification.lane.as_str());
                println!("Profile: {}", classification.profile.as_str());
                println!("Components: {}", components.join(", "));
                println!("Changed files: {}", classification.changed_files.len());
            }
        },
        Err(e) => {
            eprintln!("verify lane: {}", e);
            std::process::exit(1);
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup {
            target,
            with_evomap,
            yes,
            force,
            register_claude,
            dry_run,
            format,
        } => cmd_setup(
            target,
            with_evomap,
            yes,
            force,
            register_claude,
            dry_run,
            &format,
        ),
        Commands::Init {
            target,
            slug,
            dry_run,
            mode,
            migrate_tracked_overlay,
            format,
        } => {
            let overlay_mode = OverlayMode::parse(&mode);
            if migrate_tracked_overlay && overlay_mode == OverlayMode::Shared {
                eprintln!(
                    "ags init: --migrate-tracked-overlay requires --mode local (shared/tracked overlays stay committed)"
                );
                std::process::exit(1);
            }
            cmd_project_init(
                &target,
                slug,
                dry_run,
                &format,
                overlay_mode,
                migrate_tracked_overlay,
            )
        }
        Commands::Plan {
            profile,
            target,
            with_evomap,
            format,
        } => cmd_private_plan(&profile, target, &format, with_evomap),
        Commands::Apply {
            profile,
            target,
            yes,
            force,
            with_evomap,
            register_claude,
            format,
        } => cmd_private_apply(
            &profile,
            target,
            yes,
            force,
            &format,
            with_evomap,
            register_claude,
        ),
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
            fix,
            repair,
            dry_run,
            target,
        } => cmd_doctor(&format, repair || fix, dry_run, &target),
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
                // --apply REQUIRES --target
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

        // ── M3 Gate operations ──
        Commands::Gate { action } => match action {
            GateAction::Check {
                path,
                format,
                approve_writes,
            } => cmd_gate_check(&path, &format, approve_writes),
            GateAction::PromptRequest {
                request,
                target,
                no_preflight,
                format,
            } => cmd_gate_prompt_request(&request, &target, no_preflight, &format),
            GateAction::Output {
                path,
                for_request,
                format,
            } => cmd_gate_output(&path, for_request.as_deref(), &format),
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

        // ── M5 Capability Registry ──
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
                format,
            } => cmd_receipt_generate(
                &task_card,
                &gate_result,
                gate_reason.as_deref(),
                &verifications,
                delivery_report.as_deref(),
                &format,
            ),
            ReceiptAction::Verify { path, format } => cmd_receipt_verify(&path, &format),
        },
        Commands::Compliance { action } => match action {
            ComplianceAction::Check { path, format } => cmd_compliance_check(&path, &format),
        },

        // ── Session operations (M2 — kernel activation) ──
        Commands::Session { action } => match action {
            SessionAction::Preflight {
                for_agent,
                target,
                format,
            } => cmd_session_preflight(&for_agent, &target, &format),
        },

        // ── Skill governance operations ──
        Commands::Skill {
            action,
            format,
            fix,
        } => match action {
            Some(SkillAction::Scan { format }) => cmd_skill_scan(&format),
            Some(SkillAction::Check { format }) => cmd_skill_check(&format),
            Some(SkillAction::Propose {
                action,
                skill,
                apply,
                format,
            }) => cmd_skill_propose(&action, &skill, apply, &format),
            Some(SkillAction::Verify {
                host,
                strict,
                format,
            }) => cmd_skill_verify(&host, strict, &format),
            Some(SkillAction::Inventory { format, write }) => cmd_skill_inventory(&format, write),
            None => cmd_skill_overview(&format, fix),
        },

        // ── Release / Rollback operations ──
        Commands::Release { action } => match action {
            ReleaseAction::Verify { target, format } => cmd_release_verify(&target, &format),
            ReleaseAction::Package {
                profile,
                dry_run,
                format,
            } => cmd_release_package(&profile, dry_run, &format),
        },
        Commands::Rollback { action } => match action {
            RollbackAction::Plan {
                profile,
                target,
                format,
            } => match profile {
                Some(profile) => cmd_private_rollback_plan(&profile, target, &format),
                None => cmd_rollback_plan(&format),
            },
        },

        // ── MCP operations ──
        Commands::Mcp { action } => match action {
            McpAction::Serve { transport } => cmd_mcp_serve(&transport),
        },
        Commands::Hooks { action } => match action {
            HooksAction::Install { confirm } => cmd_hooks_install(confirm),
        },

        // ── Runner operations ──
        Commands::Run {
            path,
            check_only,
            dry_run,
            approve_writes,
            format,
        } => cmd_run(&path, check_only, dry_run, approve_writes, &format),

        // ── Verify operations ──
        Commands::Verify {
            action,
            scope,
            profile,
            format,
            target,
            with_evomap,
        } => {
            if let Some(profile) = profile {
                let install_target = if target == *"." {
                    None
                } else {
                    Some(target.clone())
                };
                cmd_private_verify(&profile, install_target, &format, with_evomap);
            }
            match action {
                Some(VerifyAction::Run {
                    scope,
                    format,
                    target,
                }) => cmd_verify_run(&scope, &format, &target),
                Some(VerifyAction::Lane {
                    range,
                    format,
                    target,
                }) => cmd_verify_lane(&range, &format, &target),
                None => cmd_verify_run(&scope, &format, &target),
            }
        }

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

// ── MCP server handler ──────────────────────────────────────────────────────

/// Start the AGS MCP server with the given transport.
///
/// V1 supports only stdio transport. The server reads line-delimited
/// JSON-RPC 2.0 messages from stdin and writes responses to stdout.
/// Stderr is reserved for server logging.
///
/// AGS MCP and EvoMap MCP are parallel peers — AGS MCP does NOT proxy,
/// wrap, or broker EvoMap MCP calls.
fn cmd_mcp_serve(transport: &str) {
    match transport {
        "stdio" => {
            eprintln!(
                "[ags-mcp] starting AGS MCP host initialization adapter v{} on stdio",
                AGS_VERSION
            );
            eprintln!("[ags-mcp] AGS MCP is the mandatory governance interface (NOT a governed third-party MCP).");
            eprintln!("[ags-mcp] EvoMap boundary: AGS MCP and EvoMap MCP are parallel peers. AGS MCP does not proxy EvoMap MCP.");
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

#[cfg(test)]
mod release_package_tests {
    use super::{
        claude_ags_command_content, claude_ags_command_path,
        codex_ags_command_skill_agent_metadata_content, codex_ags_command_skill_content,
        codex_ags_command_skill_specs, codex_ags_named_skill_path, is_public_release_profile,
        matches_path_boundary, private_install_plan, project_init_plan, release_package_plan,
        retired_codex_ags_skill_dirs, write_project_init_file, AGS_VERSION,
    };
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn file_boundary_requires_exact_match() {
        assert!(matches_path_boundary(
            "scripts/verify.sh",
            "scripts/verify.sh"
        ));
        assert!(!matches_path_boundary(
            "scripts/verify.sh.bak",
            "scripts/verify.sh"
        ));
        assert!(!matches_path_boundary(
            "scripts/verify.sh/extra",
            "scripts/verify.sh"
        ));
    }

    #[test]
    fn directory_boundary_allows_descendants_only_when_marked_as_directory() {
        assert!(matches_path_boundary("crates", "crates/"));
        assert!(matches_path_boundary("crates/runner/src/lib.rs", "crates/"));
        assert!(!matches_path_boundary("crates-private/lib.rs", "crates/"));
        assert!(!matches_path_boundary("crates/runner/src/lib.rs", "crates"));
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .to_path_buf()
    }

    #[test]
    fn private_install_plan_excludes_evomap_by_default() {
        let target = std::env::temp_dir().join("ags-private-install-plan-default-test");
        let plan = private_install_plan(&workspace_root(), &target, false);
        assert!(!plan.with_evomap);
        assert!(!plan
            .files
            .iter()
            .any(|file| file.path.ends_with("mcp/gep.mcp.json")));
        assert!(plan
            .files
            .iter()
            .any(|file| file.path == claude_ags_command_path()));
        let manifest = plan
            .files
            .iter()
            .find(|file| file.path.ends_with("install-manifest.json"))
            .expect("manifest file must be generated");
        assert!(manifest.content.contains("\"slash_command\": \"/ags\""));
        assert!(manifest.content.contains("ags-setup"));
        assert!(manifest.content.contains("ags-init"));
        assert!(manifest.content.contains("ags-skill"));
        assert!(manifest.content.contains(".claude/commands/ags.md"));
        assert!(!manifest.content.contains(".codex/skills/ags/SKILL.md"));
        for (name, _, _, _, _) in codex_ags_command_skill_specs() {
            assert!(plan
                .files
                .iter()
                .any(|file| file.path == codex_ags_named_skill_path(name)));
        }
        for retired_dir in retired_codex_ags_skill_dirs() {
            assert!(plan.cleanup_dirs.iter().any(|dir| dir == &retired_dir));
        }
    }

    #[test]
    fn tencent_agent_host_snippets_register_ags_mcp() {
        // Tencent Agent / WorkBuddy / CodeBuddy-Code are platform-host MCP
        // integration snippets. They register AGS MCP only; they do not create
        // runtime adapters or change execution-policy authority.
        let target = std::env::temp_dir().join("ags-tencent-snippet-struct-test");
        let plan = private_install_plan(&workspace_root(), &target, false);
        for name in [
            "hosts/tencent-agent.mcp.snippet.json",
            "hosts/workbuddy.mcp.snippet.json",
            "hosts/codebuddy-code.mcp.snippet.json",
        ] {
            let file = plan
                .files
                .iter()
                .find(|f| f.path.ends_with(name))
                .unwrap_or_else(|| panic!("missing host MCP snippet: {name}"));
            let json: serde_json::Value = serde_json::from_str(&file.content)
                .unwrap_or_else(|e| panic!("{name} must be valid JSON: {e}"));
            let entry = json
                .get("mcpServers")
                .and_then(|servers| servers.get("ags"))
                .unwrap_or_else(|| panic!("{name} must expose mcpServers.ags"));
            assert_eq!(
                entry.get("mandatory_first_tool").and_then(|v| v.as_str()),
                Some("ags_preflight"),
                "{name} must register ags_preflight as mandatory_first_tool"
            );
            assert_eq!(
                entry.get("command").and_then(|v| v.as_str()),
                Some("ags"),
                "{name} ags entry must launch the `ags` command"
            );
        }
    }

    #[test]
    fn claude_ags_command_mentions_preflight_and_current_version() {
        let content = claude_ags_command_content();
        assert!(content.contains("ags_preflight"));
        assert!(content.contains("ags session preflight --for claude-code --target ."));
        assert!(content.contains("ags setup --with-evomap --yes --force --register-claude"));
        assert!(content.contains("ags init --target ."));
        assert!(content.contains("/ags setup"));
        assert!(content.contains("/ags init"));
        assert!(content.contains(AGS_VERSION));
    }

    #[test]
    fn codex_ags_command_skills_mention_top_level_routes() {
        for (name, display_name, _, _, summary) in codex_ags_command_skill_specs() {
            let content = codex_ags_command_skill_content(name, display_name, summary);
            let route = name.strip_prefix("ags-").unwrap_or(name);
            assert!(content.contains(&format!("name: \"{name}\"")));
            assert!(content.contains(&format!("/ags {route}")));
            assert!(content.contains("ags session preflight --for codex --target ."));
            assert!(content.contains(AGS_VERSION));
            assert!(content.contains("必须先执行"));
        }
    }

    #[test]
    fn codex_ags_skill_metadata_uses_command_shaped_display_names() {
        for (_, display_name, short_description, default_prompt, _) in
            codex_ags_command_skill_specs()
        {
            let metadata = codex_ags_command_skill_agent_metadata_content(
                display_name,
                short_description,
                default_prompt,
            );
            assert!(display_name.starts_with("AGS "));
            assert!(short_description
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch)));
            assert!(metadata.contains(&format!("display_name: \"{display_name}\"")));
            assert!(metadata.contains(short_description));
            assert!(metadata.contains(default_prompt));
        }
    }

    #[test]
    fn private_install_plan_includes_evomap_when_requested() {
        let target = std::env::temp_dir().join("ags-private-install-plan-evomap-test");
        let plan = private_install_plan(&workspace_root(), &target, true);
        assert!(plan.with_evomap);
        assert!(plan
            .files
            .iter()
            .any(|file| file.path.ends_with("mcp/gep.mcp.json")));
        assert!(plan.files.iter().any(|file| file
            .path
            .ends_with("hosts/claude-code.evomap-mcp.snippet.json")));
        assert!(plan
            .files
            .iter()
            .any(|file| file.path.ends_with("bin/evolver-proxy-mcp")));
    }

    fn unique_temp_project(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{}-{suffix}", std::process::id()))
    }

    #[test]
    fn project_init_plan_ignores_gep_runtime_assets() {
        let target = unique_temp_project("ags-project-init-ignore-plan");
        std::fs::create_dir_all(&target).unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");
        assert!(gitignore.content.contains("assets/gep/"));
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn project_init_gitignore_append_is_idempotent() {
        let target = unique_temp_project("ags-project-init-ignore-idempotent");
        std::fs::create_dir_all(&target).unwrap();
        let gitignore_path = target.join(".gitignore");
        std::fs::write(&gitignore_path, "/target/\n").unwrap();
        let plan = project_init_plan(&target, None);
        let gitignore = plan
            .files
            .iter()
            .find(|file| file.path.ends_with(".gitignore"))
            .expect("project init should manage .gitignore");

        let first = write_project_init_file(gitignore, &plan.append_files);
        let second = write_project_init_file(gitignore, &plan.append_files);

        assert_eq!(first.status, suite_doctor::CheckStatus::Pass);
        assert_eq!(second.status, suite_doctor::CheckStatus::Pass);
        let content = std::fs::read_to_string(&gitignore_path).unwrap();
        assert_eq!(content.matches("assets/gep/").count(), 1);
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn public_release_profile_detection_is_explicit() {
        assert!(is_public_release_profile("public-core"));
        assert!(is_public_release_profile("public-full"));
        assert!(!is_public_release_profile("private-full"));
    }

    #[test]
    fn public_release_package_keeps_rust_workspace_and_strips_evomap_runtime() {
        let (plan, failed) = release_package_plan(&workspace_root(), "public-full", true);
        assert!(
            !failed,
            "public-full package plan must not include forbidden files"
        );

        let included = plan["included_files"]
            .as_array()
            .expect("included_files must be an array");
        let included: Vec<&str> = included.iter().filter_map(|value| value.as_str()).collect();

        assert!(included.contains(&"AGENTS.md"));
        assert!(included.contains(&"Cargo.toml"));
        assert!(included.contains(&"crates/ags-cli/src/main.rs"));
        assert!(included.contains(&"protocol/task-card-template.md"));
        assert!(!included.contains(&"manifests/templates/runtime-profiles.template.yaml"));
        assert!(!included.contains(&"protocol/evolution-memory.md"));

        for rel in included {
            let lower = rel.to_ascii_lowercase();
            assert!(
                !lower.contains("evomap")
                    && !lower.contains("evolver")
                    && !lower.contains("/gep/")
                    && !lower.ends_with("/gep")
                    && !lower.starts_with(".evolver/")
                    && !lower.starts_with("assets/gep/"),
                "public package leaked EvoMap/GEP surface: {rel}"
            );
        }
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::{
        apply_overlay, compute_overlay_plan, git_tracked_set, merge_overlay_exclude,
        overlay_exclude_entries, overlay_migratable_entries, InstallFile, OverlayMode,
        OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_END,
    };
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn mk(path: PathBuf) -> InstallFile {
        InstallFile {
            path,
            description: String::new(),
            content: String::new(),
            mode: None,
        }
    }

    #[test]
    fn overlay_entries_are_anchored_and_skip_paths_outside_repo() {
        let target = Path::new("/tmp/ags-repo");
        let files = vec![
            mk(target.join("AGENTS.md")),
            mk(target.join("protocol/agent-task-protocol.md")),
            mk(PathBuf::from("/home/u/.agents/memory/x/context-capsule.md")),
        ];
        let entries = overlay_exclude_entries(target, &files);
        assert_eq!(
            entries,
            vec![
                "/AGENTS.md".to_string(),
                "/protocol/agent-task-protocol.md".to_string(),
            ]
        );
    }

    #[test]
    fn migratable_excludes_shared_append_targets() {
        let entries = vec![
            "/.gitignore".to_string(),
            "/AGENTS.md".to_string(),
            "/CLAUDE.md".to_string(),
            "/WORKSPACE.md".to_string(),
            "/protocol/agent-task-protocol.md".to_string(),
        ];
        let migratable = overlay_migratable_entries(&entries);
        assert_eq!(
            migratable,
            vec![
                "/WORKSPACE.md".to_string(),
                "/protocol/agent-task-protocol.md".to_string(),
            ]
        );
        for shared in ["/AGENTS.md", "/CLAUDE.md", "/.gitignore"] {
            assert!(!migratable.iter().any(|e| e == shared));
        }
    }

    #[test]
    fn merge_overlay_exclude_is_idempotent_and_preserves_user_lines() {
        let entries = vec!["/AGENTS.md".to_string(), "/WORKSPACE.md".to_string()];
        let once = merge_overlay_exclude("build/\n*.log\n", &entries);
        assert!(!once.had_malformed_markers);
        let once = once.content;
        assert!(once.contains("build/"));
        assert!(once.contains("*.log"));
        assert!(once.contains("/AGENTS.md"));
        assert!(once.contains("/WORKSPACE.md"));
        assert!(once.contains(OVERLAY_BLOCK_BEGIN));
        assert!(once.contains(OVERLAY_BLOCK_END));

        let twice = merge_overlay_exclude(&once, &entries).content;
        assert_eq!(once, twice, "overlay merge must be idempotent");
        assert_eq!(twice.matches(OVERLAY_BLOCK_BEGIN).count(), 1);
        assert_eq!(twice.matches(OVERLAY_BLOCK_END).count(), 1);
    }

    #[test]
    fn merge_overlay_exclude_empty_entries_removes_block() {
        let with = merge_overlay_exclude("user.txt\n", &["/AGENTS.md".to_string()]).content;
        assert!(with.contains(OVERLAY_BLOCK_BEGIN));
        let without = merge_overlay_exclude(&with, &[]).content;
        assert!(!without.contains(OVERLAY_BLOCK_BEGIN));
        assert!(!without.contains(OVERLAY_BLOCK_END));
        assert!(without.contains("user.txt"));
    }

    #[test]
    fn merge_overlay_exclude_preserves_user_lines_when_begin_has_no_end() {
        // A truncated managed block: BEGIN with no matching END. User ignore
        // lines after the orphan BEGIN must NOT be swallowed (the old bug).
        let malformed = format!(
            "secret.key\n{}\n/AGENTS.md\nkeep-me.txt\nbuild/\n",
            OVERLAY_BLOCK_BEGIN
        );
        let entries = vec!["/WORKSPACE.md".to_string()];
        let merged = merge_overlay_exclude(&malformed, &entries);

        assert!(
            merged.had_malformed_markers,
            "orphan BEGIN must be flagged as malformed"
        );
        for line in ["secret.key", "/AGENTS.md", "keep-me.txt", "build/"] {
            assert!(
                merged.content.contains(line),
                "user line {line:?} must be preserved, got:\n{}",
                merged.content
            );
        }
        // A fresh well-formed block is appended rather than replacing in place.
        assert!(merged.content.contains("/WORKSPACE.md"));
        assert!(merged.content.contains(OVERLAY_BLOCK_END));

        // Re-running must neither delete content nor grow unbounded.
        let again = merge_overlay_exclude(&merged.content, &entries);
        assert_eq!(
            merged.content, again.content,
            "malformed-input merge must still be idempotent"
        );
        assert_eq!(again.content.matches(OVERLAY_BLOCK_END).count(), 1);
    }

    #[test]
    fn merge_overlay_exclude_preserves_lines_around_stray_end() {
        // A stray END with no preceding BEGIN must be kept as ordinary content.
        let stray = format!("a.txt\n{}\nb.txt\n", OVERLAY_BLOCK_END);
        let merged = merge_overlay_exclude(&stray, &["/AGENTS.md".to_string()]);
        assert!(merged.had_malformed_markers);
        assert!(merged.content.contains("a.txt"));
        assert!(merged.content.contains("b.txt"));
        assert!(merged.content.contains("/AGENTS.md"));
    }

    #[test]
    fn merge_overlay_exclude_preserves_user_lines_when_begin_is_nested() {
        // Nested BEGIN markers make the existing marker structure malformed.
        // Once malformed, AGS must preserve all original lines and only append
        // a fresh managed block; it must not treat the inner BEGIN..END as a
        // removable well-formed block.
        let malformed = format!(
            "before\n{}\nouter-user\n{}\ninner-user-should-stay\n{}\nafter\n",
            OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_END
        );
        let entries = vec!["/WORKSPACE.md".to_string()];
        let merged = merge_overlay_exclude(&malformed, &entries);

        assert!(merged.had_malformed_markers);
        for line in [
            "before",
            "outer-user",
            "inner-user-should-stay",
            "after",
            "/WORKSPACE.md",
        ] {
            assert!(
                merged.content.contains(line),
                "line {line:?} must be preserved or appended, got:\n{}",
                merged.content
            );
        }

        let again = merge_overlay_exclude(&merged.content, &entries);
        assert_eq!(
            merged.content, again.content,
            "nested malformed merge must be idempotent"
        );
        assert_eq!(again.content.matches("/WORKSPACE.md").count(), 1);
    }

    fn unique_repo(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ags-overlay-{name}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init", "-q"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git init failed");
        dir
    }

    #[test]
    fn local_overlay_hides_files_and_is_idempotent() {
        let target = unique_repo("local");
        std::fs::write(target.join("WORKSPACE.md"), "ags").unwrap();
        std::fs::write(target.join("AGENTS.md"), "ags").unwrap();
        let files = vec![
            mk(target.join("WORKSPACE.md")),
            mk(target.join("AGENTS.md")),
        ];

        let plan = compute_overlay_plan(&target, &files, OverlayMode::Local, false);
        assert!(plan.is_git_repo);
        let _ = apply_overlay(&plan);

        let exclude = plan.exclude_path.clone().unwrap();
        let body = std::fs::read_to_string(&exclude).unwrap();
        assert!(body.contains("/WORKSPACE.md"));
        assert!(body.contains("/AGENTS.md"));

        let status = std::process::Command::new("git")
            .current_dir(&target)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&status.stdout).trim().is_empty(),
            "git status should be clean, got: {}",
            String::from_utf8_lossy(&status.stdout)
        );

        // Re-running must not change the exclude file.
        let plan2 = compute_overlay_plan(&target, &files, OverlayMode::Local, false);
        let _ = apply_overlay(&plan2);
        let body2 = std::fs::read_to_string(&exclude).unwrap();
        assert_eq!(body, body2, "second apply must be idempotent");

        let _ = std::fs::remove_dir_all(&target);
    }

    #[test]
    fn migrate_untracks_ags_files_but_keeps_shared_and_working_copy() {
        let target = unique_repo("migrate");
        std::fs::write(target.join("WORKSPACE.md"), "ags-owned").unwrap();
        std::fs::write(target.join("AGENTS.md"), "repo-owned").unwrap();
        let added = std::process::Command::new("git")
            .current_dir(&target)
            .args(["add", "WORKSPACE.md", "AGENTS.md"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(added, "git add failed");

        let files = vec![
            mk(target.join("WORKSPACE.md")),
            mk(target.join("AGENTS.md")),
        ];
        let plan = compute_overlay_plan(&target, &files, OverlayMode::Local, true);
        assert!(plan.tracked_migratable.iter().any(|e| e == "/WORKSPACE.md"));
        assert!(
            !plan.tracked_migratable.iter().any(|e| e == "/AGENTS.md"),
            "shared append target must never be migrated"
        );
        let _ = apply_overlay(&plan);

        let tracked = git_tracked_set(&target);
        assert!(
            !tracked.contains("WORKSPACE.md"),
            "AGS-owned file should be untracked after migrate"
        );
        assert!(
            tracked.contains("AGENTS.md"),
            "shared file must stay tracked (safety)"
        );
        assert!(
            target.join("WORKSPACE.md").exists(),
            "working copy must be preserved by git rm --cached"
        );

        let _ = std::fs::remove_dir_all(&target);
    }
}
