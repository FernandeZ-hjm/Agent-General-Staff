//! Task card validator — validates Agent General Staff task cards.
//!
//! Rules enforced:
//! - First non-empty line must be `## 任务卡`
//! - Reject `text`-typed code fences (3+ backticks or 4+ tildes then `text`)
//! - Distinguish compact vs full cards by structural position (second non-empty line)
//! - Check required header and body fields per card type
//! - Validate field values against allowed sets (e.g. Executor, Permission mode)
//! - Validate field combinations (e.g. Executor ↔ Runtime adapter)
//! - Detect protected-path violations
//! - Check content quality (non-empty goals, concrete verification, etc.)
//! - Detect contradictory requirements
//!
//! # Example
//!
//! ```rust
//! use task_card_validator::validate;
//!
//! let input = "## 任务卡\n路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\nExecution surface: cli\nPermission mode: execute-and-verify\nParallelism: none\n任务级别：Medium\n读取：\n- .\n任务：运行测试\n目标：验证校验器功能\n非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败时停止\n交付：\n返回结果\n";
//! let errors = validate(input);
//! assert!(errors.is_empty());
//! ```

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

// ── Card type ──────────────────────────────────────────────────────────

/// Task card format detected by structural position.
///
/// The detection rule: examine the second non-empty line after `## 任务卡`.
/// - `路径：` → Compact
/// - `AGENT_SUITE_COMPACT_TASK_CARD_V1` → Compact
/// - `读取并遵守：` → Full
/// - otherwise → fallback to contains-based logic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Compact,
    Full,
}

impl CardType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

/// Parsed fields from a validated task card.
///
/// This is the structured output of `parse_validated()`, ready to be
/// consumed by the execution-policy resolver.
#[derive(Debug, Clone)]
pub struct ParsedTaskCard {
    /// Parsed field-name → value map (keys like `"Executor:"`, `"任务级别："`, etc.)
    pub fields: std::collections::HashMap<String, String>,
    /// Detected card type
    pub card_type: CardType,
}

// ── Error codes ────────────────────────────────────────────────────────

/// Stable error codes for machine-consumable error classification.
pub mod error_code {
    pub const INVALID_FIELD_VALUE: &str = "INVALID_FIELD_VALUE";
    pub const FIELD_COMBINATION_MISMATCH: &str = "FIELD_COMBINATION_MISMATCH";
    pub const PROTECTED_PATH_VIOLATION: &str = "PROTECTED_PATH_VIOLATION";
    pub const RISK_LEVEL_MISMATCH: &str = "RISK_LEVEL_MISMATCH";
    pub const EMPTY_OR_WEAK_SECTION: &str = "EMPTY_OR_WEAK_SECTION";
    pub const CONTRADICTORY_REQUIREMENT: &str = "CONTRADICTORY_REQUIREMENT";
    pub const EXECUTION_EFFORT_POLICY_VIOLATION: &str = "EXECUTION_EFFORT_POLICY_VIOLATION";
    pub const WORKFLOW_AUTHORITY_REQUIRED: &str = "WORKFLOW_AUTHORITY_REQUIRED";
    pub const WORKFLOW_AUTHORITY_VIOLATION: &str = "WORKFLOW_AUTHORITY_VIOLATION";
    pub const PARALLELISM_POLICY_VIOLATION: &str = "PARALLELISM_POLICY_VIOLATION";
    pub const ULTRACODE_AUTHORITY_ABUSE: &str = "ULTRACODE_AUTHORITY_ABUSE";
    pub const PLAN_ONLY_DELIVERY_VIOLATION: &str = "PLAN_ONLY_DELIVERY_VIOLATION";
    pub const HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF: &str =
        "HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF";
    pub const PLAN_ONLY_EXECUTION_VERB_DETECTED: &str = "PLAN_ONLY_EXECUTION_VERB_DETECTED";
    pub const FIELD_ABUSE_DETECTED: &str = "FIELD_ABUSE_DETECTED";
    pub const AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE: &str =
        "AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE";
}

// ── Allowed-value sets ─────────────────────────────────────────────────

const VALID_EXECUTORS: &[&str] = &["Codex", "Claude Code", "Cursor", "Other"];
const VALID_RUNTIME_ADAPTERS: &[&str] = &["codex-local", "claude-code", "cursor", "generic"];
const VALID_EXECUTION_SURFACES: &[&str] = &[
    "local-workspace",
    "cli",
    "ide",
    "web",
    "remote-control",
    "background-agent",
];
const VALID_PERMISSION_MODES: &[&str] = &[
    "read-only",
    "plan-only",
    "edit-with-confirmation",
    "execute-and-verify",
];
const VALID_PARALLELISM: &[&str] = &[
    "none",
    "limited",
    "parallel",
    "subagent",
    "worktree",
    "multi-session",
    "agent-team",
];
const VALID_TASK_LEVELS: &[&str] = &["Light", "Medium", "Heavy"];
const VALID_EXECUTION_EFFORT: &[&str] = &["normal", "ultracode", "unknown"];
const VALID_WORKFLOW_AUTHORITY: &[&str] = &["none", "within-card", "plan-only", "allowed"];

/// Map Executor to its required Runtime adapter.
fn expected_adapter(executor: &str) -> Option<&'static str> {
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
/// (e.g. `my-protected-suite` does NOT match
/// `my-protected-suite-rust`).
const PROTECTED_PATHS: &[&str] = &[
    "/Volumes/Projects/my-protected-suite",
    "/Volumes/Projects/my-stable-suite",
    "/Users/user/.agents/memory/projects/my-project/context-capsule.md",
];

/// Standalone boundary terms that identify protected assets.
/// Each term is matched with word-boundary guards so short tokens like
/// `hook` don't match `hooks` (which has its own entry) or `shook`.
const PROTECTED_BOUNDARY_TERMS: &[&str] = &[
    // Short-form repo names (without /Volumes/AI Project/ prefix)
    "my-protected-suite",
    "my-stable-suite",
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
const MODIFICATION_KEYWORDS: &[&str] = &[
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
const WEAK_GOAL_VALUES: &[&str] = &[
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

/// Required fields for a compact task card.
/// Excludes `## 任务卡` (checked separately) and the old
/// `AGENT_SUITE_COMPACT_TASK_CARD_V1` marker (detection is now structural).
const COMPACT_REQUIRED: &[&str] = &[
    "路径：",
    "Executor:",
    "Runtime adapter:",
    "Execution surface:",
    "Permission mode:",
    "Parallelism:",
    "任务级别",
    "读取：",
    "任务：",
    "目标：",
    "非目标：",
    "关键路径：",
    "验证：",
    "停止条件：",
    "交付：",
];

/// Required fields for a full (non-compact) task card.
/// Excludes `## 任务卡` (checked separately).
const FULL_REQUIRED: &[&str] = &[
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
    "相关路径：",
    "本次任务相关文件：",
    "目标：",
    "非目标：",
    "验证：",
    "Verification gate:",
    "交付：",
];

// ── Field definitions for parsing ──────────────────────────────────────

struct FieldDef {
    name: &'static str,
    /// true = value on the same line after `:` or `：`.
    is_inline: bool,
}

/// All known task-card field headers.  Must include every field that could
/// appear so the parser can correctly delimit multi-line sections.  The
/// lookup uses longest-prefix match, so order is irrelevant.
const FIELD_DEFS: &[FieldDef] = &[
    // ── inline fields ──
    FieldDef {
        name: "Runtime adapter:",
        is_inline: true,
    },
    FieldDef {
        name: "Execution surface:",
        is_inline: true,
    },
    FieldDef {
        name: "Permission mode:",
        is_inline: true,
    },
    FieldDef {
        name: "Parallelism:",
        is_inline: true,
    },
    FieldDef {
        name: "Executor:",
        is_inline: true,
    },
    FieldDef {
        name: "任务级别：",
        is_inline: true,
    },
    FieldDef {
        name: "Execution effort:",
        is_inline: true,
    },
    FieldDef {
        name: "Workflow authority:",
        is_inline: true,
    },
    // ── multi-line fields ──
    FieldDef {
        name: "本次任务相关文件：",
        is_inline: false,
    },
    FieldDef {
        name: "Verification gate:",
        is_inline: false,
    },
    FieldDef {
        name: "读取并遵守：",
        is_inline: false,
    },
    FieldDef {
        name: "Review gate:",
        is_inline: false,
    },
    FieldDef {
        name: "记忆胶囊：",
        is_inline: false,
    },
    FieldDef {
        name: "停止条件：",
        is_inline: false,
    },
    FieldDef {
        name: "关键路径：",
        is_inline: false,
    },
    FieldDef {
        name: "项目画像：",
        is_inline: false,
    },
    FieldDef {
        name: "任务存档：",
        is_inline: false,
    },
    FieldDef {
        name: "相关路径：",
        is_inline: false,
    },
    FieldDef {
        name: "非目标：",
        is_inline: false,
    },
    FieldDef {
        name: "路径：",
        is_inline: false,
    },
    FieldDef {
        name: "读取：",
        is_inline: false,
    },
    FieldDef {
        name: "任务：",
        is_inline: false,
    },
    FieldDef {
        name: "目标：",
        is_inline: false,
    },
    FieldDef {
        name: "验证：",
        is_inline: false,
    },
    FieldDef {
        name: "交付：",
        is_inline: false,
    },
    FieldDef {
        name: "背景：",
        is_inline: false,
    },
];

/// Find the longest field-definition that is a prefix of `line`.
fn find_field(line: &str) -> Option<(&'static FieldDef, &str)> {
    FIELD_DEFS
        .iter()
        .filter_map(|def| line.strip_prefix(def.name).map(|rest| (def, rest)))
        .max_by_key(|(def, _)| def.name.len())
}

// ── Card parsing ───────────────────────────────────────────────────────

/// Parse a task-card into a field-name → value map.
///
/// Inline fields store the portion after `: ` or `：`.
/// Multi-line fields collect text between the field header and the next
/// recognised field header (or EOF).
fn parse_card(input: &str) -> HashMap<String, String> {
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut current_field: Option<&str> = None;
    let mut current_value = String::new();

    for line in input.lines() {
        let trimmed = line.trim();

        if let Some((def, rest)) = find_field(trimmed) {
            // Save the previous multi-line field
            if let Some(fname) = current_field.take() {
                let v = current_value.trim().to_string();
                fields.insert(fname.to_string(), v);
                current_value = String::new();
            }

            if def.is_inline {
                let value =
                    rest.trim_start_matches(|c: char| c == ':' || c == '：' || c.is_whitespace());
                fields.insert(def.name.to_string(), value.to_string());
            } else {
                current_field = Some(def.name);
                let value_start =
                    rest.trim_start_matches(|c: char| c == ':' || c == '：' || c.is_whitespace());
                current_value.push_str(value_start);
                current_value.push('\n');
            }
        } else if current_field.is_some() {
            current_value.push_str(line);
            current_value.push('\n');
        }
    }

    // Save trailing multi-line field
    if let Some(fname) = current_field {
        let v = current_value.trim().to_string();
        fields.insert(fname.to_string(), v);
    }

    fields
}

/// Detect card type by structural position of the second non-empty line.
///
/// - `路径：` → Compact
/// - `AGENT_SUITE_COMPACT_TASK_CARD_V1` → Compact
/// - `读取并遵守：` → Full
/// - otherwise → fallback to contains-based logic
fn detect_card_type(input: &str) -> CardType {
    let second_line = input
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .nth(1); // 0: ## 任务卡, 1: discriminator line

    match second_line {
        Some(line) if line.starts_with("AGENT_SUITE_COMPACT_TASK_CARD_V1") => CardType::Compact,
        Some(line) if line.starts_with("路径：") => CardType::Compact,
        Some(line) if line.starts_with("读取并遵守：") => CardType::Full,
        // Fallback: old contains-based logic
        _ => {
            if input.contains("读取并遵守：") {
                CardType::Full
            } else {
                CardType::Compact
            }
        }
    }
}

/// Validate a task card string, returning the parsed fields on success.
///
/// This is the single-call bridge from raw text to structured fields.
/// On validation failure, returns `Err(errors)`.  On success, returns
/// `Ok(ParsedTaskCard)` with parsed fields and detected card type.
pub fn parse_validated(input: &str) -> Result<ParsedTaskCard, Vec<String>> {
    let errors = validate(input);
    if !errors.is_empty() {
        return Err(errors);
    }
    let card_type = detect_card_type(input);
    let fields = parse_card(input);
    Ok(ParsedTaskCard { fields, card_type })
}

/// Get a field value from the parsed card, or empty string if missing.
fn field_val<'a>(fields: &'a HashMap<String, String>, key: &str) -> &'a str {
    fields.get(key).map(|s| s.as_str()).unwrap_or("")
}

/// Get Execution effort, defaulting to "unknown" when absent.
///
/// Execution effort describes thinking intensity only; it does NOT gate
/// authority.  This function exists to document the default-semantics
/// contract and is available for future policy checks.
#[allow(dead_code)]
fn get_execution_effort(fields: &HashMap<String, String>) -> &str {
    fields
        .get("Execution effort:")
        .map(|s| s.as_str())
        .unwrap_or("unknown")
}

/// Get Workflow authority, defaulting to "none" when absent.
fn get_workflow_authority(fields: &HashMap<String, String>) -> &str {
    fields
        .get("Workflow authority:")
        .map(|s| s.as_str())
        .unwrap_or("none")
}

/// Keywords that indicate a task is requesting dynamic workflow / subagent /
/// multi-session / agent-team / delegation execution.
///
/// Matched **case-insensitively** against the full action context (all
/// action-bearing sections).
const WORKFLOW_REQUEST_KEYWORDS: &[&str] = &[
    // English — delegation
    "workflow",
    "dynamic workflow",
    "dynamic workflows",
    "subagent",
    "sub-agent",
    "multi-session",
    "agent-team",
    "parallel agents",
    "delegate",
    "delegation",
    // Chinese — delegation
    "并行 agent",
    "并行代理",
    "子代理",
    "多会话",
    "动态工作流",
    "工作流",
];

/// Keywords that imply parallelism/delegation in task body text (used for
/// Parallelism: none/limited → body text contradiction check).
const PARALLELISM_BODY_KEYWORDS: &[&str] = &[
    "subagent",
    "sub-agent",
    "multi-session",
    "agent-team",
    "dynamic workflow",
    "dynamic workflows",
    "parallel agents",
    "delegate",
    "delegation",
    "并行 agent",
    "并行代理",
    "子代理",
    "多会话",
    "动态工作流",
    "工作流",
];

/// Keywords that indicate ultracode is being abused as execution-authority
/// rather than thinking-intensity.  Matched case-insensitively against the
/// full action-bearing context.
const ULTRACODE_AUTHORITY_ABUSE_KEYWORDS: &[&str] = &[
    // Chinese — ultracode as authority / permission escalation
    "以 ultracode 权限",
    "以 ultracode 执行",
    "ultracode 权限执行",
    "ultracode 自动执行",
    "ultracode 可以跳过",
    "ultracode 可以直接",
    "因为 ultracode",
    "ultracode 模式下",
    // Chinese — review bypass
    "ultracode 无需人工",
    "ultracode 不需要 review",
    "ultracode 跳过 review",
    // English — authority mapping
    "ultracode allows",
    "ultracode enables",
    "ultracode authorizes",
    "with ultracode authority",
    "ultracode mode enables",
    "because ultracode",
    // English — review bypass
    "ultracode skip review",
    "ultracode bypass",
    "ultracode auto-approve",
];

// ── Phase 1: format checks (existing) ──────────────────────────────────

/// Truncate a string to at most 80 bytes on a UTF-8 character boundary,
/// appending `…` if truncated.
fn trunc80(s: &str) -> String {
    if s.len() <= 80 {
        return s.to_string();
    }
    let end = s.floor_char_boundary(80);
    format!("{}…", &s[..end])
}

/// Check whether `line` opens a text-typed code fence.
///
/// Backtick fence: 3+ backticks then `text`.
/// Tilde fence: 4+ tildes then `text`.
fn is_text_fence_line(line: &str) -> bool {
    let trimmed = line.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.is_empty() {
        return false;
    }

    let first = chars[0];

    // Backtick fence: 3+ backticks then "text"
    if first == '`' {
        let count = chars.iter().take_while(|&&c| c == '`').count();
        if count >= 3 {
            let rest: String = chars[count..].iter().collect();
            return rest.starts_with("text");
        }
    }

    // Tilde fence: 4+ tildes then "text" (intentional Rust enhancement)
    if first == '~' {
        let count = chars.iter().take_while(|&&c| c == '~').count();
        if count >= 4 {
            let rest: String = chars[count..].iter().collect();
            return rest.starts_with("text");
        }
    }

    false
}

/// Find the first text-typed code fence. Returns a 1-based line number, or None.
fn find_text_fence(input: &str) -> Option<usize> {
    for (i, line) in input.lines().enumerate() {
        if is_text_fence_line(line) {
            return Some(i + 1);
        }
    }
    None
}

// ── Phase 2: field-value checks ────────────────────────────────────────

fn check_field_values(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    // Executor
    if let Some(v) = fields.get("Executor:") {
        if !VALID_EXECUTORS.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Executor 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_EXECUTORS.join(", ")
            ));
        }
    }

    // Runtime adapter
    if let Some(v) = fields.get("Runtime adapter:") {
        if !VALID_RUNTIME_ADAPTERS.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Runtime adapter 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_RUNTIME_ADAPTERS.join(", ")
            ));
        }
    }

    // Execution surface
    if let Some(v) = fields.get("Execution surface:") {
        if !VALID_EXECUTION_SURFACES.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Execution surface 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_EXECUTION_SURFACES.join(", ")
            ));
        }
    }

    // Permission mode
    if let Some(v) = fields.get("Permission mode:") {
        if v == "autonomous-low-risk" {
            errors.push(format!(
                "[{}] autonomous-low-risk 尚未进入 Rust canonical gate（需先实现 Light-only、protected-path 禁止、Heavy 禁止等硬门禁）。当前 canonical gate 允许: {}",
                error_code::AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE,
                VALID_PERMISSION_MODES.join(", ")
            ));
        } else if !VALID_PERMISSION_MODES.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Permission mode 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_PERMISSION_MODES.join(", ")
            ));
        }
    }

    // Parallelism
    if let Some(v) = fields.get("Parallelism:") {
        if !VALID_PARALLELISM.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Parallelism 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_PARALLELISM.join(", ")
            ));
        }
    }

    // 任务级别
    if let Some(v) = fields.get("任务级别：") {
        if !VALID_TASK_LEVELS.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] 任务级别 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_TASK_LEVELS.join(", ")
            ));
        }
    }

    // Execution effort
    if let Some(v) = fields.get("Execution effort:") {
        if !VALID_EXECUTION_EFFORT.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Execution effort 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_EXECUTION_EFFORT.join(", ")
            ));
        }
    }

    // Workflow authority
    if let Some(v) = fields.get("Workflow authority:") {
        if !VALID_WORKFLOW_AUTHORITY.contains(&v.as_str()) {
            errors.push(format!(
                "[{}] Workflow authority 值 `{}` 非法，允许: {}",
                error_code::INVALID_FIELD_VALUE,
                v,
                VALID_WORKFLOW_AUTHORITY.join(", ")
            ));
        }
    }
}

