//! Phase 6: contradiction detection.
use super::*;

// ── Phase 6: contradiction checks ──────────────────────────────────────

/// Check whether `text` expresses modification intent after ignoring common
/// no-op, planning, confirmation, and stop-condition phrases.
///
/// The rule is segment-based rather than whole-text replacement: a task card
/// can safely say "不修改文件" in one clause and still be rejected when another
/// clause says "再修改 validator".
pub(crate) fn has_modification_intent(text: &str) -> bool {
    normalize_modification_intent_text(text)
        .split(is_intent_segment_separator)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .any(segment_has_positive_modification_intent)
}

pub(crate) fn normalize_modification_intent_text(text: &str) -> String {
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

pub(crate) fn is_intent_segment_separator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | ';' | '；' | ',' | '，' | '。' | '、' | '|' | '/' | '：' | ':'
    )
}

pub(crate) fn segment_has_positive_modification_intent(segment: &str) -> bool {
    let has_keyword = MODIFICATION_KEYWORDS
        .iter()
        .any(|kw| segment.contains(&kw.to_lowercase()));

    has_keyword && !is_safe_non_modification_segment(segment)
}

pub(crate) fn is_safe_non_modification_segment(segment: &str) -> bool {
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

pub(crate) fn is_stop_or_confirmation_segment(segment: &str) -> bool {
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

pub(crate) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Extract the "stop condition" sub-section from a Verification gate field.
///
/// Full cards encode stop conditions inside `Verification gate:` as:
/// `- stop condition:\n  - <text>`
/// This function extracts the text after `- stop condition:` for handoff
/// checking, so Heavy+plan-only full cards are not falsely rejected.
pub(crate) fn extract_verification_gate_stop_condition(gate: &str) -> String {
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

pub(crate) fn workflow_request_context(fields: &HashMap<String, String>) -> String {
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

pub(crate) fn normalize_workflow_request_text(text: &str) -> String {
    text.replace("agent-workflow", "agentprotocol")
        .replace("workflow-sync-check", "sync_check_crate")
}

pub(crate) fn has_workflow_request_intent(text: &str) -> bool {
    text.to_lowercase()
        .split(is_intent_segment_separator)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .any(segment_has_positive_workflow_request)
}

pub(crate) fn segment_has_positive_workflow_request(segment: &str) -> bool {
    let normalized = normalize_workflow_request_text(segment);
    let has_keyword = WORKFLOW_REQUEST_KEYWORDS
        .iter()
        .any(|kw| normalized.contains(&kw.to_lowercase()));

    has_keyword && !is_safe_non_workflow_request_segment(&normalized)
}

pub(crate) fn is_safe_non_workflow_request_segment(segment: &str) -> bool {
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

pub(crate) fn contains_path_with_boundary(text: &str, needle: &str) -> bool {
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

pub(crate) fn contains_protected_mention(text: &str) -> bool {
    PROTECTED_PATHS
        .iter()
        .any(|pp| contains_path_with_boundary(text, pp))
        || PROTECTED_BOUNDARY_TERMS
            .iter()
            .any(|term| contains_path_with_boundary(text, term))
}

pub(crate) fn has_explicit_protected_read_only_boundary(fields: &HashMap<String, String>) -> bool {
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
pub(crate) fn action_context(fields: &HashMap<String, String>) -> String {
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
pub(crate) fn extended_action_context(fields: &HashMap<String, String>) -> String {
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

pub(crate) fn check_contradictions(fields: &HashMap<String, String>, errors: &mut Vec<String>) {
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
        || non_goal.contains("不修改 /Volumes/Projects/example-private-suite")
        || non_goal.contains("不修改 /Volumes/Projects/example-stable-suite");

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
