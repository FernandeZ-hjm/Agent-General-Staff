//! CLI command surface (clap).

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod actions;
mod kernel_actions;
pub(crate) use actions::*;
pub(crate) use kernel_actions::*;

#[derive(Parser)]
#[command(
    name = "ags",
    about = "Agent Governance Suite CLI",
    after_help = "Common flow:\n  ags setup --yes      Initialize the global AGS runtime\n  ags init             Onboard the current project\n  ags doctor           Diagnose AGS health\n  ags skill            Review local skills",
    version = crate::context::AGS_VERSION,
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

// ── M1 Object Command Sub-enums ───────────────────────────────────────────
#[derive(Subcommand)]
pub(crate) enum Commands {
    /// 安装/升级 AGS 本机治理内核 (五段链路第 1 段). Install/upgrade the global
    /// AGS governance kernel so AGS is visible to host agents. Plan-only by
    /// default; --yes writes and emits a receipt, then guides Agent governance.
    Setup {
        /// Target runtime home (default: $AGS_HOME or ~/.ags/runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Write setup files. Without --yes, setup prints a plan only.
        #[arg(long)]
        yes: bool,
        /// Overwrite differing files after writing .bak.<timestamp> backups.
        #[arg(long)]
        force: bool,
        /// Register AGS MCP servers in Claude Code user config after setup.
        #[arg(long)]
        register_claude: bool,
        /// Capability Route enrollment mode written to machine-local runtime
        /// evidence (off | suite-only | adopted | review-all). Omit to PRESERVE
        /// the machine's existing enrollment (suite-only when none) — this keeps
        /// `ags setup --yes` idempotent. Plan-only without --yes. AGS records the
        /// choice only — never auto-installs or logs in.
        #[arg(long = "capability-route",
              value_parser = ["off", "suite-only", "adopted", "review-all"])]
        capability_route: Option<String>,
        /// Print plan only, even if --yes is omitted.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// 初始化项目内 AGS 能力入口 (五段链路第 4 段). Onboard the current project
    /// into AGS governance: entry files, project profile, protocol, portable
    /// validator, and a first-class memory capsule. Runs after global
    /// setup → agents → skill.
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
    /// 诊断并修复 AGS 全局链路. Diagnose the AGS global pipeline (runtime /
    /// agents / skills / hooks / MCP / project init / memory capsule / update
    /// drift / receipts). Read-only by default; --fix runs only safe whitelisted
    /// repairs.
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

    // ── Global agent governance (五段链路第 2 段) ────────────────────
    /// 纳管本机 Agent 宿主. Govern local Agent hosts (Claude Code / Codex /
    /// Cursor / Tencent Agent): scan, plan AGS MCP onboarding (advise-only),
    /// and verify host visibility. ags_preflight is the governance entry.
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },

    // ── Cross-Agent capability layer (+ hidden M5 registry compat) ────
    /// 跨 Agent 能力可见性与入口同步底层/兼容层（前台主入口是 `ags skill`）.
    /// Cross-Agent capability layer: inventory / install / sync / verify host
    /// visibility and entry plans (over the shared skill-governance console).
    /// Hidden `list`/`show` remain the M5 internal suite-capability registry.
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

    // ── Global skill governance (五段链路第 3 段) ─────────────────
    /// 纳管本机技能本体（前台主入口）. Local skill-body governance home:
    /// inventory / dedupe / update / sync / verify (+ hidden compat scan /
    /// check / propose / upstream). Dry-run by default; --apply writes only the
    /// thin index under the single guard. `ags capability` is the underlying layer.
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

    // ── Global update maintenance (五段链路第 5 段) ──────────────────
    /// 更新 AGS 全局内核/runtime/Agent 注册/技能/已纳管项目/public-safe 投影.
    /// Unified update across core/runtime/agents/skills/projects/public lanes.
    /// Default plan-only; --apply writes AGS-owned dirs and emits a receipt.
    Update {
        #[command(subcommand)]
        action: UpdateAction,
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
    /// stdio transport only.
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

        /// Stop after gate check; exit 0 if allow, 1 if stop.
        #[arg(long, default_value_t = false)]
        check_only: bool,

        /// Full pipeline, output structured launch plan, do not execute.
        #[arg(long, default_value_t = false)]
        dry_run: bool,

        /// Write-approval audit/hint signal for the policy resolver; may act as
        /// the M9 generic-adapter capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,

        /// Structured current-task approval signal from the live request
        /// (audit/hint only — task level does not downgrade the permission mode).
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,

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
        /// Write-approval audit/hint signal; may act as the M9 generic-adapter
        /// capability override.
        #[arg(long, default_value_t = false)]
        approve_writes: bool,
        /// Structured current-task approval signal from the live request
        /// (audit/hint only — task level does not downgrade the permission mode).
        #[arg(long, default_value_t = false)]
        current_task_approval: bool,
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
pub(in crate::cli) fn parse_target(s: &str) -> Result<(String, PathBuf), String> {
    let (name, path) = s.split_once('=').ok_or_else(|| {
        format!("invalid target format: '{s}'. Expected NAME=PATH (e.g. stable=/path/to/stable)")
    })?;
    Ok((name.to_string(), PathBuf::from(path)))
}

// ── Shared dispatch functions (used by both M1 and M0 commands) ───────────
