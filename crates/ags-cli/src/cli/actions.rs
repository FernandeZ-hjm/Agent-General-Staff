//! Front-stage / facade command action sub-enums.

use clap::Subcommand;
use std::path::PathBuf;

/// Cross-Agent capability layer + (hidden) M5 suite-capability registry.
///
/// `inventory` / `verify` / `install` / `sync` operate on **cross-Agent host
/// capabilities** (skills + MCP + CLI-backed) over the shared skill-governance
/// console: per-host thin-index visibility and entry plans. The hidden
/// `list` / `show` are the M5 **internal suite-capability registry**
/// (`rust:*` / `policy:*` discovered inside a target repo) — a different,
/// repo-scoped concern kept for MCP/CI compatibility.
#[derive(Subcommand)]
pub(crate) enum CapabilityAction {
    /// (M5, hidden) List all discovered suite capabilities in a repo.
    #[command(hide = true)]
    List {
        /// Project root path (default: current directory)
        #[arg(long, default_value = ".")]
        target: PathBuf,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// (M5, hidden) Show a suite capability by ID.
    #[command(hide = true)]
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
    /// Cross-Agent capability inventory with per-host thin-index visibility.
    ///
    /// Unified view of skills + governed MCPs + CLI-backed capabilities and
    /// whether each is visible to each host. Read-only.
    Inventory {
        /// Host to scope visibility to (repeatable). Default: claude-code + codex.
        #[arg(long = "host")]
        host: Vec<String>,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify cross-Agent host visibility for a host (read-only).
    ///
    /// Canonical home for host-visibility checks (`ags skill verify` remains a
    /// compatibility alias). Claude Code / Codex supported; Cursor reserved.
    Verify {
        /// Host to verify: claude-code | codex (cursor reserved)
        #[arg(long, default_value = "claude-code")]
        host: String,
        /// Gate mode: exit nonzero unless status is "ok" (post-apply gate).
        #[arg(long)]
        strict: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Plan (or, with `--apply`, perform) a single capability's cross-host entry.
    ///
    /// AGS-owned skill thin-index writes go through the confirmation guard
    /// (with backup); MCP / CLI-backed registration is advised per host
    /// (Claude Code, Codex), never run by AGS.
    Install {
        /// Capability name (skill / MCP / CLI-backed).
        #[arg(long = "capability")]
        capability: String,
        /// Confirm and perform AGS-owned writes. Without it, dry-run only.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Batch cross-host entry plan for all adopted/governed capabilities.
    ///
    /// With `--apply`, performs AGS-owned skill thin-index writes through the
    /// guard; MCP / CLI-backed registration stays advised-only.
    Sync {
        /// Confirm and perform AGS-owned writes. Without it, dry-run only.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
}
/// Skill-body governance subcommands.
///
/// `ags skill` is the skill-body governance face. `scan` / `check` /
/// `inventory` / `upstream` are read-only; `propose --apply` writes ONLY
/// AGS-owned per-host thin-index entries (with backup) through the console's
/// single mutation guard and never runs external installers. `verify --host`
/// reports cross-Agent host visibility and is the seam slated to move under
/// the `ags capability` command layer in a future release.
#[derive(Subcommand)]
pub(crate) enum SkillAction {
    /// (hidden compat) Scan the suite manifest and governance files for status.
    ///
    /// Reports available, missing, disabled, and degraded skills with
    /// profile information (required/optional/personal).
    #[command(hide = true)]
    Scan {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// (hidden compat) Validate governance YAML files for schema compliance.
    ///
    /// Checks parseability, cross-references adoption log with manifest,
    /// and reports schema version consistency across files.
    #[command(hide = true)]
    Check {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// (hidden compat) Propose a management action — dry-run unless `--apply`.
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
    /// Verify cross-Agent host visibility for a host (read-only).
    ///
    /// Claude Code and Codex: check `~/.claude/skills` / `~/.codex/skills`
    /// `SKILL.md` (symlink-aware) and `claude mcp list` / `codex mcp list`.
    /// Cursor is reserved (unsupported in this version; model fields are
    /// stable). Degrades, never panics, when a host CLI is unavailable.
    ///
    /// This is the cross-Agent visibility check; it is also available as
    /// `ags capability verify` (the canonical home). It remains here as a
    /// compatibility entry.
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
    Inventory {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
        /// Also write a Markdown report to governance/skills-inventory.md
        #[arg(long)]
        write: bool,
    },
    /// Detect duplicate skills/hooks across the canonical store; plan quarantine.
    /// 检测 canonical store 重复技能/hook，产出备份+隔离计划。默认 dry-run；
    /// --apply 经守约把非 keeper 副本隔离到 governance/backups（canonical 本体绝不删）。
    Dedupe {
        /// Confirm and quarantine non-keeper copies (reversible; never deletes).
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Incremental, auditable upstream update proposal (check/plan only).
    /// 增量、可审计的上游更新提案。仅 check/plan，不自动 pull、不覆盖本地 canonical。
    Update {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Batch cross-host thin-index distribution. 为已纳管能力批量分发 thin-index。
    /// 默认 dry-run；--apply 写入。与 `ags capability sync` 同底层。
    Sync {
        /// Confirm and perform AGS-owned thin-index writes. Without it, dry-run.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// (hidden compat alias of `update`) Upstream update proposal (stub).
    ///
    /// Reads manifests/skills-registry.yaml and reports which suite skills
    /// watch which upstream comparison source plus declared candidates. No
    /// crawl/clone/fetch is performed and no concrete diff is proposed; local
    /// suite files remain canonical. Real crawl_then_diff_proposal is deferred
    /// to a future task.
    #[command(hide = true)]
    Upstream {
        /// Output format: text (default) or json
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
    /// Plan AGS MCP onboarding (advise-only). 产出 AGS MCP 纳管计划（仅建议）。
    ///
    /// Default dry-run; with --apply shows the same selectable confirmation
    /// surface and still writes nothing. AGS never writes host config, never
    /// runs external registrars (claude mcp add / codex mcp / lark-cli), and
    /// never writes receipts for advice. 默认 dry-run；--apply 只切到确认视图，
    /// 不写 receipt，不写宿主配置。
    Govern {
        /// Limit to one host id (claude-code|codex|cursor|workbuddy|codebuddy-code).
        #[arg(long)]
        agent: Option<String>,
        /// Confirm-view only: print selectable host/tool registration advice.
        /// AGS still never writes host config or receipts for advice.
        #[arg(long)]
        apply: bool,
        /// Output format: text (default) or json
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Verify a host's AGS visibility (thin-index + AGS MCP).
    /// 校验某宿主的 AGS 可见性（thin-index + AGS MCP）。
    Verify {
        /// Host to verify: claude-code | codex (cursor reserved)
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
impl UpdateLane {
    pub(crate) fn all() -> [UpdateLane; 6] {
        use UpdateLane::*;
        [Core, Runtime, Agents, Skills, Projects, Public]
    }
    pub(crate) fn id(&self) -> &'static str {
        match self {
            UpdateLane::Core => "core",
            UpdateLane::Runtime => "runtime",
            UpdateLane::Agents => "agents",
            UpdateLane::Skills => "skills",
            UpdateLane::Projects => "projects",
            UpdateLane::Public => "public",
        }
    }
    /// True only for lanes AGS may auto-execute locally (core / runtime).
    pub(crate) fn auto_executes_locally(&self) -> bool {
        matches!(self, UpdateLane::Core | UpdateLane::Runtime)
    }
    pub(crate) fn risk_tier(&self) -> &'static str {
        match self {
            UpdateLane::Core | UpdateLane::Public => "heavy",
            UpdateLane::Runtime | UpdateLane::Skills => "medium",
            UpdateLane::Agents | UpdateLane::Projects => "advice",
        }
    }
}
/// Unified update — five-segment stage 5 (统一更新). check/plan read-only;
/// apply/repair-local write only AGS-owned dirs under --apply; rollback plan-only.
#[derive(Subcommand)]
pub(crate) enum UpdateAction {
    /// Read-only drift report across all six lanes. 只读六 lane 漂移报告。
    Check {
        #[arg(long, default_value = "text", value_parser = ["text", "json"])]
        format: String,
    },
    /// Lazy, post-task, calendar-throttled update notifier. Reads runtime state,
    /// checks a public release/tag source at most once per 7 local days, and
    /// reports whether a newer AGS exists. Fails silently; never auto-updates.
    /// JSON is the hook/runner authority. 任务结束后懒检查更新。
    Notify {
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
    /// Execute local lanes (git pull + cargo build + rewrite AGS-owned runtime);
    /// agents/projects/public stay plan+advice. Requires --apply (Heavy).
    /// 执行本机 lane；其余仅出计划+建议。需 --apply（Heavy 批准）。
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
    /// Plan-only rollback umbrella (setup/runtime, agent govern, skill dedupe,
    /// init overlay). 仅出回滚计划，不改任何文件。
    Rollback {
        #[arg(long, default_value = "all", value_parser = ["runtime", "agents", "skills", "projects", "all"])]
        scope: String,
        #[arg(long)]
        target: Option<PathBuf>,
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
