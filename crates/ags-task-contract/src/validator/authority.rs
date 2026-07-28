//! Phase 7: execution authority gate.
use super::*;

// ── Phase 7: Execution Authority Gate ──────────────────────────────────

/// Check that execution mode, topology, and delegation planning stay
/// independent and fail closed when the body contradicts them.
///
/// Core principles:
/// - Execution power is never authority.  Higher reasoning may improve
///   planning, but it does not upgrade permission.
/// - Dynamic workflow / subagent / delegation requires explicit task-card
///   Delegation planning.
/// - Matching is case-insensitive and scans ALL action-bearing sections
///   (not just 任务：+ 目标：), closing the narrow-scope bypass.
/// - ExecutionTopology field value is cross-checked against Delegation planning
///   to prevent field-combination bypasses.
pub(crate) fn check_execution_authority_gate(
    fields: &HashMap<String, String>,
    errors: &mut Vec<String>,
) {
    let delegation_planning = get_delegation_planning(fields);
    let execution_topology = field_val(fields, "Execution topology:");
    let execution_mode = field_val(fields, "Execution mode:");

    // Build the full action-request text from ALL action-bearing sections
    // and lowercase once for case-insensitive matching.
    // Use extended_action_context to catch abuse in 读取/背景 fields.
    let action_text = extended_action_context(fields);
    let workflow_text = workflow_request_context(fields);
    let action_lower = action_text.to_lowercase();
    let workflow_lower = normalize_workflow_request_text(&workflow_text.to_lowercase());

    // ── Execution effort: the exhaustive tier must not be abused as authority ──
    let execution_effort = field_val(fields, "Execution effort:");
    if execution_effort == "exhaustive" {
        let effort_abuse = EXHAUSTIVE_EFFORT_AUTHORITY_ABUSE_KEYWORDS
            .iter()
            .any(|kw| action_lower.contains(&kw.to_lowercase()));
        if effort_abuse {
            errors.push(format!(
                "[{}] Execution effort 为 {}（exhaustive 强度），但任务行动区域将其当作执行权限/跳过 review/自动执行的依据。Execution effort 只能表示思考强度，不能映射为 plan、parallel、permission escalation 或 workflow authority",
                error_code::EXECUTION_EFFORT_POLICY_VIOLATION,
                execution_effort
            ));
        }
    }

    let asks_fanout = has_workflow_request_intent(&workflow_lower);
    let permits_fanout = matches!(execution_mode, "fanout-in-card" | "fanout-cross-card");

    if asks_fanout && !permits_fanout {
        errors.push(format!(
            "[{}] Execution mode 为 {}，但任务行动区域要求多写者委派或跨 Agent 执行",
            error_code::EXECUTION_MODE_AUTHORITY_VIOLATION,
            execution_mode
        ));
    }

    if execution_topology == "single" && asks_fanout {
        errors.push(format!(
            "[{}] Execution topology 为 single，但任务行动区域要求并行或 worktree 执行",
            error_code::EXECUTION_TOPOLOGY_POLICY_VIOLATION
        ));
    }

    if execution_mode == "plan-only" && execution_topology != "single" {
        errors.push(format!(
            "[{}] plan-only 不能请求 {} 拓扑；只读计划必须使用 single",
            error_code::EXECUTION_TOPOLOGY_POLICY_VIOLATION,
            execution_topology
        ));
    }

    let asks_to_plan_delegation = [
        "delegation plan",
        "plan delegation",
        "制定委派",
        "设计委派",
        "规划子任务",
    ]
    .iter()
    .any(|keyword| workflow_lower.contains(keyword));
    if delegation_planning == "no" && asks_to_plan_delegation {
        errors.push(format!(
            "[{}] Delegation planning 为 no，但任务要求现场制定委派方案",
            error_code::DELEGATION_PLANNING_REQUIRED
        ));
    }

    // ── 子任务编排 (subtask orchestration) ↔ Delegation planning / ExecutionTopology ──
    // A non-`none` mode declares splittable subtask structure. It must be backed
    // by a fanout execution mode and a multi-worker topology. The slot only
    // DECLARES structure; actual subagent / workflow
    // ignition is translated by the claude-code adapter / runner from the
    // resolved policy, never fired by the task-card body itself.
    let subtask_mode = get_subtask_orchestration_mode(fields);
    if subtask_mode != "none" {
        if !permits_fanout {
            errors.push(format!(
                "[{}] 子任务编排 mode 为 {}，要求 Execution mode 为 fanout-in-card 或 fanout-cross-card",
                error_code::SUBTASK_ORCHESTRATION_VIOLATION,
                subtask_mode
            ));
        }
        if !matches!(execution_topology, "parallel" | "worktree") {
            errors.push(format!(
                "[{}] 子任务编排 mode 为 {}，要求 Execution topology 为 parallel 或 worktree，当前为 `{}`",
                error_code::SUBTASK_ORCHESTRATION_VIOLATION,
                subtask_mode,
                execution_topology
            ));
        }
    }
}
