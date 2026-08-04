//! Front-stage / facade command action sub-enums.

use clap::Subcommand;
use std::path::PathBuf;

/// Unified public onboarding lifecycle.
#[derive(Subcommand)]
pub(crate) enum OnboardingAction {
    /// Assess the active machine/project and emit a deterministic action plan.
    Plan {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Show the current readiness states without applying anything.
    Status {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Apply exactly one reviewed item from the current deterministic plan.
    Apply {
        #[arg(long)]
        item: String,
        /// Exact plan hash printed by `ags onboarding plan`. Required so the
        /// reviewed packaged manifest and current machine facts cannot drift
        /// between review and apply.
        #[arg(long)]
        plan_hash: String,
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        /// Required confirmation. There is no batch or implicit apply mode.
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Re-assess readiness and fail when a required component is not ready.
    Verify {
        #[arg(long, default_value = ".")]
        target: PathBuf,
        #[arg(long, default_value = "codex")]
        host: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

/// Cross-Agent host capability layer.
#[derive(Subcommand)]
pub(crate) enum CapabilityAction {
    /// Cross-Agent capability inventory with per-host thin-index visibility.
    ///
    /// Unified view of skills + governed MCPs + CLI-backed capabilities and
    /// whether each is visible to each host. Read-only.
    Inventory {
        /// Host to scope visibility to (repeatable). Default: claude-code + codex + codebuddy-code.
        #[arg(long = "host")]
        host: Vec<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify cross-Agent host visibility and required capability coverage (read-only).
    ///
    /// Resolves the installed AGS capability authority independently of the
    /// current project directory. Missing required registry parents remain in
    /// the expected set and fail closed. `ags skill verify` is the public
    /// human-facing facade for the same host verification. Claude Code, Codex,
    /// Cursor, OMP, and CodeBuddy-Code are supported capability identities.
    Verify {
        /// Host to verify: claude-code | codex | cursor | omp | codebuddy-code
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (post-apply gate).
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Derive the machine-local Skill/MCP capability snapshot + attestation hash.
    ///
    /// Captures the strict intersection of governed routable skills that are
    /// healthy and visible to one active host, plus a deterministic
    /// `snapshot_hash`. The registry stays authoritative for what MAY route.
    /// With `--write` the snapshot is written to the machine-local runtime home
    /// (never tracked, never published).
    Snapshot {
        /// Active host whose routable skill table is captured.
        #[arg(long = "host", default_value = "codex")]
        host: String,
        /// Project root path (default: current directory).
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Write the snapshot JSON to the machine-local runtime home.
        #[arg(long)]
        write: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Static skill catalog commands.
#[derive(Subcommand)]
pub(crate) enum SkillAction {
    /// Plan or explicitly adopt one audited local Skill directory into the
    /// machine-private registry. The default is plan-only.
    Adopt {
        /// Local Skill directory or its SKILL.md file.
        source: PathBuf,
        /// Optional machine-private YAML routing metadata. It is audited and
        /// hash-bound to the reviewed plan, but never copied into AGS Git.
        #[arg(long)]
        metadata: Option<PathBuf>,
        /// Target host id; repeatable. Empty or `all` selects all supported hosts.
        #[arg(long = "host")]
        host: Vec<String>,
        /// Exact hash from the reviewed plan. Required with --yes.
        #[arg(long)]
        plan_hash: Option<String>,
        /// Confirm the reviewed machine-private writes.
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan or explicitly remove one machine-private adopted Skill. Immutable
    /// body revisions remain available for recoverable rollback.
    Remove {
        skill_id: String,
        /// Exact hash from the reviewed removal plan. Required with --yes.
        #[arg(long)]
        plan_hash: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Inspect private registry, immutable body, host indexes, and active snapshots.
    Status {
        skill_id: String,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify host visibility against the installed static catalog.
    Verify {
        #[arg(long, default_value = "claude-code")]
        host: String,
        #[arg(long)]
        strict: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Read the canonical skill inventory.
    Inventory {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Agent host governance — five-segment stage 2 (纳管本机 Agent 宿主).
#[derive(Subcommand)]
pub(crate) enum AgentsAction {
    /// Scan local Agent hosts and AGS MCP registration (read-only).
    /// 盘点本机 Agent 宿主与 AGS MCP 注册状态。只读，不写任何配置。
    Scan {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan host onboarding; --apply installs AGS-owned memory lifecycle wiring.
    ///
    /// Default dry-run. `--apply` writes only AGS-owned Claude Code, Codex,
    /// Cursor, CodeBuddy, or OMP workspace lifecycle adapters. External MCP
    /// registrars remain advice-only.
    Govern {
        /// Limit to one host id (claude-code|codex|omp|cursor|workbuddy|codebuddy-code).
        #[arg(long)]
        agent: Option<String>,
        /// Canonical workspace to inspect or receive the adapter.
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Install the selected supported host's AGS memory lifecycle adapter.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify capability visibility plus the host-native memory lifecycle.
    /// 校验宿主能力可见性及原生记忆闭环。
    Verify {
        /// Host to verify: claude-code | codex | omp | codebuddy-code | cursor
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (post-apply gate).
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Update lane selector. Only `core` / `runtime` auto-execute locally; the rest
/// are plan + advice + receipt only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum UpdateLane {
    Core,
    Runtime,
    Agents,
    Skills,
    Projects,
    Public,
}
/// Unified update — five-segment stage 5 (统一更新). check/plan read-only;
/// apply/repair-local write only AGS-owned dirs under --apply.
#[derive(Subcommand)]
pub(crate) enum UpdateAction {
    /// Read-only drift report across all six lanes. 只读六 lane 漂移报告。
    Check {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Structured six-lane plan + suggested commands + receipt outline. 结构化计划。
    Plan {
        /// Limit to one lane.
        #[arg(long, value_enum)]
        lane: Option<UpdateLane>,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Execute local lanes (core build, runtime rewrite, managed-project AGS
    /// projection refresh); agents/skills/public stay plan+advice. Requires
    /// --apply; risk follows the selected lane.
    /// 执行本机 lane；其余仅出计划+建议。需显式 --apply。
    Apply {
        #[arg(long, value_enum)]
        lane: Option<UpdateLane>,
        #[arg(long)]
        target: Option<PathBuf>,
        /// Confirm writes. Without it, dry-run plan only.
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        force: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Re-check post-update state: version, runtime, host visibility.
    /// 复核更新后状态。--strict 有漂移即非零退出。
    Verify {
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Repair local runtime/agent/skill visibility drift only. No git pull, no
    /// cargo build. 只修本机可见性漂移：重写 AGS 自有 runtime/thin-index。
    RepairLocal {
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        force: bool,
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}

// ── Top-level Commands ────────────────────────────────────────────────────
