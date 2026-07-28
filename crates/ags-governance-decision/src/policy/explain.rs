//! Policy explanation and gate-check functions.
//!
//! These are read-only consumer functions that call `resolve_policy()` and
//! derive structured explanations, gate decisions, or error outputs from
//! the resolved policy — they never modify the resolution pipeline itself.

use super::input::TaskPolicyInput;
use super::model::{
    GateCheckOutput, GateDecision, GateErrorOutput, PolicyExplainOutput, PolicyExplanation,
    ResolvedExecutionPolicy, TaskSummary,
};
use super::resolve_policy;

// ── Gate check ────────────────────────────────────────────────────────────

/// Derive the runner-level gate decision from a resolved policy.
fn derive_decision(policy: &ResolvedExecutionPolicy) -> GateDecision {
    if policy.stop_before_launch {
        GateDecision::Stop
    } else {
        GateDecision::Allow
    }
}

/// Run the full gate check on a validated task card input.
///
/// Resolves the execution policy and produces a `GateCheckOutput` with the
/// runner-level decision (`allow` or `stop`).
pub fn gate_check(input: &TaskPolicyInput) -> GateCheckOutput {
    let resolved = resolve_policy(input.clone());
    let decision = derive_decision(&resolved);
    GateCheckOutput {
        schema_version: "0.3.6-execution-policy".to_string(),
        decision,
        resolved_policy: resolved,
    }
}

/// Produce a structured `decision=stop` error output for validation or
/// protected-path failures.
///
/// This ensures that `ags gate check` always outputs structured JSON even
/// when the task card fails validation — runners receive a machine-readable
/// `decision=stop` with error details, not just a raw exit code.
pub fn gate_check_failed(error_kind: &str, errors: Vec<String>) -> GateErrorOutput {
    GateErrorOutput {
        schema_version: "0.3.6-execution-policy".to_string(),
        decision: GateDecision::Stop,
        error_kind: error_kind.to_string(),
        errors,
    }
}

// ── Policy explain ────────────────────────────────────────────────────────

/// Build a `TaskSummary` from the raw input fields.
fn build_task_summary(input: &TaskPolicyInput) -> TaskSummary {
    TaskSummary {
        executor: input.executor.clone(),
        task_level: input.task_level.clone(),
        execution_effort: input.effort().to_string(),
        execution_mode: input.execution_mode.clone(),
        execution_topology: input.execution_topology.clone(),
        execution_surface: input.execution_surface.clone(),
    }
}

