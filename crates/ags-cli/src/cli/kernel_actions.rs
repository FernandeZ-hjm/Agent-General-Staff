//! Hidden kernel command action sub-enums.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum TaskAction {
    /// Validate one or more task cards
    Validate {
        /// Task card files to validate (use "-" for stdin)
        paths: Vec<String>,
    },
    /// Validate that a delivery report closes every task-card goal,
    /// acceptance criterion, and verification item, with matching hashes.
    Close {
        /// Canonical task-card file
        task_card: String,
        /// Delivery-report file
        delivery_report: String,
        /// Output format
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Compile a task intent into a canonical task card (M4).
    ///
    /// Reads a field-structured confirmed handoff contract (or stdin with "-") and deterministically
    /// compiles it into the canonical task-card skeleton (the classic fixed
    /// skeleton in protocol/task-card-template.md; the compact format has been
    /// removed).  This is a rule engine only — no AI calls, no free-form
    /// prompt generation.
    ///
    /// Slot filling uses project context (CLAUDE.md, WORKSPACE.md, protocol
    /// files, known workspace identity, and local memory paths).  Slots that
    /// cannot be filled are reported as missing and the command exits 1.
    Compile {
        /// Confirmed handoff contract file (use "-" for stdin)
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
        /// This is one half of the hard gate between a confirmed handoff
        /// contract and task-card generation. Without this flag, the compiler
        /// produces a diagnostic report only. Set it only after an explicit
        /// task-card instruction ("生成任务卡", "按这个方案出任务卡", etc.).
        ///
        /// Without --task-card-requested, the report will show
        /// executable_allowed=false with block_reason=task_card_not_requested.
        #[arg(long, default_value_t = false)]
        task_card_requested: bool,
        /// The host has reached the final, decision-complete artifact in its
        /// Plan mode. The artifact is compiled directly as the canonical AGS
        /// task card; no extra task-card confirmation prompt is required.
        ///
        /// Host UI approval must switch out of Plan mode and dispatch this
        /// exact card without regeneration. This flag is an alternative to
        /// --task-card-requested and still requires
        /// --confirmed-handoff-contract.
        #[arg(long, default_value_t = false, conflicts_with = "task_card_requested")]
        host_plan_mode_final: bool,
        /// Structured evidence that solution, scope, verification, and handoff
        /// boundaries have already been confirmed for this task card.
        ///
        /// This does not authorize mutation. Task-card generation additionally
        /// requires either --task-card-requested or --host-plan-mode-final and
        /// is still blocked when the intent contains unresolved or reopened
        /// design work.
        #[arg(long, default_value_t = false)]
        confirmed_handoff_contract: bool,
    },
}
#[derive(Subcommand)]
pub(crate) enum PolicyAction {
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
        /// Write-approval audit/hint signal (CLI flag); may act as the M9
        /// generic-adapter capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Structured current-task approval signal from the live request
        /// (audit/hint only — task level does not downgrade the permission mode).
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,
    },

    /// Explain each policy decision with rule IDs, downgrades, stop reasons, and safety assertions.
    Explain {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Write-approval audit/hint signal (CLI flag); may act as the M9
        /// generic-adapter capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Structured current-task approval signal from the live request
        /// (audit/hint only — task level does not downgrade the permission mode).
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,
    },

    /// Validate, resolve, and exit with decision: 0 = allow, 1 = stop/validation fail.
    Check {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Write-approval audit/hint signal (CLI flag); may act as the M9
        /// generic-adapter capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Structured current-task approval signal from the live request
        /// (audit/hint only — task level does not downgrade the permission mode).
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,
    },
}
/// Runner-facing gate operations (M3).
#[derive(Subcommand)]
pub(crate) enum GateAction {
    /// Run the gate check and output a runner-level decision.
    ///
    /// Outputs decision: allow|stop with embedded resolved policy.
    /// On validation failure, outputs structured decision=stop JSON with
    /// error details — never just a raw exit code.
    Check {
        /// Task card file (use "-" for stdin)
        path: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Write-approval audit/hint signal (CLI flag); may act as the M9
        /// generic-adapter capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Structured current-task approval: the host detected an explicit user
        /// execution instruction ("实现 / 修复 / 做完") on the live request.
        /// Audit/hint only — task level does not downgrade the permission mode.
        /// Never derived from task-card text.
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,
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

    /// Task-card skill-tag availability gate (the three-gate rule).
    ///
    /// Validates a task card's trailing `[skill: …]` tags against BOTH the static
    /// registry (route_state: routable + legal invoke_hint) AND the live machine
    /// snapshot (visible + healthy + auth-satisfied + enrolled for the host). A
    /// degraded / auth-required / not-visible / unmanaged / not-routable tag is
    /// REJECTED — decision = stop. Deterministic, fail-closed. This is the runtime
    /// availability layer on top of the validator's offline static gate.
    SkillTags {
        /// Task card file (use "-" for stdin).
        path: String,
        /// Target repository path used to read capability manifests + snapshot.
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Active host the tags must be available for (default `claude-code`).
        #[arg(long = "for", default_value = "claude-code")]
        for_agent: String,
        /// Output format: text (human-readable) or json (machine-readable)
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Receipt operations (M6).
#[derive(Subcommand)]
pub(crate) enum ReceiptAction {
    /// Generate a receipt from a task card.
    Generate {
        /// Task card file (use "-" for stdin)
        #[arg(long = "task-card")]
        task_card: String,
        /// Gate decision: allow or stop
        #[arg(long, default_value = "allow", value_parser = ["allow", "stop"])]
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
pub(crate) enum ComplianceAction {
    /// Check a receipt for compliance with policy gates.
    Check {
        /// Receipt file path
        path: String,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
// ── M2 Object Command Sub-enums ───────────────────────────────────────────
#[derive(Subcommand)]
pub(crate) enum ProjectAction {
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
pub(crate) enum ProtocolAction {
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
pub(crate) enum AgentAction {
    /// Export agent-specific project instructions (read-only).
    ///
    /// Generates instructions tailored to known agents, with a generic
    /// governed-host fallback for any other non-empty agent identifier.
    Instructions {
        /// Agent identifier: codex, claude-code, omp, cursor, tencent-agent, workbuddy, codebuddy-code, cowork, or another host id
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
pub(crate) enum SessionAction {
    /// Run aggregated session preflight for an agent (kernel activation entry point).
    ///
    /// Combines `project detect`, `protocol status`, and `agent instructions`
    /// into a single read-only report. Reports project identity, protocol
    /// status, memory capsule/task-memory paths, stop conditions, warnings,
    /// failures, and recommended next steps.
    Preflight {
        /// Agent identifier: codex, claude-code, omp, cursor, tencent-agent, workbuddy, codebuddy-code, cowork, or another host id
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
/// preflight). `full` adds advisory local drift checks, `release` validates a
/// public source tree, and `promotion` compares a source to an explicit public
/// worktree.
#[derive(Subcommand)]
pub(crate) enum VerifyAction {
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
/// (Tencent Agent, Codex, OMP, Cursor, Claude Code) to call as a global
/// governance capability.
///
/// Third-party MCP servers remain host-owned and are never proxied by AGS.
#[derive(Subcommand)]
pub(crate) enum McpAction {
    /// Start the thin AGS MCP adapter on stdio.
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
    /// Internal workspace daemon entrypoint used by the stdio adapter.
    #[command(hide = true)]
    WorkspaceDaemon {
        #[arg(long)]
        workspace: PathBuf,
    },
}
/// `ags hooks` — manage repo-owned git hooks (opt-in, explicit confirmation).
#[derive(Subcommand)]
pub(crate) enum HooksAction {
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

// ── Release actions ─────────────────────────────────────────────────────────
/// Release packaging operations — dry-run only, no apply to stable/public.
#[derive(Subcommand)]
pub(crate) enum ReleaseAction {
    /// Plan a release package — lists what files WOULD be included.
    ///
    /// Public profiles include the public Rust workspace and governance
    /// runtime, while excluding build output, local runtime state, real memory,
    /// preinstalled skill packs, and local agent config.
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
