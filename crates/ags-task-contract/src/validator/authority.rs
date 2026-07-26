//! Phase 7: execution authority gate.
use super::*;

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
pub(crate) fn check_execution_authority_gate(
    fields: &HashMap<String, String>,
    errors: &mut Vec<String>,
) {
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

    // ── Execution effort: the exhaustive tier must not be abused as authority ──
    // `exhaustive` is the neutral value; `ultracode` is the legacy alias. Either
    // way, effort is thinking-intensity only, never permission / review-skip /
    // auto-execute authority.
    let execution_effort = get_execution_effort(fields);
    if is_exhaustive_effort(execution_effort) {
        let effort_abuse = EXHAUSTIVE_EFFORT_AUTHORITY_ABUSE_KEYWORDS
            .iter()
            .any(|kw| action_lower.contains(&kw.to_lowercase()));
        if effort_abuse {
            errors.push(format!(
                "[{}] Execution effort 为 {}（exhaustive 强度），但任务行动区域将其当作执行权限/跳过 review/自动执行的依据。Execution effort 只能表示思考强度，不能映射为 plan、parallel、permission escalation 或 workflow authority",
                error_code::ULTRACODE_AUTHORITY_ABUSE,
                execution_effort
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
        if permission == "plan-only" {
            errors.push(format!(
                "[{}] Workflow authority 为 within-card，但 Permission mode 为 {}（需要 execute-and-verify）",
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

    // ── 子任务编排 (subtask orchestration) ↔ Workflow authority / Parallelism ──
    // A non-`none` mode declares splittable subtask structure. It must be backed
    // by BOTH a non-`none` Workflow authority AND a delegation-capable
    // Parallelism — the slot only DECLARES structure; actual subagent / workflow
    // ignition is translated by the claude-code adapter / runner from the
    // resolved policy, never fired by the task-card body itself.
    let subtask_mode = get_subtask_orchestration_mode(fields);
    if subtask_mode != "none" {
        if authority == "none" {
            errors.push(format!(
                "[{}] 子任务编排 mode 为 {}，要求 Workflow authority 非 none（不允许 mode != none 但 authority=none）",
                error_code::SUBTASK_ORCHESTRATION_VIOLATION,
                subtask_mode
            ));
        }
        if !matches!(
            parallelism,
            "subagent" | "worktree" | "multi-session" | "agent-team"
        ) {
            errors.push(format!(
                "[{}] 子任务编排 mode 为 {}，要求 Parallelism 为 subagent/worktree/multi-session/agent-team，当前为 `{}`",
                error_code::SUBTASK_ORCHESTRATION_VIOLATION,
                subtask_mode,
                parallelism
            ));
        }
    }
}
