//! Phase 1-5 checks: format, field values, combinations, protected paths, content quality.
use super::*;

/// Keywords that indicate a task is requesting dynamic workflow / subagent /
/// multi-session / agent-team / delegation execution.
///
/// Matched **case-insensitively** against the full action context (all
/// action-bearing sections).
pub(crate) const WORKFLOW_REQUEST_KEYWORDS: &[&str] = &[
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
pub(crate) const PARALLELISM_BODY_KEYWORDS: &[&str] = &[
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
pub(crate) const ULTRACODE_AUTHORITY_ABUSE_KEYWORDS: &[&str] = &[
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
pub(crate) fn trunc80(s: &str) -> String {
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
pub(crate) fn is_text_fence_line(line: &str) -> bool {
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
pub(crate) fn find_text_fence(input: &str) -> Option<usize> {
    for (i, line) in input.lines().enumerate() {
        if is_text_fence_line(line) {
            return Some(i + 1);
        }
    }
    None
}

pub(crate) fn check_retired_skill_tags(input: &str, errors: &mut Vec<String>) {
    let lines: Vec<&str> = input.lines().collect();

    for (line_idx, tag) in trailing_skill_metadata_lines(&lines) {
        if let Some((retired, replacement)) = RETIRED_SKILL_TAGS
            .iter()
            .find(|(name, _)| tag.eq_ignore_ascii_case(name))
        {
            let replacement_hint = if replacement.is_empty() {
                "该标记已删除，不再提供替代标记".to_string()
            } else {
                format!("请改用 `[skill: {replacement}]`")
            };
            errors.push(format!(
                "[{}] 第 {} 行：旧技能标记 `[skill: {retired}]` 已删除，{replacement_hint}",
                error_code::RETIRED_SKILL_TAG,
                line_idx + 1
            ));
        }
    }
}

fn trailing_skill_metadata_lines<'a>(lines: &'a [&'a str]) -> Vec<(usize, &'a str)> {
    let mut found = Vec::new();
    let mut idx = lines.len();

    while idx > 0 {
        idx -= 1;
        let line = lines[idx];
        if line.trim().is_empty() {
            continue;
        }

        if let Some(tag) = standalone_skill_tag(line) {
            found.push((idx, tag));
            continue;
        }

        break;
    }

    found.reverse();
    found
}

fn standalone_skill_tag(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with("[skill:") || !trimmed.ends_with(']') {
        return None;
    }

    let tag = trimmed["[skill:".len()..trimmed.len() - 1].trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag)
    }
}

// ── Phase 2: field-value checks ────────────────────────────────────────

pub(crate) fn check_field_values(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
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

pub(crate) fn check_field_combinations(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
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

pub(crate) fn check_protected_paths(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
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

pub(crate) fn is_weak_value(v: &str) -> bool {
    let trimmed = v.trim();
    let lowered = trimmed.to_ascii_lowercase();
    trimmed.is_empty()
        || WEAK_GOAL_VALUES.contains(&trimmed)
        || WEAK_GOAL_VALUES.contains(&lowered.as_str())
}

pub(crate) fn check_content_quality(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
    // 目标 — required field
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

    // 非目标 — required field
    if let Some(non_goal) = fields.get("非目标：") {
        if non_goal.trim().is_empty() {
            errors.push(format!(
                "[{}] 非目标 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }

    // 验证 — required field
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

    // 停止条件 — optional field, validated only when present
    if let Some(stop) = fields.get("停止条件：") {
        if stop.trim().is_empty() {
            errors.push(format!(
                "[{}] 停止条件 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }

    // 交付 — required field
    if let Some(delivery) = fields.get("交付：") {
        if delivery.trim().is_empty() {
            errors.push(format!(
                "[{}] 交付 不能为空",
                error_code::EMPTY_OR_WEAK_SECTION
            ));
        }
    }
}
