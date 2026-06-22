//! Allowed-value sets, protected paths, and intent/keyword constants.

// ── Allowed-value sets ─────────────────────────────────────────────────

pub(crate) const VALID_EXECUTORS: &[&str] = &["Codex", "Claude Code", "Cursor", "Other"];
pub(crate) const VALID_RUNTIME_ADAPTERS: &[&str] =
    &["codex-local", "claude-code", "cursor", "generic"];
pub(crate) const VALID_EXECUTION_SURFACES: &[&str] = &[
    "local-workspace",
    "cli",
    "ide",
    "web",
    "remote-control",
    "background-agent",
];
pub(crate) const VALID_PERMISSION_MODES: &[&str] = &[
    "read-only",
    "plan-only",
    "edit-with-confirmation",
    "execute-and-verify",
];
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
pub(crate) const VALID_EXECUTION_EFFORT: &[&str] = &["normal", "ultracode", "unknown"];
pub(crate) const VALID_WORKFLOW_AUTHORITY: &[&str] =
    &["none", "within-card", "plan-only", "allowed"];

/// Explicit task-card skill tags that have been retired from the active skill
/// surface. Empty replacement means the old tag was deleted with no successor.
pub(crate) const RETIRED_SKILL_TAGS: &[(&str, &str)] = &[
    ("auto-brainstorm", "grill-with-docs"),
    ("auto-debug", "diagnosing-bugs"),
    ("auto-verify", "verification-before-completion"),
    ("tdd", "test-driven-development"),
    ("diagnose", "diagnosing-bugs"),
    ("zoom-out", "codebase-design"),
    ("caveman-review", "review"),
    ("code-review", "review"),
    ("verify", "verification-before-completion"),
    ("commit", ""),
    ("caveman-commit", ""),
];

/// Map Executor to its required Runtime adapter.
pub(crate) fn expected_adapter(executor: &str) -> Option<&'static str> {
    match executor {
        "Codex" => Some("codex-local"),
        "Claude Code" => Some("claude-code"),
        "Cursor" => Some("cursor"),
        "Other" => Some("generic"),
        _ => None,
    }
}

// ── Protected paths ────────────────────────────────────────────────────

/// Full absolute paths that indicate protected assets.
/// Matched with trailing-boundary check to avoid prefix confusion
/// (e.g. `example-private-suite` does NOT match
/// `example-private-suite-rust`).
pub(crate) const PROTECTED_PATHS: &[&str] = &[
    "/Volumes/Projects/example-private-suite",
    "/Volumes/Projects/example-stable-suite",
    "~/.agents/memory/projects/example-private-suite/context-capsule.md",
];

/// Standalone boundary terms that identify protected assets.
/// Each term is matched with word-boundary guards so short tokens like
/// `hook` don't match `hooks` (which has its own entry) or `shook`.
pub(crate) const PROTECTED_BOUNDARY_TERMS: &[&str] = &[
    // Short-form repo names (without /Volumes/AI Project/ prefix)
    "example-private-suite",
    "example-stable-suite",
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

// ── Required fields (format check) ──────────────────────────────────────

/// Required fields for the canonical task card — the classic fixed skeleton
/// defined in `protocol/task-card-template.md`. This is the single legal
/// task-card format; the compact task-card format has been removed.
/// Excludes `## 任务卡` (checked separately by the first-line rule).
pub(crate) const REQUIRED_FIELDS: &[&str] = &[
    "读取并遵守：",
    "Executor:",
    "Runtime adapter:",
    "Execution surface:",
    "Permission mode:",
    "Parallelism:",
    "任务级别",
    "Review gate:",
    "任务：",
    "背景：",
    "项目画像：",
    "记忆胶囊：",
    "任务存档：",
    "目标文件夹路径：",
    "相关路径：",
    "本次任务相关文件：",
    "目标：",
    "非目标：",
    "验证：",
    "Verification gate:",
    "交付：",
];