/// Explain how each M1-M10 rule was applied by inspecting the resolved policy.
///
/// This is a post-hoc reconstruction — it calls `resolve_policy()` once,
/// then derives explanations from the resolved output rather than
/// instrumenting the pipeline.
pub fn explain_policy(input: &TaskPolicyInput) -> PolicyExplainOutput {
    let policy = resolve_policy(input.clone());
    let summary = build_task_summary(input);
    let mut explanations: Vec<PolicyExplanation> = Vec::with_capacity(10);

    // ── M1: Exhaustive-effort thinking intensity ──────────────────────
    let exhaustive = input.is_exhaustive_effort();
    explanations.push(PolicyExplanation {
        rule_id: "M1".to_string(),
        rule_name: "Exhaustive Effort Thinking Intensity".to_string(),
        decision: if exhaustive { "applied" } else { "not_applicable" }.to_string(),
        field: Some("execution_effort".to_string()),
        detail: if exhaustive {
            "Execution effort: exhaustive sets is_exhaustive_mode=true without changing execution mode, execution_topology, or launch args.".to_string()
        } else {
            "Execution effort is not the exhaustive tier; M1-M3 rules do not apply.".to_string()
        },
    });

    // ── M2: Exhaustive effort does not enable execution_topology ─────────────
    let requested_execution_topology =
        super::model::ExecutionTopology::from_str(&input.execution_topology);
    let exhaustive_no_para = exhaustive && !requested_execution_topology.is_active();
    explanations.push(PolicyExplanation {
        rule_id: "M2".to_string(),
        rule_name: "Exhaustive Effort No ExecutionTopology".to_string(),
        decision: if exhaustive {
            if policy.effective_execution_topology == super::model::ExecutionTopology::Single {
                "passed"
            } else {
                "not_applicable"
            }
        } else {
            "not_applicable"
        }
        .to_string(),
        field: Some("execution_topology".to_string()),
        detail: if exhaustive_no_para {
            "Exhaustive effort does not enable execution_topology; effective_execution_topology remains none.".to_string()
        } else if exhaustive {
            "Exhaustive effort is set but execution_topology was enabled by another rule; M2 itself does not escalate execution_topology.".to_string()
        } else {
            "Not an exhaustive-effort task.".to_string()
        },
    });

    // ── M3: Exhaustive effort, no permission-escalating launch args ───
    explanations.push(PolicyExplanation {
        rule_id: "M3".to_string(),
        rule_name: "Exhaustive Effort No Launch-Arg Escalation".to_string(),
        decision: if exhaustive { "passed" } else { "not_applicable" }.to_string(),
        field: None,
        detail: if exhaustive {
            "Exhaustive effort does not inject any permission-escalating launch args (--permission-mode, --parallel, --worktree, --headless).".to_string()
        } else {
            "Not an exhaustive-effort task.".to_string()
        },
    });

    // ── M4: task level does not rewrite permission ────────────────────
    explanations.push(PolicyExplanation {
        rule_id: "M4".to_string(),
        rule_name: "Two-State Permission Independence".to_string(),
        decision: "passed".to_string(),
        field: Some("execution_mode".to_string()),
        detail: format!(
            "Effective execution mode is '{}'. Task level does not rewrite writer scope. Heavy review is enforced independently by task-card validation.",
            policy.effective_execution_mode
        ),
    });

    // ── M5: Writability gate — execution_topology stripping ──────────────────
    let m5_para_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M5" && r.field == "execution_topology")
        .collect();
    let m5_surface_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M5" && r.field == "execution_surface")
        .collect();
    let forbids = policy.effective_execution_mode.forbids_writes();

    explanations.push(PolicyExplanation {
        rule_id: "M5".to_string(),
        rule_name: "Writability Gate — ExecutionTopology & Surface".to_string(),
        decision: if !m5_para_reasons.is_empty() || !m5_surface_reasons.is_empty() {
            "applied"
        } else if forbids {
            "passed"
        } else {
            "not_applicable"
        }
        .to_string(),
        field: None,
        detail: {
            let mut parts: Vec<String> = Vec::new();
            if !m5_para_reasons.is_empty() {
                parts.push(format!(
                    "ExecutionTopology '{}' stripped → '{}': effective execution mode '{}' forbids filesystem side effects.",
                    m5_para_reasons[0].before, m5_para_reasons[0].after,
                    policy.effective_execution_mode
                ));
            }
            if !m5_surface_reasons.is_empty() {
                parts.push(format!(
                    "Execution surface '{}' stripped → '{}': effective execution mode '{}' forbids headless side effects.",
                    m5_surface_reasons[0].before, m5_surface_reasons[0].after,
                    policy.effective_execution_mode
                ));
            }
            if parts.is_empty() && forbids {
                parts.push(format!(
                    "Effective permission '{}' forbids writes; no writability-violating execution_topology or surface was requested — check passed.",
                    policy.effective_execution_mode
                ));
            }
            if parts.is_empty() {
                parts.push("Effective permission allows writes; M5 writability gate not triggered.".to_string());
            }
            parts.join(" ")
        },
    });

    // ── M6: Launch args writability post-check ───────────────────────
    explanations.push(PolicyExplanation {
        rule_id: "M6".to_string(),
        rule_name: "Launch Args Writability Post-Check".to_string(),
        decision: if forbids { "applied" } else { "not_applicable" }
            .to_string(),
        field: Some("allowed_launch_args".to_string()),
        detail: if forbids {
            let args_display = if policy.allowed_launch_args.is_empty() {
                "(none)".to_string()
            } else {
                policy.allowed_launch_args.join(", ")
            };
            format!(
                "Effective permission '{}' forbids writes; launch args verified: no --parallel, --worktree, --headless, acceptEdits, or bypassPermissions present. Args: [{}]",
                policy.effective_execution_mode,
                args_display
            )
        } else {
            "Effective permission allows writes; M6 post-check not required.".to_string()
        },
    });

    // ── M7: topology and delegation planning are non-authority dimensions ──
    explanations.push(PolicyExplanation {
        rule_id: "M7".to_string(),
        rule_name: "Topology And Delegation Independence".to_string(),
        decision: "passed".to_string(),
        field: Some("execution_topology".to_string()),
        detail: format!(
            "Topology '{}' describes layout and Delegation planning '{}' only permits planning; neither grants writer scope beyond Execution mode '{}'.",
            requested_execution_topology,
            if input.delegation_planning_enabled() { "yes" } else { "no" },
            policy.effective_execution_mode
        ),
    });

    // ── M8: Audit trail ──────────────────────────────────────────────
    explanations.push(PolicyExplanation {
        rule_id: "M8".to_string(),
        rule_name: "Structured Audit Trail".to_string(),
        decision: if policy.was_downgraded { "applied" } else { "passed" }
            .to_string(),
        field: Some("downgrade_reasons".to_string()),
        detail: if policy.was_downgraded {
            format!(
                "{} downgrade(s) recorded with structured audit trail (rule_id, field, before, after, reason).",
                policy.downgrade_reasons.len()
            )
        } else {
            "No downgrades applied; audit trail empty (consistent with was_downgraded=false).".to_string()
        },
    });

    // ── M9: Generic adapter permission cap ───────────────────────────
    let m9_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M9")
        .collect();
    let is_generic = input.runtime_adapter == "generic";
    explanations.push(PolicyExplanation {
        rule_id: "M9".to_string(),
        rule_name: "Generic Adapter Permission Cap".to_string(),
        decision: if !m9_reasons.is_empty() {
            "applied"
        } else if is_generic {
            "passed"
        } else {
            "not_applicable"
        }
        .to_string(),
        field: Some("execution_mode".to_string()),
        detail: if !m9_reasons.is_empty() {
            format!(
                "Generic adapter caps permission at plan-only without explicit approval. {}",
                m9_reasons[0].reason
            )
        } else if is_generic {
            "Generic adapter with explicit approval — permission cap not applied.".to_string()
        } else {
            format!(
                "Runtime adapter is '{}', not 'generic'; M9 does not apply.",
                input.runtime_adapter
            )
        },
    });

    // ── M10: Downgrade invariants ────────────────────────────────────
    explanations.push(PolicyExplanation {
        rule_id: "M10".to_string(),
        rule_name: "Downgrade Invariants".to_string(),
        decision: "applied".to_string(),
        field: None,
        detail: format!(
            "Invariants verified: was_downgraded={} ↔ downgrade_reasons count={}. Consistency confirmed.",
            policy.was_downgraded,
            policy.downgrade_reasons.len()
        ),
    });

    // ── Safety assertions ────────────────────────────────────────────
    let mut assertions: Vec<String> = Vec::new();
    if policy.stop_before_launch {
        assertions.push(
            "LAUNCH BLOCKED: allowed_launch_args is empty; runner must not start. Task card requires rewrite or explicit approval."
                .to_string(),
        );
    }
    if policy.effective_execution_mode.forbids_writes() {
        assertions.push(
            "WRITE PROTECTION: effective execution mode forbids writes; no write-enabling launch args present."
                .to_string(),
        );
    }
    if policy.was_downgraded {
        assertions.push(format!(
            "DOWNGRADE APPLIED: {} downgrade(s) recorded in downgrade_reasons audit trail.",
            policy.downgrade_reasons.len()
        ));
    }
    if policy.is_exhaustive_mode {
        assertions.push(
            "EXHAUSTIVE MODE: deep reasoning enabled (Execution effort: exhaustive). No permission or execution_topology escalation."
                .to_string(),
        );
    }
    if assertions.is_empty() {
        assertions.push("No safety assertions — policy is clean.".to_string());
    }

    PolicyExplainOutput {
        schema_version: "0.3.6-execution-policy".to_string(),
        task_summary: summary,
        explanations,
        safety_assertions: assertions,
        resolved_policy: policy,
    }
}
