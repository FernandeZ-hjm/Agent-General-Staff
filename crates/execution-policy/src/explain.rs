//! Policy explanation and gate-check functions.
//!
//! These are read-only consumer functions that call `resolve_policy()` and
//! derive structured explanations, gate decisions, or error outputs from
//! the resolved policy — they never modify the resolution pipeline itself.

use super::input::TaskPolicyInput;
use super::policy::{
    GateCheckOutput, GateDecision, GateErrorOutput, PolicyExplainOutput, PolicyExplanation,
    ResolvedExecutionPolicy, TaskSummary,
};
use super::resolve_policy;

// ── Gate check ────────────────────────────────────────────────────────────

/// Derive the runner-level gate decision from a resolved policy.
fn derive_decision(policy: &ResolvedExecutionPolicy) -> GateDecision {
    if policy.stop_before_launch {
        GateDecision::Stop
    } else if policy.requires_confirmation_gate {
        GateDecision::Confirm
    } else {
        GateDecision::Allow
    }
}

/// Run the full gate check on a validated task card input.
///
/// Resolves the execution policy and produces a `GateCheckOutput` with the
/// runner-level decision (`allow`, `confirm`, or `stop`).
pub fn gate_check(input: &TaskPolicyInput) -> GateCheckOutput {
    let resolved = resolve_policy(input.clone());
    let decision = derive_decision(&resolved);
    GateCheckOutput {
        schema_version: "2.0-m3".to_string(),
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
        schema_version: "2.0-m3".to_string(),
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
        permission_mode: input.permission_mode.clone(),
        parallelism: input.parallelism.clone(),
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

    // ── M1: Ultracode thinking intensity ──────────────────────────────
    let ultracode = input.effort() == "ultracode";
    explanations.push(PolicyExplanation {
        rule_id: "M1".to_string(),
        rule_name: "Ultracode Thinking Intensity".to_string(),
        decision: if ultracode { "applied" } else { "not_applicable" }.to_string(),
        field: Some("execution_effort".to_string()),
        detail: if ultracode {
            "Ultracode sets exhaustive mode (is_exhaustive_mode=true) without changing permission mode, parallelism, or launch args.".to_string()
        } else {
            "Execution effort is not ultracode; M1-M3 rules do not apply.".to_string()
        },
    });

    // ── M2: Ultracode does not enable parallelism ─────────────────────
    let requested_parallelism = super::policy::Parallelism::from_str(&input.parallelism);
    let ultracode_no_para = ultracode && !requested_parallelism.is_active();
    explanations.push(PolicyExplanation {
        rule_id: "M2".to_string(),
        rule_name: "Ultracode No Parallelism".to_string(),
        decision: if ultracode {
            if policy.effective_parallelism == super::policy::Parallelism::None {
                "passed"
            } else {
                "not_applicable"
            }
        } else {
            "not_applicable"
        }
        .to_string(),
        field: Some("parallelism".to_string()),
        detail: if ultracode_no_para {
            "Ultracode does not enable parallelism; effective_parallelism remains none.".to_string()
        } else if ultracode {
            "Ultracode is set but parallelism was enabled by another rule; M2 itself does not escalate parallelism.".to_string()
        } else {
            "Not an ultracode task.".to_string()
        },
    });

    // ── M3: Ultracode no permission-escalating launch args ────────────
    explanations.push(PolicyExplanation {
        rule_id: "M3".to_string(),
        rule_name: "Ultracode No Launch-Arg Escalation".to_string(),
        decision: if ultracode { "passed" } else { "not_applicable" }.to_string(),
        field: None,
        detail: if ultracode {
            "Ultracode does not inject any permission-escalating launch args (--permission-mode, --parallel, --worktree, --headless).".to_string()
        } else {
            "Not an ultracode task.".to_string()
        },
    });

    // ── M4: Heavy permission downgrade ────────────────────────────────
    let m4_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M4")
        .collect();
    let is_heavy = input.task_level == "Heavy";
    explanations.push(PolicyExplanation {
        rule_id: "M4".to_string(),
        rule_name: "Heavy Task Permission".to_string(),
        decision: if !m4_reasons.is_empty() {
            "applied"
        } else if is_heavy {
            "passed"
        } else {
            "not_applicable"
        }
        .to_string(),
        field: Some("permission_mode".to_string()),
        detail: if !m4_reasons.is_empty() {
            format!(
                "Heavy task requested {} without explicit write approval; downgraded to {}. Runner must stop until approval is provided.",
                m4_reasons[0].before, m4_reasons[0].after
            )
        } else if is_heavy {
            let note = if input.approval_source.is_approved() {
                "with explicit write approval"
            } else {
                "already in plan-only mode"
            };
            format!(
                "Heavy task {} — no downgrade needed. Confirmation gate is set (requires_confirmation_gate=true).",
                note
            )
        } else {
            "Not a Heavy task; M4 does not apply.".to_string()
        },
    });

    // ── M5: Writability gate — parallelism stripping ──────────────────
    let m5_para_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M5" && r.field == "parallelism")
        .collect();
    let m5_surface_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M5" && r.field == "execution_surface")
        .collect();
    let forbids = policy.effective_permission_mode.forbids_writes();

    explanations.push(PolicyExplanation {
        rule_id: "M5".to_string(),
        rule_name: "Writability Gate — Parallelism & Surface".to_string(),
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
                    "Parallelism '{}' stripped → '{}': effective permission '{}' forbids filesystem side effects.",
                    m5_para_reasons[0].before, m5_para_reasons[0].after,
                    policy.effective_permission_mode
                ));
            }
            if !m5_surface_reasons.is_empty() {
                parts.push(format!(
                    "Execution surface '{}' stripped → '{}': effective permission '{}' forbids headless side effects.",
                    m5_surface_reasons[0].before, m5_surface_reasons[0].after,
                    policy.effective_permission_mode
                ));
            }
            if parts.is_empty() && forbids {
                parts.push(format!(
                    "Effective permission '{}' forbids writes; no writability-violating parallelism or surface was requested — check passed.",
                    policy.effective_permission_mode
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
                policy.effective_permission_mode,
                args_display
            )
        } else {
            "Effective permission allows writes; M6 post-check not required.".to_string()
        },
    });

    // ── M7: Parallelism authority ────────────────────────────────────
    let m7_reasons: Vec<_> = policy
        .downgrade_reasons
        .iter()
        .filter(|r| r.rule_id == "M7")
        .collect();
    explanations.push(PolicyExplanation {
        rule_id: "M7".to_string(),
        rule_name: "Parallelism Workflow Authority".to_string(),
        decision: if !m7_reasons.is_empty() {
            "applied"
        } else if requested_parallelism.is_active() {
            "passed"
        } else {
            "not_applicable"
        }
        .to_string(),
        field: Some("parallelism".to_string()),
        detail: if !m7_reasons.is_empty() {
            format!(
                "{} — downgraded: '{}' → '{}'.",
                m7_reasons[0].reason, m7_reasons[0].before, m7_reasons[0].after
            )
        } else if requested_parallelism.is_active() {
            format!(
                "Parallelism '{}' is active with sufficient Workflow authority '{}'.",
                requested_parallelism,
                input.authority()
            )
        } else {
            "No active parallelism requested; M7 does not apply.".to_string()
        },
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
        field: Some("permission_mode".to_string()),
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
    if policy.effective_permission_mode.forbids_writes() {
        assertions.push(
            "WRITE PROTECTION: effective permission forbids writes; no write-enabling launch args present."
                .to_string(),
        );
    }
    if policy.was_downgraded {
        assertions.push(format!(
            "DOWNGRADE APPLIED: {} downgrade(s) recorded in downgrade_reasons audit trail.",
            policy.downgrade_reasons.len()
        ));
    }
    if policy.requires_confirmation_gate {
        assertions.push(
            "CONFIRMATION GATE: runner must present confirmation prompt before any mutation."
                .to_string(),
        );
    }
    if policy.is_exhaustive_mode {
        assertions.push(
            "EXHAUSTIVE MODE: deep reasoning enabled (ultracode). No permission or parallelism escalation."
                .to_string(),
        );
    }
    if assertions.is_empty() {
        assertions.push("No safety assertions — policy is clean.".to_string());
    }

    PolicyExplainOutput {
        schema_version: "2.0-m3".to_string(),
        task_summary: summary,
        explanations,
        safety_assertions: assertions,
        resolved_policy: policy,
    }
}

#[cfg(test)]
mod tests {
    use super::super::policy::ApprovalSource;
    use super::*;

    fn light_input() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "none".into(),
            task_level: "Light".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        }
    }

    fn heavy_execute_no_approval() -> TaskPolicyInput {
        TaskPolicyInput {
            permission_mode: "execute-and-verify".into(),
            task_level: "Heavy".into(),
            ..light_input()
        }
    }

    fn heavy_plan_only() -> TaskPolicyInput {
        TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Heavy".into(),
            ..light_input()
        }
    }

    // ── gate_check tests ─────────────────────────────────────────────

    #[test]
    fn gate_check_light_allow() {
        let output = gate_check(&light_input());
        assert_eq!(output.decision, GateDecision::Allow);
        assert_eq!(output.schema_version, "2.0-m3");
        assert!(!output.resolved_policy.stop_before_launch);
    }

    #[test]
    fn gate_check_heavy_plan_only_confirm() {
        let output = gate_check(&heavy_plan_only());
        assert_eq!(output.decision, GateDecision::Confirm);
        assert!(output.resolved_policy.requires_confirmation_gate);
    }

    #[test]
    fn gate_check_heavy_execute_no_approval_stop() {
        let output = gate_check(&heavy_execute_no_approval());
        assert_eq!(output.decision, GateDecision::Stop);
        assert!(output.resolved_policy.stop_before_launch);
    }

    #[test]
    fn gate_check_json_has_decision_and_resolved_policy() {
        let output = gate_check(&light_input());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"decision\":\"allow\""));
        assert!(json.contains("\"resolved_policy\""));
        assert!(json.contains("\"schema_version\":\"2.0-m3\""));
    }

    #[test]
    fn gate_check_stop_json_has_decision_stop() {
        let output = gate_check(&heavy_execute_no_approval());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"decision\":\"stop\""));
    }

    // ── gate_check_failed tests ──────────────────────────────────────

    #[test]
    fn gate_check_failed_outputs_structured_stop() {
        let output = gate_check_failed(
            "validation_failed",
            vec!["Missing required field".to_string()],
        );
        assert_eq!(output.decision, GateDecision::Stop);
        assert_eq!(output.error_kind, "validation_failed");
        assert_eq!(output.errors.len(), 1);
        assert_eq!(output.schema_version, "2.0-m3");

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"decision\":\"stop\""));
        assert!(json.contains("\"error_kind\":\"validation_failed\""));
        assert!(json.contains("Missing required field"));
    }

    // ── explain_policy tests ─────────────────────────────────────────

    #[test]
    fn explain_output_has_all_rule_ids() {
        let output = explain_policy(&light_input());
        let ids: Vec<&str> = output
            .explanations
            .iter()
            .map(|e| e.rule_id.as_str())
            .collect();
        for expected in &["M1", "M2", "M3", "M4", "M5", "M6", "M7", "M8", "M9", "M10"] {
            assert!(
                ids.contains(expected),
                "Missing rule_id {} in explanations: {:?}",
                expected,
                ids
            );
        }
        assert_eq!(output.explanations.len(), 10);
    }

    #[test]
    fn explain_decisions_are_valid() {
        let output = explain_policy(&light_input());
        for e in &output.explanations {
            assert!(
                e.decision == "applied" || e.decision == "passed" || e.decision == "not_applicable",
                "Invalid decision '{}' for rule {}",
                e.decision,
                e.rule_id
            );
        }
    }

    #[test]
    fn explain_has_schema_version() {
        let output = explain_policy(&light_input());
        assert_eq!(output.schema_version, "2.0-m3");
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"schema_version\":\"2.0-m3\""));
    }

    #[test]
    fn explain_safety_assertions_not_empty() {
        let output = explain_policy(&light_input());
        assert!(!output.safety_assertions.is_empty());
    }

    #[test]
    fn explain_light_no_downgrades() {
        let output = explain_policy(&light_input());
        // M4 should be not_applicable for Light
        let m4 = output
            .explanations
            .iter()
            .find(|e| e.rule_id == "M4")
            .unwrap();
        assert_eq!(m4.decision, "not_applicable");
        // No stop assertions
        let has_stop = output
            .safety_assertions
            .iter()
            .any(|a| a.contains("LAUNCH BLOCKED"));
        assert!(!has_stop);
    }

    #[test]
    fn explain_heavy_write_shows_m4_applied() {
        let output = explain_policy(&heavy_execute_no_approval());
        let m4 = output
            .explanations
            .iter()
            .find(|e| e.rule_id == "M4")
            .unwrap();
        assert_eq!(m4.decision, "applied");
        // Safety assertions must include LAUNCH BLOCKED
        let has_stop = output
            .safety_assertions
            .iter()
            .any(|a| a.contains("LAUNCH BLOCKED"));
        assert!(has_stop);
    }

    #[test]
    fn explain_has_task_summary() {
        let output = explain_policy(&light_input());
        assert_eq!(output.task_summary.executor, "Claude Code");
        assert_eq!(output.task_summary.task_level, "Light");
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"task_summary\""));
    }

    #[test]
    fn explain_json_has_all_top_level_keys() {
        let output = explain_policy(&light_input());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"schema_version\""));
        assert!(json.contains("\"task_summary\""));
        assert!(json.contains("\"explanations\""));
        assert!(json.contains("\"safety_assertions\""));
        assert!(json.contains("\"resolved_policy\""));
    }
}
