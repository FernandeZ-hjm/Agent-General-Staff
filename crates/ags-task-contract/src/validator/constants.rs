//! Allowed-value sets, protected paths, and intent/keyword constants.

// ── Allowed-value sets ─────────────────────────────────────────────────

pub(crate) const VALID_EXECUTORS: &[&str] = &["Codex", "Claude Code", "Cursor", "OMP", "Other"];
pub(crate) const VALID_RUNTIME_ADAPTERS: &[&str] =
    &["codex-local", "claude-code", "cursor", "omp", "generic"];
pub(crate) const VALID_EXECUTION_SURFACES: &[&str] = &[
    "local-workspace",
    "cli",
    "ide",
    "web",
    "remote-control",
    "background-agent",
];
pub(crate) const VALID_PERMISSION_MODES: &[&str] = &["plan-only", "execute-and-verify"];
pub(crate) const VALID_PARALLELISM: &[&str] = &[
    "none",
    "limited",
    "parallel",
    "subagent",
    "worktree",
    "multi-session",
    "agent-team",
];
pub(crate) const VALID_TASK_LEVELS: &[&str] = &["Light", "Medium", "Heavy"];
/// Allowed `Execution effort` values. `unknown` is the absent default.
pub(crate) const VALID_EXECUTION_EFFORT: &[&str] =
    &["low", "normal", "high", "exhaustive", "unknown"];
pub(crate) const VALID_WORKFLOW_AUTHORITY: &[&str] =
    &["none", "within-card", "plan-only", "allowed"];
pub(crate) const VALID_HANDOFF_SOURCES: &[&str] =
    &["explicit-handoff", "host-plan-mode", "existing-card"];
/// Allowed `子任务编排` (subtask orchestration) mode values. `none` = no
/// orchestration declared; `optional` / `required` declare splittable subtask
/// structure. A non-`none` mode requires a delegation-capable Parallelism and a
/// non-`none` Workflow authority (checked in the execution authority gate). The
/// slot only DECLARES splittable structure — actual subagent/workflow ignition is
/// translated by the claude-code adapter / runner from the resolved policy.
pub(crate) const VALID_SUBTASK_ORCHESTRATION_MODES: &[&str] = &["none", "optional", "required"];

/// Map Executor to its required Runtime adapter.
pub(crate) fn expected_adapter(executor: &str) -> Option<&'static str> {
    match executor {
        "Codex" => Some("codex-local"),
        "Claude Code" => Some("claude-code"),
        "Cursor" => Some("cursor"),
        "OMP" => Some("omp"),
        "Other" => Some("generic"),
        _ => None,
    }
}

// ── Protected paths ────────────────────────────────────────────────────

/// Standalone boundary terms that identify protected assets.
/// Each term is matched with word-boundary guards so short tokens like
/// `hook` don't match `hooks` (which has its own entry) or `shook`.
pub(crate) const PROTECTED_BOUNDARY_TERMS: &[&str] = &[
    // Repository-role boundary names. Matching requires a trailing path/text
    // boundary, so `agent-governance-suite-private-rust` remains distinct.
    "agent-governance-suite-private",
    "agent-governance-suite-stable",
    "private suite",
    "stable suite",
    // Governance files
    "AGENTS.md",
    "CLAUDE.md",
    "context-capsule.md",
    // Protocol
    "protocol/",
    // Hook / memory / bootstrap infrastructure
    "hook",
    "hooks",
    "memory",
    "bootstrap",
    // Boundary markers
    "public boundary",
    "private boundary",
    "stable boundary",
    "public/private",
    "private/stable",
];

/// Keywords that indicate modification intent.
pub(crate) const MODIFICATION_KEYWORDS: &[&str] = &[
    "修改",
    "覆盖",
    "删除",
    "同步",
    "迁移",
    "修复",
    "实现",
    "升级",
    "重写",
    "替换",
    "实施",
    "执行",
    "应用",
    "调整",
    "生成",
    "创建",
    "写入",
    "部署",
    "安装",
    "发布",
    "fix",
    "implement",
    "modify",
    "change",
    "update",
    "delete",
    "remove",
    "replace",
    "refactor",
    "rewrite",
    "patch",
    "deploy",
    "install",
    "publish",
    "sync",
];

/// Values considered too weak for `目标：`.
pub(crate) const WEAK_GOAL_VALUES: &[&str] = &[
    "test",
    "todo",
    "tbd",
    "n/a",
    "none",
    "later",
    "无",
    "待定",
    "暂无",
    "未定",
    "无目标",
    "暂无目标",
    "未明确",
    "待明确",
    "待补充",
    "以后再说",
];