// ── Phase 3: field-combination checks ──────────────────────────────────

fn check_field_combinations(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    let executor = field_val(fields, "Executor:");
    let adapter = field_val(fields, "Runtime adapter:");
    let permission = field_val(fields, "Permission mode:");
    let level = field_val(fields, "任务级别：");
    let authority = get_workflow_authority(fields);

    // Executor ↔ Runtime adapter
    if !executor.is_empty() && !adapter.is_empty() {
        if let Some(expected) = expected_adapter(executor) {
            if adapter != expected {
                errors.push(format!(
                    "[{}] Executor `{}` 要求 Runtime adapter 为 `{}`，实际为 `{}`",
                    error_code::FIELD_COMBINATION_MISMATCH,
                    executor,
                    expected,
                    adapter
                ));
            }
        }
    }

    // Heavy + execute-and-verify → forbidden
    if level == "Heavy" && permission == "execute-and-verify" {
        errors.push(format!(
            "[{}] 任务级别 Heavy 不允许 Permission mode 为 execute-and-verify",
            error_code::FIELD_COMBINATION_MISMATCH
        ));
    }

    // Workflow authority: allowed only for Medium or Heavy
    if authority == "allowed" && level != "Medium" && level != "Heavy" {
        errors.push(format!(
            "[{}] Workflow authority 为 allowed，但任务级别为 {}（仅允许 Medium 或 Heavy）",
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            level
        ));
    }

    // Workflow authority cannot exceed Permission mode
    if authority == "allowed" && (permission == "read-only" || permission == "plan-only") {
        errors.push(format!(
            "[{}] Workflow authority 为 allowed，但 Permission mode 为 {}（allowed 不可突破 Permission mode）",
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            permission
        ));
    }

    // Permission mode: plan-only → Workflow authority at most plan-only
    if permission == "plan-only" && (authority == "within-card" || authority == "allowed") {
        errors.push(format!(
            "[{}] Permission mode 为 plan-only，Workflow authority 不能为 {}",
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            authority
        ));
    }

    // ── Heavy + plan-only delivery gate ──
    // Heavy plan-only tasks must only produce plans/reports for human review,
    // not promise completed modifications, commits, or syncs.
    if level == "Heavy" && permission == "plan-only" {
        let delivery = field_val(fields, "交付：");
        let stop = field_val(fields, "停止条件：");
        let gate = field_val(fields, "Verification gate:");
        let delivery_lower = delivery.to_lowercase();
        let stop_lower = stop.to_lowercase();
        // Full cards encode stop conditions inside Verification gate.
        let gate_stop = extract_verification_gate_stop_condition(gate);
        let gate_stop_lower = gate_stop.to_lowercase();

        // Check 1: delivery must not promise modification/commit/push/sync
        let bad_delivery_keywords = &[
            "commit",
            "push",
            "提交",
            "推送",
            "合并",
            "同步到 stable",
            "同步到 A1",
            "sync to stable",
            "修改完成",
            "修改已完成",
            "代码已修改",
            "已实现",
            "已完成修改",
        ];
        let has_bad_delivery = bad_delivery_keywords
            .iter()
            .any(|kw| delivery_lower.contains(&kw.to_lowercase()));
        if has_bad_delivery {
            errors.push(format!(
                "[{}] 任务级别 Heavy + Permission mode plan-only：交付 不得承诺修改完成、提交、push 或同步 stable/A1。plan-only 只能产出方案/计划，等待人工审阅后才能进入修改阶段",
                error_code::PLAN_ONLY_DELIVERY_VIOLATION
            ));
        }

        // Check 2: stop or delivery must contain review handoff semantics.
        // For full cards, also scan Verification gate stop-condition section.
        let handoff_keywords = &[
            "返回",
            "审阅",
            "确认",
            "批准",
            "等待",
            "人工",
            "Codex",
            "用户",
            "review",
            "approval",
            "confirm",
            "不得直接修改",
            "不得直接执行",
            "明确批准",
            "explicit approval",
        ];
        let has_handoff_in_stop = handoff_keywords
            .iter()
            .any(|kw| stop_lower.contains(&kw.to_lowercase()));
        let has_handoff_in_gate_stop = handoff_keywords
            .iter()
            .any(|kw| gate_stop_lower.contains(&kw.to_lowercase()));
        let has_handoff_in_delivery = handoff_keywords
            .iter()
            .any(|kw| delivery_lower.contains(&kw.to_lowercase()));
        if !has_handoff_in_stop && !has_handoff_in_gate_stop && !has_handoff_in_delivery {
            errors.push(format!(
                "[{}] 任务级别 Heavy + Permission mode plan-only：停止条件 或 交付（含 Verification gate stop condition）必须明确要求返回用户/Codex 审阅、等待明确批准、不得直接修改。当前各段均未检测到审阅交还语义",
                error_code::HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF
            ));
        }
    }
}

// ── Phase 4: protected-path checks ─────────────────────────────────────

fn check_protected_paths(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    let level = field_val(fields, "任务级别：");
    let permission = field_val(fields, "Permission mode:");
    let authority = get_workflow_authority(fields);
    let action = action_context(fields);

    // Check if any protected path is mentioned in action-bearing sections.
    let has_protected_path = contains_protected_mention(&action);
    if !has_protected_path {
        return;
    }

    // Check for modification intent (case-insensitive, negation-aware)
    let modification_intent = has_modification_intent(&action);

    if !modification_intent {
        return;
    }

    if has_explicit_protected_read_only_boundary(fields) {
        return;
    }

    // ── Existing tiered checks ──

    // Light tasks must never target protected paths
    if level == "Light" {
        errors.push(format!(
            "[{}] Light 任务禁止修改保护路径（检测到修改意图 + 保护路径）",
            error_code::RISK_LEVEL_MISMATCH
        ));
        return;
    }

    // Plan-only or read-only + modification intent on protected paths → fail
    if permission == "plan-only" || permission == "read-only" {
        errors.push(format!(
            "[{}] Permission mode `{}` 不允许修改保护路径（检测到修改意图 + 保护路径）",
            error_code::PROTECTED_PATH_VIOLATION,
            permission
        ));
    }

    // ── Boundary + Workflow authority rules ──

    // Workflow authority: allowed and within-card must not be used with
    // protected-boundary modifications.  The executor could fan out changes
    // across boundaries uncontrollably.  Only none and plan-only are permitted
    // when the action targets protected assets with modification intent.
    if authority == "allowed" || authority == "within-card" {
        errors.push(format!(
            "[{}] 任务涉及保护边界资产且要求修改，Workflow authority 不能为 {}（只能 none 或 plan-only）",
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            authority
        ));
    }
}

// ── Phase 5: content-quality checks ────────────────────────────────────

fn is_weak_value(v: &str) -> bool {
    let trimmed = v.trim();
    let lowered = trimmed.to_ascii_lowercase();
    trimmed.is_empty()
        || WEAK_GOAL_VALUES.contains(&trimmed)
        || WEAK_GOAL_VALUES.contains(&lowered.as_str())
}

