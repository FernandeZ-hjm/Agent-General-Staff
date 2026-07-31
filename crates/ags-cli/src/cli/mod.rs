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
        /// Replace differing AGS-owned files atomically.
        #[arg(long)]
        force: bool,
        /// Register AGS MCP servers in Claude Code user config after setup.
        #[arg(long)]
        register_claude: bool,
        /// Approved workspace lifecycle hosts: comma-separated ids, `detected`,
        /// or `none`. Required on first write-mode setup.
        #[arg(long)]
        lifecycle_hosts: Option<String>,
        /// Print plan only, even if --yes is omitted.
        #[arg(long)]
        dry_run: bool,
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },

    /// Assess, plan, apply one confirmed item, and verify the public AGS
    /// onboarding profile. Machine-local and non-public capabilities are not
    /// included.
    Onboarding {
        #[command(subcommand)]
        action: OnboardingAction,
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
        /// `.git/info/exclude`; `shared` keeps them committed.
        #[arg(long, default_value = "local", value_parser = ["local", "shared"])]
        mode: String,
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
        /// Target runtime home (default: $AGS_HOME or ~/.ags/private-runtime).
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
        /// Target runtime home (default: $AGS_HOME or ~/.ags/private-runtime).
        #[arg(long)]
        target: Option<PathBuf>,
        /// Required confirmation for write-mode install.
        #[arg(long)]
        yes: bool,
        /// Replace differing AGS-owned files atomically.
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
    /// 诊断 AGS 内核/runtime 与目标项目纳管链路. Diagnose the installed AGS
    /// kernel/runtime plus the target's AGS onboarding projection (agents /
    /// skills / hooks / MCP / project init / memory capsule / update drift /
    /// receipts), including required third-party capability host routing.
    /// Doctor never runs project source formatting, tests, or builds;
    /// those belong to `ags verify`. Read-only by default; --fix runs only safe
    /// whitelisted repairs.
    Doctor {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Perform safe repair actions (default: read-only diagnosis only).
        #[arg(long)]
        fix: bool,
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
    /// OMP / Cursor / Tencent Agent): scan, plan AGS MCP onboarding, explicitly
    /// apply AGS-owned memory adapters, and verify host visibility/lifecycle.
    /// External MCP registration remains advice-only; ags_preflight is the
    /// governance entry.
    Agents {
        #[command(subcommand)]
        action: AgentsAction,
    },

    // ── Cross-Agent capability layer ──────────────────────────────────
    /// 跨 Agent 能力可见性与入口同步底层/兼容层（前台主入口是 `ags skill`）.
    /// Cross-Agent capability layer: inventory / static snapshot / verify host
    /// visibility and entry plans (over the shared skill-governance console).
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
    /// Receipt-backed project memory operations.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Unified governed-host lifecycle adapter.
    Host {
        #[command(subcommand)]
        action: HostAction,
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
    /// Inspect the installed static skill catalog and refresh metadata explicitly.
    Skill {
        /// Output format: text (default) or json.
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
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

    // ── Release operations ─────────────────────────────────────────
    /// Release verification and packaging — dry-run only
    #[command(hide = true)]
    Release {
        #[command(subcommand)]
        action: ReleaseAction,
    },
    // ── MCP operations ─────────────────────────────────────────
    /// Start AGS MCP server — expose governance tools/resources/prompts
    /// to MCP hosts (Tencent Agent, Codex, OMP, Cursor, Claude Code). V1 supports
    /// stdio transport only. Third-party MCP servers remain host-owned.
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
    /// Prepare a task card for host-owned execution through the gate-first pipeline.
    ///
    /// Flow: validate → gate → policy → adapter resolve → launch plan.
    /// The runner ONLY consumes resolved execution policy — it never reads
    /// raw task-card fields to decide permissions, execution_topology, or launch args.
    ///
    /// --check-only stops after gate check. --dry-run outputs the full launch
    /// plan. Without flags, returns `host_execution_required` after the same
    /// checks. This command never launches, verifies, or completes the task.
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
        /// (audit/hint only — task level does not downgrade the execution mode).
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
        /// Verification scope: local, release, or promotion
        #[arg(long, default_value = "local", value_parser = ["local", "release", "promotion"])]
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
        /// Explicit public worktree for `--scope promotion`.
        #[arg(long)]
        public_root: Option<PathBuf>,
        #[command(subcommand)]
        action: Option<VerifyAction>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_lifecycle_accepts_every_platform_backed_adapter() {
        for lifecycle in ags_host_integration::lifecycle_specs() {
            let parsed = Cli::try_parse_from([
                "ags",
                "host",
                "lifecycle",
                "--event",
                "session-start",
                "--host",
                lifecycle.host_id,
            ]);
            if let Err(error) = parsed {
                panic!("{}: {error}", lifecycle.host_id);
            }
        }
    }
}

// ── Shared dispatch functions (used by both M1 and M0 commands) ───────────