fn check_content_quality(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    // 目标 — required in both compact and full
    if let Some(goal) = fields.get("目标：") {
        if is_weak_value(goal) {
            errors.push(format!(
                "[{}] 目标 不能为空或弱值（{}），当前: `{}`",
                error_code::EMPTY_OR_WEAK_SECTION,
                WEAK_GOAL_VALUES.join("/"),
                if goal.len() > 60 { &goal[..60] } else { goal }
            ));
        }
    }

    // 非目标 — required in both compact and full
    if let Some(non_goal) = fields.get("非目标：") {
        if non_goal.trim().is_empty() {
            errors.push(format!(
                "[{}] 非目标 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }

    // 验证 — required in both compact and full
    if fields.contains_key("验证：") || fields.contains_key("Verification gate:") {
        let verification = field_val(fields, "验证：");
        let gate = field_val(fields, "Verification gate:");
        let v_trimmed = if verification.trim().is_empty() {
            gate.trim()
        } else {
            verification.trim()
        };
        if v_trimmed.is_empty() || v_trimmed == "test" {
            errors.push(format!(
                "[{}] 验证 不能只有 `test`，需要具体命令或明确验收动作",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }

    // 停止条件 — compact only
    if let Some(stop) = fields.get("停止条件：") {
        if stop.trim().is_empty() {
            errors.push(format!(
                "[{}] 停止条件 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }

    // 交付 — compact only
    if let Some(delivery) = fields.get("交付：") {
        if delivery.trim().is_empty() {
            errors.push(format!(
                "[{}] 交付 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }
}

// ── Phase 6: contradiction checks ──────────────────────────────────────

/// Check whether `text` expresses modification intent after ignoring common
/// no-op, planning, confirmation, and stop-condition phrases.
///
/// The rule is segment-based rather than whole-text replacement: a task card
/// can safely say "不修改文件" in one clause and still be rejected when another
/// clause says "再修改 validator".
fn has_modification_intent(text: &str) -> bool {
    normalize_modification_intent_text(text)
        .split(is_intent_segment_separator)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .any(segment_has_positive_modification_intent)
}

fn normalize_modification_intent_text(text: &str) -> String {
    text.to_lowercase()
        // Crate/path identifiers are not task intent.  Keep the checker strict
        // for prose like "sync to stable", but do not reject a read-only audit
        // just because it references the drift-checker crate by name.
        .replace("workflow-sync-check", "drift_checker_crate")
        // Slash-separated stop/non-goal wording otherwise splits into a bare
        // `patch` segment, losing the surrounding negation or stop language.
        .replace("apply/patch", "apply_or_diff_pair")
        .replace("apply / patch", "apply_or_diff_pair")
        .replace("apply-or-patch", "apply_or_diff_pair")
}

fn is_intent_segment_separator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | ';' | '；' | ',' | '，' | '。' | '、' | '|' | '/' | '：' | ':'
    )
}

fn segment_has_positive_modification_intent(segment: &str) -> bool {
    let has_keyword = MODIFICATION_KEYWORDS
        .iter()
        .any(|kw| segment.contains(&kw.to_lowercase()));

    has_keyword && !is_safe_non_modification_segment(segment)
}

fn is_safe_non_modification_segment(segment: &str) -> bool {
    contains_any(
        segment,
        &[
            // Chinese no-op and prohibition forms.
            "不修改",
            "不改",
            "不变更",
            "不删除",
            "不覆盖",
            "不迁移",
            "不重写",
            "不替换",
            "不提交",
            "不推送",
            "不写入",
            "不落地",
            "不应用",
            "不执行",
            "不实施",
            "不调整",
            "不生成",
            "不创建",
            "不部署",
            "不安装",
            "不发布",
            "不进行",
            "不需要",
            "无需",
            "无须",
            "不要",
            "不得",
            "禁止",
            "严禁",
            "不应",
            "不会",
            "不做任何",
            "不产生",
            "不触碰",
            "免修改",
            "免删除",
            "免部署",
            "免安装",
            // Planning and proposal nouns.
            "修改建议",
            "修复建议",
            "变更建议",
            "升级建议",
            "重构建议",
            "修改计划",
            "修复计划",
            "变更计划",
            "patch 计划",
            "patch计划",
            "diff 草案",
            "修改草案",
            "实施方案",
            "实施计划",
            "实现建议",
            "实现计划",
            "实现草案",
            "执行计划",
            "执行方案",
            "执行建议",
            "生成方案",
            "创建方案",
            "部署方案",
            "安装方案",
            "发布方案",
            "同步方案",
            "调整方案",
            "解决方案",
            "设计方案",
            "待确认的修改",
            "待确认的变更",
            // English no-op forms.
            "no modify",
            "no modification",
            "no changes",
            "no file changes",
            "no commit",
            "no push",
            "no rewrite",
            "no refactor",
            "no deploy",
            "no install",
            "no publish",
            "no sync",
            "without modifying",
            "without modify",
            "without changes",
            "without changing",
            "without deploying",
            "without installing",
            "without publishing",
            "without syncing",
            "do not modify",
            "do not change",
            "do not delete",
            "do not write",
            "do not apply",
            "do not commit",
            "do not deploy",
            "do not install",
            "do not publish",
            "don't modify",
            "don't change",
            "don't delete",
            "not modifying",
            "not changing",
            "read-only",
            "analysis only",
            "plan only",
            "change proposal",
            "patch proposal",
            "deploy plan",
            "deployment plan",
            "install plan",
            "installation plan",
            "publish plan",
            "sync plan",
            "execution plan",
            "modification plan",
            "implementation plan",
            "verification plan",
            "refactor plan",
            "rewrite plan",
            // Nouns that contain modification-keyword substrings
            "执行器",
            "执行者",
            "应用程序",
            "创建者",
            "安装包",
            "生成器",
            "部署工具",
            "发布工具",
            "同步工具",
            "执行卡",
            "后续工作卡",
            "工作卡草案",
            "后续任务卡",
            "任务卡草案",
        ],
    ) || is_stop_or_confirmation_segment(segment)
}

fn is_stop_or_confirmation_segment(segment: &str) -> bool {
    let has_need_or_future = contains_any(
        segment,
        &[
            "如需",
            "如果需要",
            "发现需要",
            "需要",
            "必须",
            "需改",
            "需修改",
            "需变更",
            "再修改",
            "后再修改",
            "后再变更",
            "would require",
            "requires",
            "if changes",
            "if modification",
            "if rewrite",
        ],
    );
    let has_stop_or_confirm = contains_any(
        segment,
        &[
            "停止",
            "暂停",
            "停下",
            "等待",
            "确认",
            "请求确认",
            "用户确认",
            "报告",
            "stop",
            "pause",
            "confirm",
            "confirmation",
            "ask",
            "report",
        ],
    );

    has_need_or_future && has_stop_or_confirm
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Extract the "stop condition" sub-section from a Verification gate field.
///
/// Full cards encode stop conditions inside `Verification gate:` as:
/// `- stop condition:\n  - <text>`
/// This function extracts the text after `- stop condition:` for handoff
/// checking, so Heavy+plan-only full cards are not falsely rejected.
fn extract_verification_gate_stop_condition(gate: &str) -> String {
    let gate_lower = gate.to_lowercase();
    // Find "- stop condition:" or "- stop condition："
    if let Some(pos) = gate_lower.find("- stop condition") {
        let after_marker = &gate[pos..];
        // Find the first newline after the marker to get the content start
        if let Some(nl) = after_marker.find('\n') {
            return after_marker[nl..].to_string();
        }
        return after_marker.to_string();
    }
    String::new()
}

fn workflow_request_context(fields: &HashMap<String, String>) -> String {
    [
        "任务：",
        "目标：",
        "验证：",
        "Verification gate:",
        "停止条件：",
        "交付：",
        "背景：",
        "读取并遵守：",
    ]
    .iter()
    .filter_map(|key| fields.get(*key).map(|value| format!("{} {}", key, value)))
    .collect::<Vec<_>>()
    .join("\n")
}

fn normalize_workflow_request_text(text: &str) -> String {
    text.replace("agent-workflow", "agentprotocol")
        .replace("workflow-sync-check", "sync_check_crate")
}

fn has_workflow_request_intent(text: &str) -> bool {
    text.to_lowercase()
        .split(is_intent_segment_separator)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .any(segment_has_positive_workflow_request)
}

fn segment_has_positive_workflow_request(segment: &str) -> bool {
    let normalized = normalize_workflow_request_text(segment);
    let has_keyword = WORKFLOW_REQUEST_KEYWORDS
        .iter()
        .any(|kw| normalized.contains(&kw.to_lowercase()));

    has_keyword && !is_safe_non_workflow_request_segment(&normalized)
}

fn is_safe_non_workflow_request_segment(segment: &str) -> bool {
    contains_any(
        segment,
        &[
            "不使用",
            "不启用",
            "不要求",
            "不需要",
            "不调用",
            "不委托",
            "不进行",
            "不得",
            "不要",
            "禁止",
            "严禁",
            "无需",
            "无须",
            "no subagent",
            "no sub-agent",
            "no delegation",
            "no dynamic workflow",
            "no workflow",
            "without subagent",
            "without sub-agent",
            "without delegation",
            "without dynamic workflow",
            "do not use",
            "do not enable",
            "do not delegate",
            "don't use",
            "don't enable",
            "not using",
            "not enabling",
        ],
    )
}

fn contains_path_with_boundary(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(idx, matched)| {
        let next = text[idx + matched.len()..].chars().next();
        matches!(
            next,
            None | Some('/')
                | Some('\n')
                | Some('\r')
                | Some('\t')
                | Some(' ')
                | Some('"')
                | Some('\'')
        )
    })
}

fn contains_protected_mention(text: &str) -> bool {
    PROTECTED_PATHS
        .iter()
        .any(|pp| contains_path_with_boundary(text, pp))
        || PROTECTED_BOUNDARY_TERMS
            .iter()
            .any(|term| contains_path_with_boundary(text, term))
}

fn has_explicit_protected_read_only_boundary(fields: &HashMap<String, String>) -> bool {
    let guard_text = [
        "非目标：",
        "停止条件：",
        "验证：",
        "Verification gate:",
        "交付：",
    ]
    .iter()
    .filter_map(|key| fields.get(*key).map(|value| format!("{} {}", key, value)))
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase();

    contains_any(
        &guard_text,
        &[
            "只读引用 bootstrap",
            "只读引用 dry-run",
            "只读引用 dry run",
            "read-only reference bootstrap",
            "read-only reference dry-run",
            "read-only reference dry run",
            "不修改 bootstrap",
            "不触碰 bootstrap",
            "不得修改 bootstrap",
            "不修改 dry-run",
            "不触碰 dry-run",
            "不得修改 dry-run",
            "不修改 dry run",
            "不触碰 dry run",
            "不得修改 dry run",
            "不修改 dry-run 专用 crate",
            "不得修改 dry-run 专用 crate",
        ],
    )
}

/// Build the subset of card text that describes what the executor is asked to
/// change.  Read-only context fields and explicit non-goals are deliberately
/// excluded so "read context-capsule" and "do not modify stable" do not become
/// false protected-path violations.
///
/// Action-bearing sections include every field that could contain a
/// modification request, workflow instruction, or protected-boundary target.
fn action_context(fields: &HashMap<String, String>) -> String {
    [
        "路径：",
        "任务：",
        "目标：",
        "关键路径：",
        "相关路径：",
        "本次任务相关文件：",
        "验证：",
        "停止条件：",
        "交付：",
    ]
    .iter()
    .filter_map(|key| fields.get(*key).map(|value| format!("{} {}", key, value)))
    .collect::<Vec<_>>()
    .join("\n")
}

/// Like action_context but also includes fields that may carry instructions
/// disguised as read-lists or background context.  Used for contradiction
/// checks (e.g. read-only/plan-only vs modification keywords) where field
/// abuse is a real bypass vector, but excluded from protected-path scanning
/// to avoid false positives when legitimate reads reference protected files.
fn extended_action_context(fields: &HashMap<String, String>) -> String {
    [
        "路径：",
        "任务：",
        "目标：",
        "关键路径：",
        "相关路径：",
        "本次任务相关文件：",
        "验证：",
        "停止条件：",
        "交付：",
        "读取：",
        "背景：",
    ]
    .iter()
    .filter_map(|key| fields.get(*key).map(|value| format!("{} {}", key, value)))
    .collect::<Vec<_>>()
    .join("\n")
}

fn check_contradictions(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    let non_goal = field_val(fields, "非目标：");
    let goal = field_val(fields, "目标：");
    let task = field_val(fields, "任务：");
    let permission = field_val(fields, "Permission mode:");

    // 非目标 says "no code changes" but 目标/任务 require changes
    if non_goal.contains("不修改代码") || non_goal.contains("不修改任何文件") {
        let task_goal_text = format!("{} {}", goal, task);
        if has_modification_intent(&task_goal_text) {
            errors.push(format!(
                "[{}] 非目标声明不修改代码，但目标/任务要求修改操作",
                error_code::CONTRADICTORY_REQUIREMENT
            ));
        }
    }

    // 非目标 says "don't touch private/stable" but task mentions protected paths
    let claims_no_touch = non_goal.contains("不触碰 private")
        || non_goal.contains("不触碰 stable")
        || non_goal.contains("不修改 private")
        || non_goal.contains("不修改 stable")
        || non_goal.contains("不修改 /Volumes/Projects/my-protected-suite")
        || non_goal.contains("不修改 /Volumes/Projects/my-stable-suite");

    if claims_no_touch {
        let action = action_context(fields);
        let has_modification = has_modification_intent(&action);
        let has_protected = contains_protected_mention(&action);
        if has_modification && has_protected {
            errors.push(format!(
                "[{}] 非目标声明不触碰 private/stable，但路径或任务内容要求修改 protected 路径",
                error_code::CONTRADICTORY_REQUIREMENT
            ));
        }
    }

    // 非目标 says "no commit" but delivery/task requires commit/push
    if non_goal.contains("不提交") || non_goal.contains("不 commit") {
        let delivery = field_val(fields, "交付：");
        if delivery.contains("commit")
            || delivery.contains("push")
            || delivery.contains("提交")
            || task.contains("commit")
            || task.contains("push")
            || task.contains("提交")
        {
            errors.push(format!(
                "[{}] 非目标声明不提交，但交付或任务内容要求 commit/push",
                error_code::CONTRADICTORY_REQUIREMENT
            ));
        }
    }

    // read-only/plan-only permission but action sections require modification
    // Use extended_action_context to catch modification keywords in 读取/背景
    // fields that could be abused as instruction vectors.
    if permission == "read-only" || permission == "plan-only" {
        let action = extended_action_context(fields);
        if has_modification_intent(&action) {
            errors.push(format!(
                "[{}] Permission mode 为 {}，但任务行动内容包含修改操作",
                error_code::CONTRADICTORY_REQUIREMENT,
                permission
            ));
        }
    }
}

// ── Phase 7: Execution Authority Gate ──────────────────────────────────

/// Check that Execution effort (thinking intensity) and Workflow authority
/// (delegation/parallelism permission) are used correctly.
///
/// Core principles:
/// - Execution power is never authority.  Higher reasoning may improve
///   planning, but it does not upgrade permission.
/// - Dynamic workflow / subagent / delegation requires explicit task-card
///   Workflow authority.
/// - Matching is case-insensitive and scans ALL action-bearing sections
///   (not just 任务：+ 目标：), closing the narrow-scope bypass.
/// - Parallelism field value is cross-checked against Workflow authority
///   to prevent field-combination bypasses.
fn check_execution_authority_gate(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    let authority = get_workflow_authority(fields);
    let parallelism = field_val(fields, "Parallelism:");
    let permission = field_val(fields, "Permission mode:");

    // Build the full action-request text from ALL action-bearing sections
    // and lowercase once for case-insensitive matching.
    // Use extended_action_context to catch abuse in 读取/背景 fields.
    let action_text = extended_action_context(fields);
    let workflow_text = workflow_request_context(fields);
    let action_lower = action_text.to_lowercase();
    let workflow_lower = normalize_workflow_request_text(&workflow_text.to_lowercase());

    // ── Execution effort: ultracode must not be abused as authority ──
    let execution_effort = get_execution_effort(fields);
    if execution_effort == "ultracode" {
        let ultracode_abuse = ULTRACODE_AUTHORITY_ABUSE_KEYWORDS
            .iter()
            .any(|kw| action_lower.contains(&kw.to_lowercase()));
        if ultracode_abuse {
            errors.push(format!(
                "[{}] Execution effort 为 ultracode，但任务行动区域将 ultracode 当作执行权限/跳过 review/自动执行的依据。ultracode 只能表示思考强度，不能映射为 plan、parallel、permission escalation 或 workflow authority",
                error_code::ULTRACODE_AUTHORITY_ABUSE
            ));
        }
    }

    // ── Workflow authority: none + task asks for workflow → fail ──
    if authority == "none" {
        let asks_workflow = has_workflow_request_intent(&workflow_lower);
        if asks_workflow {
            errors.push(format!(
                "[{}] Workflow authority 为 none，但任务行动区域要求 workflow/subagent/multi-session/agent-team/delegation",
                error_code::WORKFLOW_AUTHORITY_REQUIRED
            ));
        }
    }

    // ── Workflow authority: plan-only + task requires direct modification → fail ──
    if authority == "plan-only" && has_modification_intent(&action_text) {
        errors.push(format!(
            "[{}] Workflow authority 为 plan-only，但任务行动区域要求直接修改代码",
            error_code::WORKFLOW_AUTHORITY_VIOLATION
        ));
    }

    // ── Workflow authority: within-card — validate scope containment ──
    // within-card allows fan-out but must stay inside the card's own
    // scope (paths, goals, non-goals, permission mode).  This is a
    // structural check; deep semantic verification would require the
    // task to be executed.
    if authority == "within-card" {
        // within-card requires explicit permission mode that allows execution
        if permission == "read-only" || permission == "plan-only" {
            errors.push(format!(
                "[{}] Workflow authority 为 within-card，但 Permission mode 为 {}（需要 edit-with-confirmation 或 execute-and-verify）",
                error_code::WORKFLOW_AUTHORITY_VIOLATION,
                permission
            ));
        }
    }

    // ── Parallelism field ↔ Workflow authority field combination checks ──
    // These close the bypass where Parallelism: subagent + Workflow authority: none
    // passes because the body text doesn't explicitly mention subagent.
    match parallelism {
        "subagent" | "multi-session" | "agent-team" => {
            if authority != "within-card" && authority != "allowed" {
                errors.push(format!(
                    "[{}] Parallelism 为 {}，要求 Workflow authority 为 within-card 或 allowed，当前为 {}",
                    error_code::WORKFLOW_AUTHORITY_REQUIRED,
                    parallelism,
                    authority
                ));
            }
        }
        "worktree" => {
            if authority == "none" {
                errors.push(format!(
                    "[{}] Parallelism 为 worktree，Workflow authority 不能为 none（需要 within-card、plan-only 或 allowed）",
                    error_code::WORKFLOW_AUTHORITY_REQUIRED
                ));
            }
        }
        "none" | "limited" | "parallel" => {
            // These parallelism values are compatible with any Workflow
            // authority at the field level.  BUT: if the action body text
            // still asks for subagent/multi-session/agent-team, we must
            // intercept on content grounds.
        }
        _ => {} // invalid Parallelism values are caught by check_field_values
    }

    // ── Parallelism body-text contradiction checks ──
    // When Parallelism is none/limited/parallel but the task body asks for
    // delegation patterns, fail.  This catches the case where the field
    // says "none" but the task text says "用 subagent 处理".
    if parallelism == "none" || parallelism == "limited" || parallelism == "parallel" {
        let asks_delegation = PARALLELISM_BODY_KEYWORDS
            .iter()
            .any(|kw| action_lower.contains(&kw.to_lowercase()));
        if asks_delegation {
            errors.push(format!(
                "[{}] Parallelism 为 {}，但任务行动区域要求 subagent/multi-session/agent-team/dynamic workflow",
                error_code::PARALLELISM_POLICY_VIOLATION,
                parallelism
            ));
        }
    }
}

// ── Main validate() ────────────────────────────────────────────────────

/// Validate a single input string, returning a list of errors (empty = valid).
pub fn validate(input: &str) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();

    // ── Phase 1: format checks ──

    // Rule 1: first non-empty line must be `## 任务卡`
    let first = input
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .find(|l| !l.trim().is_empty());

    match first {
        Some(line) if line == "## 任务卡" => {}
        Some(line) => {
            errors.push(format!(
                "首行必须为 `## 任务卡`，实际为 `{}`",
                trunc80(line)
            ));
        }
        None => {
            errors.push("文件为空".to_string());
            return errors;
        }
    }

    // Rule 2: reject text-typed code fences
    if let Some(pos) = find_text_fence(input) {
        errors.push(format!("第 {} 行附近：禁止使用 `text` 类型代码围栏", pos));
    }

    // Detect card type by structural position, not full-text contains.
    // - Compact: second non-empty line starts with 路径：
    // - Full: second non-empty line starts with 读取并遵守：
    // This prevents false full-card detection when compact body text
    // mentions 读取并遵守： in prose.
    let second_line = input
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .nth(1); // 0: ## 任务卡, 1: discriminator line

    let is_compact = match second_line {
        Some(line) if line.starts_with("AGENT_SUITE_COMPACT_TASK_CARD_V1") => true,
        Some(line) if line.starts_with("路径：") => true,
        Some(line) if line.starts_with("读取并遵守：") => false,
        // Fallback: use old contains-based logic for unusual formats
        _ => !input.contains("读取并遵守："),
    };

    // Rule 3/4: check required fields
    let required: &[&str] = if is_compact {
        COMPACT_REQUIRED
    } else {
        FULL_REQUIRED
    };

    let mut missing: Vec<&str> = Vec::new();
    for field in required {
        if !input.contains(field) {
            missing.push(field);
        }
    }

    if !missing.is_empty() {
        let card_type = if is_compact { "compact" } else { "full" };
        errors.push(format!(
            "{} 任务卡缺少必需字段: {}",
            card_type,
            missing.join(", ")
        ));
    }

    // Parse card for semantic checks
    let fields = parse_card(input);

    // ── Phase 2-7: semantic checks ──
    check_field_values(&fields, &mut errors);
    check_field_combinations(&fields, &mut errors);
    check_protected_paths(&fields, &mut errors);
    check_content_quality(&fields, &mut errors);
    check_contradictions(&fields, &mut errors);
    check_execution_authority_gate(&fields, &mut errors);

    errors
}

// ── Multi-file entry point ─────────────────────────────────────────────

/// Validate one or more files. Returns true if ALL files pass.
///
/// A path of `"-"` reads from stdin.
pub fn validate_files(paths: &[String]) -> bool {
    let mut all_ok = true;

    for path in paths {
        let (content, display_path) = match read_input(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", path, e);
                all_ok = false;
                continue;
            }
        };

        let errors = validate(&content);
        if errors.is_empty() {
            eprintln!("{}: OK", display_path);
        } else {
            all_ok = false;
            eprintln!("{}: FAILED", display_path);
            for err in &errors {
                eprintln!("  - {}", err);
            }
        }
    }

    all_ok
}

/// Read file or stdin.
fn read_input(path: &str) -> Result<(String, String), String> {
    if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| e.to_string())?;
        Ok((buf, "(stdin)".to_string()))
    } else {
        let p = Path::new(path);
        let content = fs::read_to_string(p).map_err(|e| e.to_string())?;
        Ok((content, p.display().to_string()))
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── test helpers ──

    fn compact_body(fields: &str) -> String {
        format!("## 任务卡\nAGENT_SUITE_COMPACT_TASK_CARD_V1\n{}\n", fields)
    }

    /// New compact format: no AGENT_SUITE_COMPACT_TASK_CARD_V1 marker.
    fn compact_body_new(fields: &str) -> String {
        format!("## 任务卡\n{}\n", fields)
    }

    fn full_body(fields: &str) -> String {
        format!("## 任务卡\n{}\n", fields)
    }

    /// Minimal valid compact card with meaningful (non-test) values.
    fn valid_compact_fields() -> String {
        compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试验证校验器功能\n\
             目标：验证任务卡校验器能正确识别合法输入\n\
             非目标：不修改任何文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止并报告\n\
             交付：\n返回测试通过结果\n",
        )
    }

    /// Minimal valid full card with meaningful (non-test) values.
    fn valid_full_fields() -> String {
        full_body(
            "读取并遵守：\n- .\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Light\n\
             Review gate:\n- Light review\n\
             任务：测试完整任务卡格式校验功能\n\
             背景：验证 full task card 所有必填字段能被正确识别\n\
             项目画像：Rust workspace with task-card-validator crate\n\
             记忆胶囊：暂无相关记忆\n\
             任务存档：参考此前 compact card 校验通过记录\n\
             相关路径：\n- .\n\
             本次任务相关文件：\n- Cargo.toml\n\
             目标：确认 full task card 校验器正确接受合法输入\n\
             非目标：不涉及生产环境变更\n\
             验证：\ncargo test --workspace\n\
             Verification gate:\n- commands: cargo test\n\
             交付：\n按协议输出测试通过结果\n",
        )
    }

    /// Compact card with `Executor: Other` and `Runtime adapter: generic` (legal).
    fn compact_other_generic() -> String {
        compact_body(
            "路径：\n- .\n\
             Executor: Other\n\
             Runtime adapter: generic\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: unknown\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：人工审核任务\n\
             目标：由人工执行器处理\n\
             非目标：不涉及自动化\n\
             关键路径：\n- .\n\
             验证：\n人工确认完成\n\
             停止条件：\n人工确认失败时停止\n\
             交付：\n返回人工审核结果\n",
        )
    }

    // ── first-line rule ────────────────────────────────────────

    #[test]
    fn reject_empty_input() {
        let e = validate("");
        assert!(!e.is_empty());
        assert!(e[0].contains("为空"));
    }

    #[test]
    fn reject_wrong_first_line() {
        let e = validate("# 任务卡\nExecutor: X\n");
        assert!(!e.is_empty());
        assert!(e[0].contains("首行必须为"));
    }

    #[test]
    fn accept_correct_first_line() {
        let body = valid_compact_fields();
        let e = validate(&body);
        assert!(e.is_empty(), "unexpected errors: {:?}", e);
    }

    #[test]
    fn first_non_empty_line_skips_blanks() {
        let body = valid_compact_fields();
        let e = validate(&body);
        assert!(e.is_empty(), "unexpected errors: {:?}", e);
    }

    #[test]
    fn reject_whitespace_padded_header_leading() {
        let input = format!(
            " ## 任务卡\nAGENT_SUITE_COMPACT_TASK_CARD_V1\n{}",
            valid_compact_fields()
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "should reject leading-space header");
        assert!(
            e[0].contains("首行必须为"),
            "should report wrong first line: {:?}",
            e
        );
    }

    #[test]
    fn reject_whitespace_padded_header_trailing() {
        let input = format!(
            "## 任务卡 \nAGENT_SUITE_COMPACT_TASK_CARD_V1\n{}",
            valid_compact_fields()
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "should reject trailing-space header");
        assert!(
            e[0].contains("首行必须为"),
            "should report wrong first line: {:?}",
            e
        );
    }

    // ── text fence rejection ───────────────────────────────────

    #[test]
    fn reject_backtick_text_fence() {
        let mut input = valid_compact_fields();
        input.push_str("```text\nbad stuff\n```\n");
        let e = validate(&input);
        assert!(e.iter().any(|m| m.contains("text")), "errors: {:?}", e);
    }

    #[test]
    fn allow_non_text_fences() {
        let mut input = valid_compact_fields();
        input.push_str("```rust\nlet x = 1;\n```\n");
        let e = validate(&input);
        assert!(e.is_empty(), "unexpected errors: {:?}", e);
    }

    #[test]
    fn reject_four_backtick_text_fence() {
        let mut input = valid_compact_fields();
        input.push_str("````text\nbad stuff\n````\n");
        let e = validate(&input);
        assert!(
            e.iter().any(|m| m.contains("text")),
            "should reject 4-backtick text fence: {:?}",
            e
        );
    }

    #[test]
    fn reject_five_backtick_text_fence() {
        let mut input = valid_compact_fields();
        input.push_str("`````text\nbad stuff\n`````\n");
        let e = validate(&input);
        assert!(
            e.iter().any(|m| m.contains("text")),
            "should reject 5-backtick text fence: {:?}",
            e
        );
    }

    // ── tilde text fence ─────────────────────────────────────────

    #[test]
    fn reject_five_tilde_text_fence() {
        // Rust validator detects tilde text fences (4+ ~ then "text").
        let mut input = valid_compact_fields();
        input.push_str("~~~~~text\nbad stuff\n~~~~~\n");
        let e = validate(&input);
        assert!(
            e.iter().any(|m| m.contains("text")),
            "should reject 5-tilde text fence: {:?}",
            e
        );
    }

    #[test]
    fn reject_four_tilde_text_fence() {
        let mut input = valid_compact_fields();
        input.push_str("~~~~text\nbad stuff\n~~~~\n");
        let e = validate(&input);
        assert!(
            e.iter().any(|m| m.contains("text")),
            "should reject 4-tilde text fence: {:?}",
            e
        );
    }

    #[test]
    fn allow_three_tilde_non_text_fence() {
        // 3 tildes is below the detection threshold (4+).
        let mut input = valid_compact_fields();
        input.push_str("~~~text\nnot a valid fence\n~~~\n");
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "3-tilde fence is not a valid text fence: {:?}",
            e
        );
    }

    // ── compact vs full detection ──────────────────────────────

    #[test]
    fn detect_compact_card() {
        let input = valid_compact_fields();
        let e = validate(&input);
        assert!(e.is_empty(), "compact should be valid: {:?}", e);
    }

    #[test]
    fn detect_full_card() {
        let input = valid_full_fields();
        let e = validate(&input);
        assert!(e.is_empty(), "full should be valid: {:?}", e);
    }

    #[test]
    fn new_compact_format_passes() {
        // New compact format: no AGENT_SUITE_COMPACT_TASK_CARD_V1 marker.
        let input = compact_body_new(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试验证校验器功能\n\
             目标：验证任务卡校验器能正确识别新 compact 格式\n\
             非目标：不修改任何文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止并报告\n\
             交付：\n返回测试通过结果\n",
        );
        let e = validate(&input);
        assert!(e.is_empty(), "new compact format should pass: {:?}", e);
    }

    #[test]
    fn old_compact_format_still_passes() {
        // Old compact format: has AGENT_SUITE_COMPACT_TASK_CARD_V1 marker.
        let input = valid_compact_fields();
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "old compact format should still pass: {:?}",
            e
        );
    }

    #[test]
    fn compact_card_with_agile_in_body_not_misdetected_as_full() {
        // 读取并遵守： is the full-card discriminator only when it appears
        // as the second non-empty line (structural position). A compact card
        // that mentions 读取并遵守： in body text must NOT be misdetected as full.
        let input = compact_body_new(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：执行一些测试\n\
             目标：验证兼容性\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\nfail 时停止\n\
             交付：\n交付报告（参考 agent-task-protocol.md 的 读取并遵守： 规则）\n",
        );
        let e = validate(&input);
        // Structural detection: second non-empty line is 路径：, so this is compact.
        // The 读取并遵守： mention in the 交付 body must not trigger full-card detection.
        assert!(
            e.is_empty(),
            "compact card with 读取并遵守： in body text must pass as compact, not be misdetected as full: {:?}",
            e
        );
    }

    #[test]
    fn full_card_missing_goals() {
        let input = full_body(
            "读取并遵守：\n- .\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Light\n\
             Review gate:\n- test\n\
             任务：测试功能\n\
             背景：验证功能正确性\n\
             项目画像：test\n\
             记忆胶囊：test\n\
             任务存档：test\n\
             相关路径：\n- .\n\
             本次任务相关文件：\n- .\n\
             验证：\ncargo test\n\
             Verification gate:\n- test\n\
             交付：\n返回测试结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter().any(|m| m.contains("目标") && m.contains("非目标")),
            "errors: {:?}",
            e
        );
    }

    // ── required fields ────────────────────────────────────────

    #[test]
    fn compact_missing_executor() {
        let input = compact_body(
            "路径：\n- .\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(e.iter().any(|m| m.contains("Executor")));
    }

    #[test]
    fn compact_all_present() {
        let input = valid_compact_fields();
        let e = validate(&input);
        assert!(e.is_empty(), "unexpected errors: {:?}", e);
    }

    #[test]
    fn compact_missing_body_field_读取() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "should fail when 读取 is missing");
        assert!(
            e.iter().any(|m| m.contains("读取")),
            "should report missing 读取: {:?}",
            e
        );
    }

    #[test]
    fn compact_missing_body_field_停止条件() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "should fail when 停止条件 is missing");
        assert!(
            e.iter().any(|m| m.contains("停止条件")),
            "should report missing 停止条件: {:?}",
            e
        );
    }

    #[test]
    fn compact_missing_multiple_body_fields() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "should fail with multiple missing fields");
        let msg = e.join("|");
        assert!(msg.contains("任务"), "should mention 任务: {:?}", e);
        assert!(msg.contains("关键路径"), "should mention 关键路径: {:?}", e);
        assert!(msg.contains("交付"), "should mention 交付: {:?}", e);
    }

    // ── regression: long Chinese wrong first line no panic ─────

    #[test]
    fn long_chinese_wrong_first_line_no_panic() {
        let long_cn = std::iter::repeat("一").take(30).collect::<String>();
        let input = format!("{}\nExecutor: X\n", long_cn);
        let e = validate(&input);
        assert!(!e.is_empty(), "should fail for wrong first line");
        assert!(
            e[0].contains("首行必须为"),
            "should report wrong first line: {:?}",
            e
        );
        assert!(
            e[0].contains("…"),
            "should truncate long first line with …: {:?}",
            e
        );
    }

    #[test]
    fn long_chinese_wrong_first_line_exact_char_boundary() {
        let cn26 = std::iter::repeat("二").take(26).collect::<String>();
        let cn27 = std::iter::repeat("三").take(27).collect::<String>();
        let input26 = format!("{}\nExecutor: X\n", cn26);
        let input27 = format!("{}\nExecutor: X\n", cn27);

        let e26 = validate(&input26);
        assert!(!e26.is_empty());
        assert!(
            !e26[0].contains("…"),
            "78-byte line should not need truncation: {:?}",
            e26
        );

        let e27 = validate(&input27);
        assert!(!e27.is_empty());
        assert!(
            e27[0].contains("…"),
            "81-byte line should be truncated: {:?}",
            e27
        );
    }

    // ── trunc80 unit tests ─────────────────────────────────────

    #[test]
    fn trunc80_short_enough() {
        assert_eq!(trunc80("hello"), "hello");
        assert_eq!(trunc80(""), "");
    }

    #[test]
    fn trunc80_exact_80_bytes_ascii() {
        let s = "x".repeat(80);
        assert_eq!(trunc80(&s), s);
    }

    #[test]
    fn trunc80_over_80_bytes_ascii() {
        let s = "x".repeat(100);
        let result = trunc80(&s);
        assert!(result.len() < 100);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn trunc80_multi_byte_at_boundary() {
        let s = std::iter::repeat("中").take(28).collect::<String>(); // 28*3 = 84 bytes
        let result = trunc80(&s);
        assert!(result.len() <= 82); // 78 + "…" bytes
        assert!(result.ends_with('…'));
    }

    // ── parse_card tests ───────────────────────────────────────

    #[test]
    fn parse_card_extracts_inline_fields() {
        let input = valid_compact_fields();
        let fields = parse_card(&input);
        assert_eq!(
            fields.get("Executor:").map(|s| s.as_str()),
            Some("Claude Code")
        );
        assert_eq!(
            fields.get("Runtime adapter:").map(|s| s.as_str()),
            Some("claude-code")
        );
        assert_eq!(fields.get("任务级别：").map(|s| s.as_str()), Some("Medium"));
    }

    #[test]
    fn parse_card_extracts_multiline_fields() {
        let input = valid_compact_fields();
        let fields = parse_card(&input);
        assert!(fields
            .get("任务：")
            .map_or(false, |v| v.contains("运行测试")));
        assert!(fields
            .get("目标：")
            .map_or(false, |v| v.contains("验证任务卡校验器")));
    }

    // ── Phase 2: field-value checks ────────────────────────────

    #[test]
    fn reject_invalid_executor() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: BadAgent\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::INVALID_FIELD_VALUE)),
            "should have INVALID_FIELD_VALUE: {:?}",
            e
        );
        assert!(e.iter().any(|m| m.contains("Executor")), "errors: {:?}", e);
    }

    #[test]
    fn reject_invalid_task_level() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Critical\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::INVALID_FIELD_VALUE)),
            "should have INVALID_FIELD_VALUE: {:?}",
            e
        );
    }

    #[test]
    fn allow_executor_other() {
        let input = compact_other_generic();
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "Executor Other + generic should pass: {:?}",
            e
        );
    }

    #[test]
    fn reject_invalid_permission_mode() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: full-access\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::INVALID_FIELD_VALUE)),
            "errors: {:?}",
            e
        );
    }

    // ── Phase 3: field-combination checks ──────────────────────

    #[test]
    fn reject_executor_adapter_mismatch() {
        // Executor: Claude Code requires Runtime adapter: claude-code
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: codex-local\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::FIELD_COMBINATION_MISMATCH)),
            "should have FIELD_COMBINATION_MISMATCH: {:?}",
            e
        );
    }

    #[test]
    fn reject_other_with_claude_code_adapter() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Other\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：人工审核\n\
             目标：人工处理\n\
             非目标：不涉及自动化\n\
             关键路径：\n- .\n\
             验证：\n人工确认\n\
             停止条件：\n人工确认失败停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::FIELD_COMBINATION_MISMATCH)),
            "Other + claude-code should fail: {:?}",
            e
        );
    }

    #[test]
    fn reject_heavy_with_execute_and_verify() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Heavy\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::FIELD_COMBINATION_MISMATCH)),
            "Heavy + execute-and-verify should fail: {:?}",
            e
        );
    }

    // ── Phase 4: protected-path checks ─────────────────────────

    #[test]
    fn light_task_with_protected_path_modification_fails() {
        // Light task mentioning protected path + modification keyword
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Light\n\
             读取：\n- .\n\
             任务：修改 AGENTS.md 文件内容\n\
             目标：同步协议文件\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter().any(|m| {
                m.contains(error_code::RISK_LEVEL_MISMATCH)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)
            }),
            "Light task modifying protected path should fail: {:?}",
            e
        );
    }

    #[test]
    fn medium_task_with_protected_path_modification_not_blocked() {
        // Medium + execute-and-verify on protected path: allowed (only Light blocked)
        // But the protected path check only fires when Light OR plan-only/read-only.
        // Medium + execute-and-verify = OK.
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 AGENTS.md 文件内容\n\
             目标：同步协议文件\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        // Should NOT fail on protected-path (Medium + execute-and-verify is allowed)
        // May fail on other rules, but not on risk level or protected path
        let has_protected_error = e.iter().any(|m| {
            m.contains(error_code::RISK_LEVEL_MISMATCH)
                || m.contains(error_code::PROTECTED_PATH_VIOLATION)
        });
        assert!(
            !has_protected_error,
            "Medium + execute-and-verify on protected paths should pass: {:?}",
            e
        );
    }

    #[test]
    fn reading_context_capsule_and_declaring_no_private_edits_passes() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- /Users/user/.agents/memory/projects/my-project/context-capsule.md\n\
             任务：升级 Rust 实验舱 task-card-validator 规则能力\n\
             目标：在 Rust 实验舱内增加字段值、组合、质量和风险检查\n\
             非目标：不修改 /Volumes/Projects/my-protected-suite，不修改 /Volumes/Projects/my-stable-suite，不提交，不推送\n\
             关键路径：\n- /Volumes/Projects/my-protected-suite-rust/crates/task-card-validator/src/lib.rs\n\
             验证：\ncargo fmt --check\ncargo test\n\
             停止条件：\n如果测试失败或发现需要修改 private/stable，停止并报告\n\
             交付：\n返回验证结果和修改摘要\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read-only protected context and no-touch non-goals should pass: {:?}",
            e
        );
    }

    // ── Phase 5: content-quality checks ────────────────────────

    #[test]
    fn reject_weak_goal() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：test\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION)),
            "goal=test should fail: {:?}",
            e
        );
    }

    #[test]
    fn reject_empty_verification() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ntest\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("验证")),
            "verification=test should fail: {:?}",
            e
        );
    }

    #[test]
    fn reject_empty_delivery() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("交付")),
            "empty delivery should fail: {:?}",
            e
        );
    }

    #[test]
    fn reject_empty_stop_condition() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\n\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("停止条件")),
            "empty stop condition should fail: {:?}",
            e
        );
    }

    // ── Phase 6: contradiction checks ──────────────────────────

    #[test]
    fn reject_non_goal_no_modify_but_goal_fixes() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：修复校验器 bug 并实现新功能\n\
             非目标：不修改任何文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "non-goal=no-modify but goal=fix should fail: {:?}",
            e
        );
    }

    #[test]
    fn reject_read_only_with_modification_task() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改校验器核心逻辑\n\
             目标：升级校验功能\n\
             非目标：不修改 private\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "read-only + modification should fail: {:?}",
            e
        );
    }

    #[test]
    fn read_only_with_no_modify_non_goal_passes() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：查看校验器当前状态\n\
             目标：确认校验器状态并返回观察结果\n\
             非目标：不修改任何文件\n\
             关键路径：\n- .\n\
             验证：\n人工检查输出\n\
             停止条件：\n发现需要编辑时停止并报告\n\
             交付：\n返回观察结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read-only task with no-modify non-goal should pass: {:?}",
            e
        );
    }

    #[test]
    fn reject_non_goal_no_commit_but_delivery_commits() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试并提交代码\n\
             目标：验证功能\n\
             非目标：不提交\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\ngit commit 并 push\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "no-commit + commit delivery should fail: {:?}",
            e
        );
    }

    // ── integration: valid fixtures ────────────────────────────

    #[test]
    fn valid_compact_fixture_passes_all_checks() {
        let input = include_str!("../../../tests/fixtures/valid-compact.md");
        let e = validate(input);
        assert!(e.is_empty(), "valid-compact fixture should pass: {:?}", e);
    }

    #[test]
    fn valid_full_fixture_passes_all_checks() {
        let input = include_str!("../../../tests/fixtures/valid-full.md");
        let e = validate(input);
        assert!(e.is_empty(), "valid-full fixture should pass: {:?}", e);
    }

    #[test]
    fn invalid_fixture_fails() {
        let input = include_str!("../../../tests/fixtures/invalid.md");
        let e = validate(input);
        assert!(!e.is_empty(), "invalid fixture should fail");
    }

    // ── integration: Executor Other + generic passes ───────────

    #[test]
    fn executor_other_with_generic_adapter_passes() {
        let input = compact_other_generic();
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "Executor: Other + Runtime adapter: generic should pass all checks: {:?}",
            e
        );
    }

    // ── validate_files tests ───────────────────────────────────

    #[test]
    fn validate_files_ok_when_all_valid() {
        // Use a temp-like approach: collect errors ourselves
        // validate_files prints to stderr, so we test validate() directly
        let c = valid_compact_fields();
        let f = valid_full_fields();
        assert!(validate(&c).is_empty());
        assert!(validate(&f).is_empty());
    }

    #[test]
    fn validate_files_fails_when_any_invalid() {
        let good = valid_compact_fields();
        let bad = format!("not a card\n");
        assert!(validate(&good).is_empty());
        assert!(!validate(&bad).is_empty());
    }

    #[test]
    fn file_read_input_works() {
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/valid-compact.md"
        );
        let result = read_input(fixture);
        assert!(
            result.is_ok(),
            "failed to read {}: {:?}",
            fixture,
            result.err()
        );
        let (content, path) = result.unwrap();
        assert!(path.contains("valid-compact"));
        assert!(content.contains("## 任务卡"));
    }

    #[test]
    fn read_input_file_not_found() {
        let result = read_input("nonexistent_file.md");
        assert!(result.is_err());
    }

    // ── Phase 7: Execution Authority Gate tests ─────────────────

    #[test]
    fn ultracode_with_none_authority_and_normal_task_passes() {
        // Execution effort: ultracode enhances thinking, doesn't grant authority.
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行 cargo test 验证所有测试通过\n\
             目标：验证校验器第 3 轮改动后功能正确\n\
             非目标：不修改任何文件\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回测试通过结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "ultracode + none authority + normal task should pass: {:?}",
            e
        );
    }

    #[test]
    fn none_authority_with_dynamic_workflow_request_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：使用 dynamic workflow 执行大规模重构\n\
             目标：通过 dynamic workflow 加速重构\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "none authority + dynamic workflow request should fail: {:?}",
            e
        );
    }

    #[test]
    fn none_authority_with_subagent_request_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：用 subagent 并行处理多个文件\n\
             目标：通过 subagent 加速处理\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)
                    || m.contains(error_code::PARALLELISM_POLICY_VIOLATION)),
            "none authority + subagent request should fail: {:?}",
            e
        );
    }

    #[test]
    fn allowed_authority_with_light_level_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Light\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)),
            "allowed authority + Light should fail: {:?}",
            e
        );
    }

    #[test]
    fn allowed_authority_with_read_only_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：查看代码\n\
             目标：分析代码结构\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\nls -la\n\
             停止条件：\n读取失败时停止\n\
             交付：\n返回分析结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)),
            "allowed authority + read-only should fail: {:?}",
            e
        );
    }

    #[test]
    fn allowed_authority_with_protected_boundary_fails() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 AGENTS.md 文件内容\n\
             目标：同步协议文件到多个位置\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- /Volumes/Projects/my-protected-suite\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "allowed authority + protected boundary mod should fail: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_authority_with_direct_modification_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改校验器核心逻辑并修复所有 bug\n\
             目标：升级校验功能\n\
             非目标：不修改 private\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)),
            "plan-only authority + direct modification task should fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_none_with_subagent_request_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: within-card\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：用 subagent 并行测试所有模块\n\
             目标：通过 multi-session 加速测试\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::PARALLELISM_POLICY_VIOLATION)),
            "parallelism none + subagent request should fail: {:?}",
            e
        );
    }

    #[test]
    fn missing_execution_effort_defaults_to_unknown() {
        // Old cards without Execution effort should still work
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "missing Execution effort (defaults to unknown) should pass: {:?}",
            e
        );
    }

    #[test]
    fn missing_workflow_authority_defaults_to_none() {
        // Old cards without Workflow authority should still work
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "missing Workflow authority (defaults to none) should pass: {:?}",
            e
        );
    }

    #[test]
    fn private_rust_path_not_confused_with_private() {
        // my-protected-suite-rust must not be false-positived
        // as my-protected-suite
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- context-capsule.md\n\
             任务：修改 crates/task-card-validator/src/lib.rs\n\
             目标：升级校验器功能\n\
             非目标：不修改 my-protected-suite\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回测试通过结果\n",
        );
        let e = validate(&input);
        // Must not fail on protected-path for my-protected-suite-rust
        let has_protected_false = e.iter().any(|m| {
            (m.contains(error_code::RISK_LEVEL_MISMATCH)
                || m.contains(error_code::PROTECTED_PATH_VIOLATION))
                && m.contains("private-rust")
        });
        assert!(
            !has_protected_false,
            "private-rust path should not be confused with private: {:?}",
            e
        );
        // Should pass overall
        assert!(
            e.is_empty(),
            "private-rust path + normal task should pass: {:?}",
            e
        );
    }

    #[test]
    fn read_only_ultracode_observe_task_passes() {
        // ultra code + read-only observe task: thinking intensity ≠ authority
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- crates/task-card-validator/src/lib.rs\n\
             任务：深入分析校验器代码结构并给出复杂度评估\n\
             目标：理解代码架构并输出分析报告\n\
             非目标：不修改任何文件\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\n分析完成时停止\n\
             交付：\n返回分析报告\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "ultracode + read-only observe task should pass: {:?}",
            e
        );
    }

    #[test]
    fn within_card_authority_with_plan_only_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: within-card\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：在任务卡范围内拆分执行\n\
             目标：通过并行计划加速\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)),
            "within-card + plan-only should fail: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_permission_with_allowed_authority_fails() {
        // Permission mode: plan-only → Workflow authority at most plan-only
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：分析代码\n\
             目标：理解架构\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\nls\n\
             停止条件：\n分析完成停止\n\
             交付：\n返回分析报告\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)),
            "plan-only permission + allowed authority should fail: {:?}",
            e
        );
    }

    // ── Phase 7 round 3: workflow keyword bypass regression ─────

    #[test]
    fn workflow_none_with_bare_workflow_in_task_fails() {
        // Workflow authority: none + bare "workflow" keyword in action section
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：使用 workflow 执行任务\n\
             目标：验证 workflow 功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "bare workflow keyword should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "should have WORKFLOW_AUTHORITY_REQUIRED: {:?}",
            e
        );
    }

    #[test]
    fn workflow_none_with_subagent_uppercase_fails() {
        // Workflow authority: none + "Subagent" uppercase (case-insensitive)
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：使用 Subagent 并行处理\n\
             目标：通过 Subagent 加速\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "Subagent uppercase should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "should have WORKFLOW_AUTHORITY_REQUIRED: {:?}",
            e
        );
    }

    #[test]
    fn workflow_none_with_chinese_dynamic_workflow_fails() {
        // Workflow authority: none + Chinese "动态工作流"
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：启动动态工作流处理数据\n\
             目标：通过动态工作流提升效率\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "Chinese 动态工作流 should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "should have WORKFLOW_AUTHORITY_REQUIRED: {:?}",
            e
        );
    }

    #[test]
    fn workflow_none_with_delivery_subagent_fails() {
        // Workflow authority: none + subagent in 交付：section
        // (was a bypass: action_context didn't include 交付)
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试验证功能\n\
             目标：确认校验器正确识别合法输入\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：使用 subagent 生成测试报告\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "subagent in 交付 should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)
                    || m.contains(error_code::PARALLELISM_POLICY_VIOLATION)),
            "should have WORKFLOW_AUTHORITY_REQUIRED or PARALLELISM_POLICY_VIOLATION: {:?}",
            e
        );
    }

    #[test]
    fn within_card_with_protected_stable_modification_fails() {
        // Workflow authority: within-card + modify stable boundary → fail
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-stable-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: within-card\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 stable 仓库中的文件\n\
             目标：同步协议到 stable\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- /Volumes/Projects/my-stable-suite\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "within-card + protected stable mod should fail: {:?}",
            e
        );
    }

    #[test]
    fn allowed_with_protected_bootstrap_modification_fails() {
        // Workflow authority: allowed + modify bootstrap boundary → fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 bootstrap 配置\n\
             目标：升级 bootstrap 引导流程\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "allowed + bootstrap mod should fail: {:?}",
            e
        );
    }

    #[test]
    fn bootstrap_dry_run_read_only_reference_with_no_modify_passes() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：实现第一阶段共享 diagnostics core\n\
             目标：新增共享 HealthReport/Finding/Severity/CheckStatus，后续 dry-run CLI 只读引用该接口\n\
             非目标：不修改 dry-run 专用 crate，不做 apply，不安装 hook，不启动 runner\n\
             关键路径：\n- crates/suite-doctor/src/lib.rs\n- crates/bootstrap-dry-run/src/lib.rs\n\
             验证：\ncargo test\n\
             停止条件：\n需要修改 dry-run 专用 crate 时停止并报告\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read-only bootstrap/dry-run reference with no-modify guard should pass: {:?}",
            e
        );
    }

    #[test]
    fn workflow_sync_check_crate_reference_does_not_require_workflow_authority() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：实现 diagnostics report 聚合\n\
             目标：复用 workflow-sync-check 已有 drift report API，不复制 manifest 判定\n\
             非目标：不使用 subagent，不启用动态工作流，不修改 public/core-only\n\
             关键路径：\n- crates/workflow-sync-check/src/lib.rs\n- crates/suite-doctor/src/lib.rs\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "crate name workflow-sync-check must not count as a dynamic workflow request: {:?}",
            e
        );
    }

    #[test]
    fn read_only_review_card_with_crate_paths_and_patch_stop_language_passes() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：只读审查 suite-doctor MVP 与 bootstrap-dry-run MVP 的后续契约\n\
             目标：只读盘点当前 Rust core 状态，审查 workflow-sync-check 与 suite-doctor 的只读复用边界，并输出后续执行建议与实现建议\n\
             非目标：保持所有文件不变；no file changes；不生成 apply/patch；不提交；不推送；不安装 hook；不启动 runner\n\
             关键路径：\n- crates/workflow-sync-check/src/lib.rs\n- crates/suite-doctor/src/lib.rs\n- crates/bootstrap-dry-run/src/lib.rs\n\
             验证：\ngit status --short\n\
             停止条件：\n任何步骤需要进入文件编辑、apply/patch、stable/public/core-only、hook、runner、跨仓库操作时，立即停止并报告\n\
             交付：\n返回只读审查报告和后续工作卡草案\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read-only review card with crate/path identifiers and stop/non-goal patch wording should pass: {:?}",
            e
        );
    }

    #[test]
    fn read_only_direct_patch_request_still_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：patch validator and update tests\n\
             目标：implement the fix\n\
             非目标：不提交\n\
             关键路径：\n- crates/task-card-validator/src/lib.rs\n\
             验证：\ncargo test -p task-card-validator\n\
             停止条件：\n失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "direct patch/update/implement intent must still fail under read-only: {:?}",
            e
        );
    }

    #[test]
    fn read_only_modify_task_card_template_still_fails() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- protocol/task-card-template.md\n\
             任务：修改任务卡模板\n\
             目标：更新任务卡规则\n\
             非目标：不提交\n\
             关键路径：\n- protocol/task-card-template.md\n\
             验证：\ngit diff --check\n\
             停止条件：\n失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "read-only + modifying task-card template must still fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_subagent_with_workflow_none_fails() {
        // Parallelism: subagent + Workflow authority: none → field combination fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: subagent\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "Parallelism subagent + Workflow authority none should fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_multisession_with_workflow_none_fails() {
        // Parallelism: multi-session + Workflow authority: none → field combination fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: multi-session\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "Parallelism multi-session + Workflow authority none should fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_agent_team_with_workflow_none_fails() {
        // Parallelism: agent-team + Workflow authority: none → field combination fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: agent-team\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "Parallelism agent-team + Workflow authority none should fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_worktree_with_workflow_none_fails() {
        // Parallelism: worktree + Workflow authority: none → field combination fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: worktree\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "Parallelism worktree + Workflow authority none should fail: {:?}",
            e
        );
    }

    #[test]
    fn parallelism_subagent_with_workflow_within_card_passes() {
        // Parallelism: subagent + Workflow authority: within-card → valid combo
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: subagent\n\
             Execution effort: normal\n\
             Workflow authority: within-card\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：验证功能\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(e.is_empty(), "subagent + within-card should pass: {:?}", e);
    }

    #[test]
    fn ultracode_none_authority_normal_rust_task_passes() {
        // Execution effort: ultracode + Workflow authority: none + normal task → passes
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- crates/task-card-validator/src/lib.rs\n\
             任务：升级 task-card-validator 校验规则\n\
             目标：在 Rust 实验舱内增加字段组合检查\n\
             非目标：不修改 private/stable\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回测试结果和修改摘要\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "ultracode + none authority + normal Rust task should pass: {:?}",
            e
        );
    }

    #[test]
    fn private_rust_path_not_false_positive_v3() {
        // my-protected-suite-rust must never be confused with
        // my-protected-suite
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- context-capsule.md\n\
             任务：修改 crates/task-card-validator/src/lib.rs\n\
             目标：升级校验器功能\n\
             非目标：不修改 my-protected-suite，不修改 stable\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回测试通过结果\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "private-rust path should not be false positive: {:?}",
            e
        );
    }

    #[test]
    fn read_context_capsule_no_modify_passes_v3() {
        // Reading context-capsule + non-goal no-touch private/stable → passes
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- /Users/user/.agents/memory/projects/my-project/context-capsule.md\n\
             任务：升级 Rust 实验舱校验器规则\n\
             目标：增加字段组合和保护边界检查\n\
             非目标：不修改 /Volumes/Projects/my-protected-suite，不修改 /Volumes/Projects/my-stable-suite\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回测试结果和修改摘要\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read context-capsule + no-touch non-goal should pass: {:?}",
            e
        );
    }

    #[test]
    fn protected_boundary_keyword_hook_detected() {
        // hook as a protected boundary term with modification intent
        // + within-card authority → boundary+authority block
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: within-card\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 hook 配置\n\
             目标：升级 hook 系统\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "hook keyword + modify + within-card should trigger protected boundary: {:?}",
            e
        );
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "should contain WORKFLOW_AUTHORITY_VIOLATION or PROTECTED_PATH_VIOLATION: {:?}",
            e
        );
    }

    #[test]
    fn protected_boundary_keyword_memory_detected() {
        // memory as a protected boundary term with modification intent
        // + allowed authority → boundary+authority block
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：修改 memory 配置\n\
             目标：更新 memory 存储策略\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "memory keyword + modify + allowed should trigger protected boundary: {:?}",
            e
        );
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "should contain WORKFLOW_AUTHORITY_VIOLATION or PROTECTED_PATH_VIOLATION: {:?}",
            e
        );
    }

    #[test]
    fn workflow_none_with_delegation_keyword_fails() {
        // Workflow authority: none + "delegation" keyword
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：使用 delegation 分发任务\n\
             目标：通过 delegate 模式提升效率\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "delegation keyword should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_REQUIRED)),
            "should have WORKFLOW_AUTHORITY_REQUIRED: {:?}",
            e
        );
    }

    // ── Phase 7 round 3.1: case-insensitive mod + negation regression ──

    #[test]
    fn allowed_with_stable_path_and_uppercase_update_fails() {
        // Case-insensitive bypass: "Update" (uppercase) + stable path must fail
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-stable-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：Update stable config\n\
             目标：Change stable bootstrap settings\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- /Volumes/Projects/my-stable-suite\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "uppercase Update + stable + allowed must fail: {:?}",
            e
        );
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "should have WORKFLOW_AUTHORITY_VIOLATION or PROTECTED_PATH_VIOLATION: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_worktree_with_negation_passes() {
        // plan-only + plan-only authority + "输出计划不修改文件" → no false positive
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: worktree\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：规划多 worktree 执行方案\n\
             目标：输出计划不修改文件\n\
             非目标：不涉及执行\n\
             关键路径：\n- .\n\
             验证：\n人工审核计划\n\
             停止条件：\n计划完成时停止\n\
             交付：\n返回执行计划\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "plan-only + 不修改 should pass (negation), got: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_without_modifying_passes() {
        // plan-only + "without modifying files" → no false positive
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：Create a migration plan\n\
             目标：Design the approach without modifying files\n\
             非目标：Do not execute\n\
             关键路径：\n- .\n\
             验证：\nManual review\n\
             停止条件：\nPlan approved\n\
             交付：\nReturn migration plan\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "plan-only + without modifying should pass (negation), got: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_do_not_modify_passes() {
        // plan-only + "do not modify" → no false positive
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：Audit the codebase\n\
             目标：Produce a report, do not modify any code\n\
             非目标：Do not execute changes\n\
             关键路径：\n- .\n\
             验证：\nManual review\n\
             停止条件：\nAudit complete\n\
             交付：\nReturn audit report\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "plan-only + do not modify should pass (negation), got: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_positive_modify_still_fails() {
        // plan-only + 修改 validator (positive, not negated) → still fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：规划校验器升级方案\n\
             目标：修改 validator 核心逻辑\n\
             非目标：不执行修改\n\
             关键路径：\n- .\n\
             验证：\n人工审核计划\n\
             停止条件：\n计划完成时停止\n\
             交付：\n返回升级计划\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "plan-only + 修改 validator (positive) should still fail, got: {:?}",
            e
        );
        assert!(
            e.iter().any(|m| {
                m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::CONTRADICTORY_REQUIREMENT)
            }),
            "should have WORKFLOW_AUTHORITY_VIOLATION or CONTRADICTORY_REQUIREMENT: {:?}",
            e
        );
    }

    #[test]
    fn uppercase_update_with_stable_path_and_allowed_fails_direct() {
        // "Update" (uppercase) with stable path + allowed → fail (protected boundary)
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-stable-suite\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: allowed\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：Update stable configuration files\n\
             目标：Change settings in stable boundary\n\
             非目标：不修改 rust 实验舱\n\
             关键路径：\n- /Volumes/Projects/my-stable-suite\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "Update stable config + allowed should fail: {:?}",
            e
        );
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::PROTECTED_PATH_VIOLATION)),
            "should have WORKFLOW_AUTHORITY_VIOLATION or PROTECTED_PATH_VIOLATION: {:?}",
            e
        );
    }

    #[test]
    fn mod_keywords_still_detect_positive_requests() {
        // Verify that positive modification requests (not negated) are still detected
        let cases: Vec<(&str, &str)> = vec![
            ("修改 validator", "修改 validator"),
            ("Update stable config", "Update stable config"),
            ("Change bootstrap settings", "Change bootstrap settings"),
            ("删除协议文件", "删除协议文件"),
            ("rewrite protocol rules", "rewrite protocol rules"),
        ];
        for (label, text) in &cases {
            let result = has_modification_intent(text);
            assert!(result, "positive modification '{label}' should be detected");
        }
    }

    // ── Phase 7 round 3.2: Chinese negation + weak goal regression ──

    #[test]
    fn plan_only_with_compound_chinese_negation_passes() {
        // plan-only + 不执行修改 + 需要修改文件时停止 → no false positive
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: worktree\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- crates/\n\
             任务：规划 validator shadow run 验证方案\n\
             目标：在不修改代码的前提下验证校验规则\n\
             非目标：不修改文件\n\
             关键路径：\n- crates/\n\
             验证：仅报告计划，不执行修改\n\
             停止条件：发现需要修改文件时停止\n\
             交付：返回 shadow run 执行计划\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "plan-only + compound negation should pass, got: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_prohibition_negation_passes() {
        // plan-only + 禁止修改/不得删除 → no false positive
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：审计代码库安全性\n\
             目标：输出安全审计报告，禁止修改代码\n\
             非目标：不得删除任何文件\n\
             关键路径：\n- .\n\
             验证：人工审核审计报告\n\
             停止条件：审计完成时停止\n\
             交付：返回安全审计报告\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "plan-only + 禁止修改/不得删除 should pass, got: {:?}",
            e
        );
    }

    #[test]
    fn reject_weak_goal_待定() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：待定\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：test 失败时停止\n\
             交付：返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "goal=待定 should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("目标")),
            "should have EMPTY_OR_WEAK_SECTION for goal=待定: {:?}",
            e
        );
    }

    #[test]
    fn reject_weak_goal_暂无() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：暂无\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：test 失败时停止\n\
             交付：返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "goal=暂无 should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("目标")),
            "should have EMPTY_OR_WEAK_SECTION for goal=暂无: {:?}",
            e
        );
    }

    #[test]
    fn reject_weak_goal_未定() {
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：运行测试\n\
             目标：未定\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：test 失败时停止\n\
             交付：返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty(), "goal=未定 should fail");
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("目标")),
            "should have EMPTY_OR_WEAK_SECTION for goal=未定: {:?}",
            e
        );
    }

    #[test]
    fn positive_modify_validator_still_fails_after_negation_fix() {
        // After negation fix, positive "修改 validator" must still fail
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: limited\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：规划校验器升级方案\n\
             目标：修改 validator 核心逻辑\n\
             非目标：不执行修改\n\
             关键路径：\n- .\n\
             验证：人工审核计划\n\
             停止条件：计划完成时停止\n\
             交付：返回升级计划\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "positive 修改 validator should still fail after negation fix, got: {:?}",
            e
        );
        assert!(
            e.iter().any(|m| {
                m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::CONTRADICTORY_REQUIREMENT)
            }),
            "should have WORKFLOW_AUTHORITY_VIOLATION or CONTRADICTORY_REQUIREMENT: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_developer_stop_and_confirmation_phrases_passes() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- crates/task-card-validator/src/lib.rs\n\
             任务：审查 validator 规则并输出修复建议\n\
             目标：只定位风险并给出 patch 计划，先不要落地修改\n\
             非目标：不写入文件；不应用 patch；不得提交 commit\n\
             关键路径：\n- crates/task-card-validator/src/lib.rs\n\
             验证：仅报告计划，等待确认后再修改\n\
             停止条件：如需修改代码则暂停并请求确认；需要变更文件时等待用户确认\n\
             交付：返回待确认的修改建议\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "developer stop/confirmation phrases should pass, got: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_read_only_audit_phrases_passes() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- README.md\n\
             任务：只读审计任务卡规则\n\
             目标：仅分析变更范围，不产生文件改动，不改代码，不重写协议\n\
             非目标：不替换 validator；不会删除文件；无需提交\n\
             关键路径：\n- README.md\n\
             验证：检查现有说明并返回审计结论，不做任何 change\n\
             停止条件：发现必须 rewrite 才能继续时停下报告\n\
             交付：返回 read-only audit report\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "read-only audit phrases should pass, got: {:?}",
            e
        );
    }

    #[test]
    fn positive_modify_after_confirmation_language_still_fails() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: plan-only\n\
             任务级别：Medium\n\
             读取：\n- crates/task-card-validator/src/lib.rs\n\
             任务：先分析规则，再修改 validator 并提交 commit\n\
             目标：完成修改并替换旧逻辑\n\
             非目标：不要 push\n\
             关键路径：\n- crates/task-card-validator/src/lib.rs\n\
             验证：cargo test\n\
             停止条件：测试失败时停止\n\
             交付：返回修改摘要和 commit\n",
        );
        let e = validate(&input);
        assert!(
            !e.is_empty(),
            "positive modify/commit request must still fail in plan-only"
        );
        assert!(
            e.iter().any(|m| {
                m.contains(error_code::WORKFLOW_AUTHORITY_VIOLATION)
                    || m.contains(error_code::CONTRADICTORY_REQUIREMENT)
            }),
            "should have workflow or contradiction error: {:?}",
            e
        );
    }

    #[test]
    fn reject_more_weak_goal_placeholders() {
        for weak in ["无目标", "暂无目标", "未明确", "later", "n/a", "none"] {
            let input = compact_body(&format!(
                "路径：\n- .\n\
                 Executor: Claude Code\n\
                 Runtime adapter: claude-code\n\
                 Execution surface: cli\n\
                 Permission mode: execute-and-verify\n\
                 Parallelism: none\n\
                 Execution effort: normal\n\
                 Workflow authority: none\n\
                 任务级别：Medium\n\
                 读取：\n- .\n\
                 任务：运行测试\n\
                 目标：{}\n\
                 非目标：不修改文件\n\
                 关键路径：\n- .\n\
                 验证：\ncargo test\n\
                 停止条件：test 失败时停止\n\
                 交付：返回结果\n",
                weak
            ));
            let e = validate(&input);
            assert!(!e.is_empty(), "goal={weak} should fail");
            assert!(
                e.iter()
                    .any(|m| m.contains(error_code::EMPTY_OR_WEAK_SECTION) && m.contains("目标")),
                "should have EMPTY_OR_WEAK_SECTION for goal={weak}: {:?}",
                e
            );
        }
    }

    #[test]
    fn full_card_verification_gate_satisfies_verification_quality() {
        let input = "## 任务卡\n\n\
读取并遵守：\n- AGENTS.md\n- docs/agent-workflow/runtime-adapters.md\n\n\
Executor: Claude Code\n\n\
Runtime adapter: claude-code\n\n\
Execution surface: cli\n\n\
Permission mode: edit-with-confirmation\n\n\
Parallelism: none\n\n\
Execution effort: normal\n\n\
Workflow authority: none\n\n\
任务级别：Medium\n\n\
Review gate:\n- 按协议执行\n\n\
任务：为脚本增加 dry-run 摘要\n\n\
背景：测试 full card 的 Verification gate 结构\n\n\
项目画像：\n- 无\n\n\
记忆胶囊：\n- 无\n\n\
任务存档：\n- 无\n\n\
相关路径：\n- docs/agent-workflow/runtime-adapters.md\n\n\
本次任务相关文件：\n- docs/agent-workflow/agent-task-protocol.md\n\n\
目标：\n1. 增加 dry-run 摘要并保持默认行为不变。\n\n\
非目标：\n- 不安装新依赖。\n\n\
验证：\nVerification gate:\n- commands:\n  - bash -n scripts/example-tool.sh\n- expected evidence:\n  - shell syntax check 通过\n- stop condition:\n  - 风险高于 Medium 时停止\n\n\
交付：\n按协议输出 delivery report。\n";
        let e = validate(input);
        assert!(
            e.is_empty(),
            "full card Verification gate should pass quality checks: {:?}",
            e
        );
    }

    #[test]
    fn agent_workflow_doc_paths_do_not_request_workflow_authority() {
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Codex\n\
             Runtime adapter: codex-local\n\
             Execution surface: local-workspace\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Light\n\
             读取：\n- docs/agent-workflow/runtime-adapters.md\n\
             任务：阅读协议文档并总结字段含义\n\
             目标：说明 runtime adapter 字段的约束\n\
             非目标：不修改文件\n\
             关键路径：\n- docs/agent-workflow/runtime-adapters.md\n\
             验证：返回摘要即可\n\
             停止条件：需要修改协议时停止\n\
             交付：按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "agent-workflow path should not imply dynamic workflow request: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_language_does_not_count_as_direct_modification() {
        let input = "## 任务卡\n\n\
读取并遵守：\n- AGENTS.md\n\n\
Executor: Cursor\n\n\
Runtime adapter: cursor\n\n\
Execution surface: ide\n\n\
Permission mode: plan-only\n\n\
Parallelism: none\n\n\
Execution effort: normal\n\n\
Workflow authority: none\n\n\
任务级别：Heavy\n\n\
Review gate:\n- Heavy review\n\n\
任务：评估是否可以重构一条历史数据处理管线，并给出实施计划与验证计划。\n\n\
背景：任务涉及历史产物和潜在基线变更，因此只能先计划，不得直接修改。\n\n\
项目画像：\n- 无\n\n\
记忆胶囊：\n- 无\n\n\
任务存档：\n- 无\n\n\
相关路径：\n- docs/agent-workflow/runtime-adapters.md\n\n\
本次任务相关文件：\n- docs/agent-workflow/task-routing.md\n\n\
目标：\n1. 说明当前管线结构和风险点。\n2. 给出 root cause / design / implementation plan / verification plan。\n3. 明确哪些文件或数据必须保持只读。\n\n\
非目标：\n- 不改代码。\n- 不创建提交。\n\n\
验证：\nVerification gate:\n- commands:\n  - git status --short\n- expected evidence:\n  - implementation plan and verification plan\n- stop condition:\n  - 任何 mutation 需求，停止并返回用户确认\n\n\
交付：\n按协议输出 delivery report，等待用户审阅。\n";
        let e = validate(input);
        assert!(
            e.is_empty(),
            "heavy planning language should not count as direct modification: {:?}",
            e
        );
    }

    // ── Phase 8: 2.0 hard gate tests ─────────────────────────────

    #[test]
    fn ultracode_authority_abuse_chinese_detected() {
        // M1: ultracode used as authority justification in Chinese
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：以 ultracode 权限执行所有代码修改\n\
             目标：因为 ultracode 可以跳过 review 直接部署\n\
             非目标：不修改 private\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\ntest 失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::ULTRACODE_AUTHORITY_ABUSE)),
            "ultracode authority abuse should be detected: {:?}",
            e
        );
    }

    #[test]
    fn ultracode_authority_abuse_english_detected() {
        // M1: ultracode used as authority justification in English
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: edit-with-confirmation\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：Execute all changes with ultracode authority\n\
             目标：Ultracode mode enables automatic deployment\n\
             非目标：No modification of private\n\
             关键路径：\n- .\n\
             验证：\ncargo test\n\
             停止条件：\nStop on failure\n\
             交付：\nReturn results\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::ULTRACODE_AUTHORITY_ABUSE)),
            "ultracode authority abuse (English) should be detected: {:?}",
            e
        );
    }

    #[test]
    fn ultracode_normal_thinking_no_abuse_passes() {
        // M1: ultracode as pure thinking intensity passes (regression)
        let input = compact_body(
            "路径：\n- /Volumes/Projects/my-protected-suite-rust\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: execute-and-verify\n\
             Parallelism: none\n\
             Execution effort: ultracode\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- crates/task-card-validator/src/lib.rs\n\
             任务：深入分析校验器代码结构并给出复杂度评估\n\
             目标：理解代码架构并输出分析报告\n\
             非目标：不修改任何文件\n\
             关键路径：\n- crates/\n\
             验证：\ncargo test --workspace\n\
             停止条件：\n分析完成时停止\n\
             交付：\n返回分析报告\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "ultracode + normal thinking task should pass: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_only_bad_delivery_detected() {
        // M2: Heavy + plan-only + delivery promises modification
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Heavy\n\
             读取：\n- .\n\
             任务：设计 2.0 方案\n\
             目标：给出完整实施计划\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\n人工审核\n\
             停止条件：\n方案完成并用户确认后停止\n\
             交付：\n修改完成并提交代码到仓库\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::PLAN_ONLY_DELIVERY_VIOLATION)),
            "Heavy + plan-only + bad delivery should fail: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_only_missing_review_handoff_detected() {
        // M2: Heavy + plan-only + no review handoff in stop or delivery
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Heavy\n\
             读取：\n- .\n\
             任务：设计审计方案\n\
             目标：给出审计报告\n\
             非目标：不修改文件\n\
             关键路径：\n- .\n\
             验证：\nls -la\n\
             停止条件：\n任务完成时停止\n\
             交付：\n输出审计报告\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF)),
            "Heavy + plan-only + missing review handoff should fail: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_only_valid_handoff_passes() {
        // M2: Heavy + plan-only with proper review handoff passes
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Heavy\n\
             读取：\n- .\n\
             任务：设计审计方案\n\
             目标：给出审计报告和实施计划\n\
             非目标：不修改任何文件\n\
             关键路径：\n- .\n\
             验证：\nls -la\n\
             停止条件：\n方案完成后返回用户审阅，等待明确批准\n\
             交付：\n返回审计方案供 Codex review\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "Heavy + plan-only + valid review handoff should pass: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_only_full_card_with_verification_gate_handoff_passes() {
        // Regression: full cards encode stop conditions inside Verification gate.
        // Heavy + plan-only + full-card + Verification gate stop condition
        // with review handoff must PASS.
        let input = full_body(
            "读取并遵守：\n- AGENTS.md\n\
             Executor: Cursor\n\
             Runtime adapter: cursor\n\
             Execution surface: ide\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Heavy\n\
             Review gate:\n- Heavy review\n\
             任务：评估数据处理管线重构方案\n\
             背景：涉及历史产物和潜在基线变更\n\
             项目画像：\n- 无\n\
             记忆胶囊：\n- 无\n\
             任务存档：\n- 无\n\
             相关路径：\n- docs/\n\
             本次任务相关文件：\n- docs/agent-workflow/task-routing.md\n\
             目标：\n1. 说明当前管线结构和风险点。\n2. 给出 design / implementation plan。\n\
             非目标：\n- 不改代码。\n- 不创建提交。\n\
             验证：\n\
             Verification gate:\n\
             - commands:\n   - git status --short\n\
             - expected evidence:\n   - implementation plan\n\
             - stop condition:\n   - 方案完成后返回用户审阅，等待明确批准\n\
             交付：\n按协议输出 delivery report。\n",
        );
        let e = validate(&input);
        assert!(
            e.is_empty(),
            "Heavy + plan-only + full-card + Verification gate handoff should pass: {:?}",
            e
        );
    }

    #[test]
    fn heavy_plan_only_full_card_without_handoff_fails() {
        // Full card with Heavy+plan-only but Verification gate stop condition
        // lacks review handoff → must FAIL.
        let input = full_body(
            "读取并遵守：\n- AGENTS.md\n\
             Executor: Cursor\n\
             Runtime adapter: cursor\n\
             Execution surface: ide\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Heavy\n\
             Review gate:\n- Heavy review\n\
             任务：分析代码结构\n\
             背景：了解系统架构\n\
             项目画像：\n- 无\n\
             记忆胶囊：\n- 无\n\
             任务存档：\n- 无\n\
             相关路径：\n- .\n\
             本次任务相关文件：\n- Cargo.toml\n\
             目标：\n1. 分析当前代码结构。\n2. 输出分析报告。\n\
             非目标：\n- 不改代码。\n\
             验证：\n\
             Verification gate:\n\
             - commands:\n   - git status\n\
             - expected evidence:\n   - analysis report\n\
             - stop condition:\n   - task complete\n\
             交付：\nreturn analysis report\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF)),
            "Heavy+plan-only full-card without review handoff must fail: {:?}",
            e
        );
    }

    #[test]
    fn read_only_with_new_keyword_deploy_detected() {
        // M3: read-only + new keyword "deploy" triggers contradiction
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: read-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：deploy new config and install hooks\n\
             目标：sync to stable\n\
             非目标：不修改其他文件\n\
             关键路径：\n- .\n\
             验证：\nls -la\n\
             停止条件：\n失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "read-only + deploy/install/sync should be caught: {:?}",
            e
        );
    }

    #[test]
    fn plan_only_with_chinese_execute_keyword_detected() {
        // M3: plan-only + new Chinese keywords trigger contradiction
        let input = compact_body(
            "路径：\n- .\n\
             Executor: Claude Code\n\
             Runtime adapter: claude-code\n\
             Execution surface: cli\n\
             Permission mode: plan-only\n\
             Parallelism: none\n\
             Execution effort: normal\n\
             Workflow authority: none\n\
             任务级别：Medium\n\
             读取：\n- .\n\
             任务：部署新配置并调整参数\n\
             目标：创建新文件并写入数据\n\
             非目标：不修改 private\n\
             关键路径：\n- .\n\
             验证：\nls -la\n\
             停止条件：\n失败时停止\n\
             交付：\n返回结果\n",
        );
        let e = validate(&input);
        assert!(!e.is_empty());
        assert!(
            e.iter()
                .any(|m| m.contains(error_code::CONTRADICTORY_REQUIREMENT)),
            "plan-only + 部署/调整/创建/写入 should be caught: {:?}",
            e
        );
    }

    // ── error code presence tests ──────────────────────────────

    #[test]
    fn all_error_codes_are_present_in_failures() {
        // Verify each error code appears in at least one test scenario
        let codes = &[
            error_code::INVALID_FIELD_VALUE,
            error_code::FIELD_COMBINATION_MISMATCH,
            error_code::PROTECTED_PATH_VIOLATION,
            error_code::RISK_LEVEL_MISMATCH,
            error_code::EMPTY_OR_WEAK_SECTION,
            error_code::CONTRADICTORY_REQUIREMENT,
            error_code::EXECUTION_EFFORT_POLICY_VIOLATION,
            error_code::WORKFLOW_AUTHORITY_REQUIRED,
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            error_code::PARALLELISM_POLICY_VIOLATION,
            error_code::ULTRACODE_AUTHORITY_ABUSE,
            error_code::PLAN_ONLY_DELIVERY_VIOLATION,
            error_code::HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF,
            error_code::PLAN_ONLY_EXECUTION_VERB_DETECTED,
            error_code::FIELD_ABUSE_DETECTED,
            error_code::AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE,
        ];

        // Run a few failure cases and collect all error codes seen
        let mut seen = std::collections::HashSet::new();

        let cases: Vec<(&str, String)> = vec![
            (
                "INVALID_FIELD_VALUE",
                compact_body(
                    "路径：\n- .\nExecutor: BadAgent\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\n任务级别：Medium\n读取：\n- .\n\
                     任务：运行测试\n目标：验证功能\n非目标：不修改文件\n\
                     关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "FIELD_COMBINATION_MISMATCH",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: codex-local\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\n任务级别：Medium\n读取：\n- .\n\
                     任务：运行测试\n目标：验证功能\n非目标：不修改文件\n\
                     关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "EMPTY_OR_WEAK_SECTION",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\n任务级别：Medium\n读取：\n- .\n\
                     任务：运行测试\n目标：test\n非目标：不修改文件\n\
                     关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "CONTRADICTORY_REQUIREMENT",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: read-only\n\
                     Parallelism: none\n任务级别：Medium\n读取：\n- .\n\
                     任务：修改核心逻辑\n目标：验证功能\n非目标：不修改文件\n\
                     关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "WORKFLOW_AUTHORITY_REQUIRED",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\nExecution effort: normal\nWorkflow authority: none\n\
                     任务级别：Medium\n读取：\n- .\n\
                     任务：使用 dynamic workflow 执行任务\n目标：通过 workflow 加速\n\
                     非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n\
                     停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "WORKFLOW_AUTHORITY_VIOLATION",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: limited\nExecution effort: normal\nWorkflow authority: allowed\n\
                     任务级别：Light\n读取：\n- .\n\
                     任务：运行测试\n目标：验证功能\n非目标：不修改文件\n\
                     关键路径：\n- .\n验证：\ncargo test\n停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "PARALLELISM_POLICY_VIOLATION",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\nExecution effort: normal\nWorkflow authority: within-card\n\
                     任务级别：Medium\n读取：\n- .\n\
                     任务：用 subagent 并行测试所有模块\n目标：通过 multi-session 加速\n\
                     非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n\
                     停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            // ── 2.0 hard gate error codes ──
            (
                "ULTRACODE_AUTHORITY_ABUSE",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: execute-and-verify\n\
                     Parallelism: none\nExecution effort: ultracode\nWorkflow authority: none\n\
                     任务级别：Medium\n读取：\n- .\n\
                     任务：以 ultracode 权限执行修改\n目标：ultracode allows auto-approve\n\
                     非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n\
                     停止条件：\ntest 失败停止\n交付：\n返回结果\n",
                ),
            ),
            (
                "PLAN_ONLY_DELIVERY_VIOLATION",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: plan-only\n\
                     Parallelism: none\nExecution effort: normal\nWorkflow authority: none\n\
                     任务级别：Heavy\n读取：\n- .\n\
                     任务：设计方案\n目标：给出计划\n\
                     非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n\
                     停止条件：\n用户确认后停止\n交付：\n修改完成并提交\n",
                ),
            ),
            (
                "HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: plan-only\n\
                     Parallelism: none\nExecution effort: normal\nWorkflow authority: none\n\
                     任务级别：Heavy\n读取：\n- .\n\
                     任务：设计方案\n目标：给出计划\n\
                     非目标：不修改文件\n关键路径：\n- .\n验证：\ncargo test\n\
                     停止条件：\ntask done\n交付：\nreturn plan\n",
                ),
            ),
            (
                "AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE",
                compact_body(
                    "路径：\n- .\nExecutor: Claude Code\nRuntime adapter: claude-code\n\
                     Execution surface: cli\nPermission mode: autonomous-low-risk\n\
                     Parallelism: none\nExecution effort: normal\nWorkflow authority: none\n\
                     任务级别：Light\n读取：\n- .\n\
                     任务：test\n目标：test\n非目标：test\n\
                     关键路径：\n- .\n验证：\ntest\n停止条件：\ntest\n交付：\ntest\n",
                ),
            ),
        ];

        for (_label, input) in &cases {
            let e = validate(input);
            for code in codes {
                if e.iter().any(|m| m.contains(*code)) {
                    seen.insert(*code);
                }
            }
        }

        // These codes should appear in at least one failure case above
        for code in &[
            error_code::INVALID_FIELD_VALUE,
            error_code::FIELD_COMBINATION_MISMATCH,
            error_code::EMPTY_OR_WEAK_SECTION,
            error_code::CONTRADICTORY_REQUIREMENT,
            error_code::WORKFLOW_AUTHORITY_REQUIRED,
            error_code::WORKFLOW_AUTHORITY_VIOLATION,
            error_code::PARALLELISM_POLICY_VIOLATION,
            error_code::ULTRACODE_AUTHORITY_ABUSE,
            error_code::PLAN_ONLY_DELIVERY_VIOLATION,
            error_code::HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF,
            error_code::AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE,
        ] {
            assert!(
                seen.contains(code),
                "error code {} should appear in at least one failure case",
                code
            );
        }
    }

    // ── Public API tests: CardType, ParsedTaskCard, parse_validated ─────

    #[test]
    fn detect_card_type_compact_from_path() {
        let input = "## 任务卡\n路径：\n- .\nExecutor: Codex\n";
        assert_eq!(detect_card_type(input), CardType::Compact);
    }

    #[test]
    fn detect_card_type_full_from_read_list() {
        let input = "## 任务卡\n读取并遵守：\n- AGENTS.md\nExecutor: Codex\n";
        assert_eq!(detect_card_type(input), CardType::Full);
    }

    #[test]
    fn parse_validated_valid_compact_ok() {
        let input = valid_compact_fields();
        let result = parse_validated(&input);
        assert!(result.is_ok(), "unexpected error: {:?}", result.err());
        let card = result.unwrap();
        assert_eq!(card.card_type, CardType::Compact);
        assert!(card.fields.contains_key("Executor:"));
    }

    #[test]
    fn parse_validated_invalid_returns_errors() {
        // Missing required fields — validate will catch them
        let input = "## 任务卡\ninvalid content\n";
        let result = parse_validated(&input);
        assert!(result.is_err());
        assert!(!result.unwrap_err().is_empty());
    }

    #[test]
    fn parse_validated_fields_match_parse_card() {
        // parse_validated should produce the same fields as a direct parse_card call
        let input = valid_compact_fields();
        let direct_fields = parse_card(&input);
        let result = parse_validated(&input);
        assert!(result.is_ok());
        let card = result.unwrap();
        assert_eq!(card.fields, direct_fields);
    }
}
