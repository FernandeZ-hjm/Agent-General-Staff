//! Execution-policy resolver for Agent Governance Suite.
//!
//! Policy M1-M10 rule IDs used below are distinct from Roadmap M0-M8 milestone
//! IDs — they live in separate namespaces.
//!
//! Takes a validated task card's structured fields (`TaskPolicyInput`) and
//! resolves the execution policy (`ResolvedExecutionPolicy`) — answering:
//! how should this task actually execute, what flags to use, what gets
//! downgraded, and whether to stop before launch.
//!
//! # Architecture
//!
//! ```text
//! TaskPolicyInput  ──►  resolve_policy()  ──►  ResolvedExecutionPolicy
//!   (strings)              │                       (typed enums)
//!                          │
//!                          ├─ build_initial_policy()
//!                          ├─ apply_ultracode_rules()        (M1-M3)
//!                          ├─ apply_heavy_permission_rule()  (M4)
//!                          ├─ apply_generic_adapter_rule()   (M9)
//!                          ├─ apply_parallelism_authority_rule() (M7)
//!                          ├─ generate_launch_args()          (M5/M6 enforced)
//!                          ├─ apply_stop_on_stripped_parallelism()
//!                          ├─ apply_launch_args_writability_gate() (M5-M6 post-check)
//!                          └─ verify_downgrade_invariants()  (M10)
//! ```
//!
//! # Example
//!
//! ```rust
//! use execution_policy::{resolve_policy, TaskPolicyInput, ApprovalSource};
//!
//! let input = TaskPolicyInput {
//!     executor: "Claude Code".into(),
//!     runtime_adapter: "claude-code".into(),
//!     execution_surface: "cli".into(),
//!     permission_mode: "plan-only".into(),
//!     parallelism: "none".into(),
//!     task_level: "Heavy".into(),
//!     execution_effort: Some("normal".into()),
//!     workflow_authority: Some("none".into()),
//!     approval_source: ApprovalSource::None,
//! };
//!
//! let policy = resolve_policy(input);
//! assert_eq!(policy.effective_permission_mode.to_string(), "plan-only");
//! assert!(policy.requires_confirmation_gate);
//! assert!(!policy.was_downgraded); // already plan-only
//! ```

mod explain;
mod input;
mod policy;
mod rules;

pub use explain::{explain_policy, gate_check, gate_check_failed};
pub use input::TaskPolicyInput;
pub use policy::{
    ApprovalSource, DowngradeReason, GateCheckOutput, GateDecision, GateErrorOutput, Parallelism,
    PermissionMode, PolicyExplainOutput, PolicyExplanation, ResolvedExecutionPolicy, StopReason,
    TaskSummary,
};

use rules::{
    apply_generic_adapter_rule, apply_heavy_permission_rule, apply_launch_args_writability_gate,
    apply_parallelism_authority_rule, apply_stop_before_launch_arg_gate,
    apply_stop_on_stripped_headless, apply_stop_on_stripped_parallelism, apply_ultracode_rules,
    build_initial_policy, generate_launch_args, verify_downgrade_invariants,
};

/// Resolve execution policy from a validated task card's structured fields.
///
/// Applies all MUST rules in order:
/// 1. Build initial policy from input
/// 2. M1-M3: Ultracode thinking-intensity rules
/// 3. M4: Heavy task permission downgrade
/// 4. M9: Generic adapter permission cap
/// 5. M7: Parallelism authority check
/// 6. Generate runtime-specific launch args (M5/M6 enforced inline)
/// 7. M5 enforcement: stop if writability-violating parallelism was stripped
/// 8. M5 enforcement: stop if background-agent surface was stripped
/// 9. Stop finalization: stopped policies expose no launch args
/// 10. M5-M6 post-check: structural invariant on launch args
/// 11. M10: Verify downgrade invariants
pub fn resolve_policy(input: TaskPolicyInput) -> ResolvedExecutionPolicy {
    let mut policy = build_initial_policy(&input);

    // M1-M3: ultracode → thinking intensity only
    apply_ultracode_rules(&input, &mut policy);

    // M4: Heavy + non-plan-only → plan-only (without explicit approval)
    apply_heavy_permission_rule(&input, &mut policy);

    // M9: generic adapter permission cap
    apply_generic_adapter_rule(&input, &mut policy);

    // M7: parallelism requires workflow authority
    apply_parallelism_authority_rule(&input, &mut policy);

    // Generate runtime-specific launch args (M5/M6 enforced here)
    generate_launch_args(&input, &mut policy);

    // M5 enforcement: stop if parallelism was stripped due to writability gate
    apply_stop_on_stripped_parallelism(&input, &mut policy);

    // M5 enforcement: stop if background-agent surface was stripped
    apply_stop_on_stripped_headless(&input, &mut policy);

    // Stop finalization: no launch args are consumable once launch is blocked.
    apply_stop_before_launch_arg_gate(&mut policy);

    // M5-M6: read-only/plan-only structural invariant (verified in tests)
    apply_launch_args_writability_gate(&policy);

    // M10: downgrade invariants
    verify_downgrade_invariants(&policy);

    policy
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a standard Light execute-and-verify input.
    fn light_execute_input() -> TaskPolicyInput {
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

    /// Helper: build a standard Heavy execute-and-verify input (no approval).
    fn heavy_execute_no_approval() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "none".into(),
            task_level: "Heavy".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        }
    }

    /// Helper: build a Heavy edit-with-confirmation input with approval.
    fn heavy_edit_with_approval() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "edit-with-confirmation".into(),
            parallelism: "none".into(),
            task_level: "Heavy".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::CliFlag,
        }
    }

    /// Helper: build an ultracode input at Medium level.
    fn ultracode_medium_input() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "none".into(),
            task_level: "Medium".into(),
            execution_effort: Some("ultracode".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        }
    }

    /// Helper: build a generic adapter execute-and-verify input.
    fn generic_execute_input() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Other".into(),
            runtime_adapter: "generic".into(),
            execution_surface: "local-workspace".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "none".into(),
            task_level: "Medium".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        }
    }

    /// Helper: build a subagent input without required workflow authority.
    fn subagent_no_authority_input() -> TaskPolicyInput {
        TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "subagent".into(),
            task_level: "Medium".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        }
    }

    // ── T1: Light execute-and-verify happy path ──────────────────────

    #[test]
    fn light_execute_and_verify_happy_path() {
        let input = light_execute_input();
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert!(!policy.was_downgraded);
        assert!(policy.downgrade_reasons.is_empty());
        assert!(!policy.requires_confirmation_gate);
        assert!(!policy.stop_before_launch);
        assert!(policy.stop_reasons.is_empty());
        assert_eq!(policy.executor, "Claude Code");
        assert_eq!(policy.runtime_adapter, "claude-code");
        assert!(!policy.is_exhaustive_mode);
        assert_eq!(policy.execution_effort, "normal");
        assert_eq!(policy.approval_source, ApprovalSource::None);
    }

    // ── T2: Heavy execute-and-verify without approval → NO downgrade ──
    //
    // Task LEVEL is decoupled from execution authority: a Heavy card keeps its
    // declared permission mode and only gains a confirmation gate. No downgrade,
    // no cap, no stop — approval is not required to execute a Heavy card.

    #[test]
    fn heavy_execute_without_approval_is_executable_with_confirmation() {
        let input = heavy_execute_no_approval();
        let policy = resolve_policy(input);

        // Declared permission preserved — NOT downgraded to plan-only.
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);
        assert!(policy.downgrade_reasons.is_empty());

        // Heavy still requires a confirmation gate.
        assert!(policy.requires_confirmation_gate);

        // Executable: no stop, launch args are not cleared by a stop gate.
        assert!(!policy.stop_before_launch);
        assert!(policy.stop_reasons.is_empty());
    }

    // ── T3: Heavy edit-with-confirmation WITH approval → no downgrade ─

    #[test]
    fn heavy_edit_with_approval_no_downgrade() {
        let input = heavy_edit_with_approval();
        let policy = resolve_policy(input);

        // No downgrade — task level never rewrites the declared permission mode
        // (the approval signal is audit/hint only, not what preserves the mode).
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::EditWithConfirmation
        );
        assert!(!policy.was_downgraded);
        assert!(policy.downgrade_reasons.is_empty());
        // Still requires confirmation gate for Heavy
        assert!(policy.requires_confirmation_gate);
        assert_eq!(policy.approval_source, ApprovalSource::CliFlag);
    }

    // ── T4: Ultracode does NOT escalate permission ───────────────────

    #[test]
    fn ultracode_does_not_escalate_permission() {
        let input = ultracode_medium_input();
        let policy = resolve_policy(input);

        // Ultracode sets exhaustive mode
        assert!(policy.is_exhaustive_mode);
        assert_eq!(policy.execution_effort, "ultracode");

        // But does NOT change permission
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);

        // Does NOT enable parallelism
        assert_eq!(policy.effective_parallelism, Parallelism::None);

        // Does NOT add --permission-mode plan (that would be a downgrade/change)
        assert!(!policy
            .allowed_launch_args
            .contains(&"--permission-mode".to_string()));
    }

    // ── T5: Ultracode does NOT enable parallelism ────────────────────

    #[test]
    fn ultracode_does_not_enable_parallelism() {
        let mut input = ultracode_medium_input();
        input.parallelism = "none".into();

        let policy = resolve_policy(input);

        assert!(policy.is_exhaustive_mode);
        assert_eq!(policy.effective_parallelism, Parallelism::None);
    }

    // ── T6: Ultracode does NOT generate --permission-mode plan ───────

    #[test]
    fn ultracode_does_not_generate_permission_mode_plan() {
        let input = ultracode_medium_input();
        let policy = resolve_policy(input);

        assert!(
            !policy
                .allowed_launch_args
                .contains(&"--permission-mode".to_string()),
            "ultracode must not inject --permission-mode"
        );
    }

    // ── T7: Subagent without Workflow authority → downgrade to none ──

    #[test]
    fn subagent_without_workflow_authority_downgrades_to_none() {
        let input = subagent_no_authority_input();
        let policy = resolve_policy(input);

        // Parallelism downgraded
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert!(policy.was_downgraded);
        assert_eq!(policy.downgrade_reasons.len(), 1);
        assert_eq!(policy.downgrade_reasons[0].rule_id, "M7");
        assert_eq!(policy.downgrade_reasons[0].field, "parallelism");
        assert_eq!(policy.downgrade_reasons[0].before, "subagent");
        assert_eq!(policy.downgrade_reasons[0].after, "none");
    }

    // ── T8: Generic adapter execute-and-verify → plan-only ──────────

    #[test]
    fn generic_adapter_downgrades_execute_to_plan_only() {
        let input = generic_execute_input();
        let policy = resolve_policy(input);

        // Downgraded: execute-and-verify → plan-only
        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert!(policy.was_downgraded);
        assert_eq!(policy.downgrade_reasons.len(), 1);
        assert_eq!(policy.downgrade_reasons[0].rule_id, "M9");
    }

    // ── T9: Subagent with within-card authority → allowed ────────────

    #[test]
    fn subagent_with_within_card_authority_allowed() {
        let input = TaskPolicyInput {
            parallelism: "subagent".into(),
            workflow_authority: Some("within-card".into()),
            ..subagent_no_authority_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_parallelism, Parallelism::Subagent);
        assert!(!policy.was_downgraded);
        assert!(policy
            .allowed_launch_args
            .contains(&"--parallel".to_string()));
    }

    // ── T10: Worktree with plan-only authority (execute-and-verify) → allowed ─

    #[test]
    fn worktree_with_plan_only_authority_allowed() {
        let input = TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "edit-with-confirmation".into(),
            parallelism: "worktree".into(),
            task_level: "Medium".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("plan-only".into()),
            approval_source: ApprovalSource::None,
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_parallelism, Parallelism::Worktree);
        assert!(!policy.was_downgraded);
        assert!(policy
            .allowed_launch_args
            .contains(&"--parallel".to_string()));
        assert!(policy
            .allowed_launch_args
            .contains(&"--worktree".to_string()));
    }

    // ── T11: M10 downgrade invariant — every downgrade has a reason ──

    #[test]
    fn every_downgrade_has_recorded_reason() {
        // Genuine downgrade vehicle: the M9 generic-adapter cap (task level no
        // longer downgrades).
        let input = generic_execute_input();
        let policy = resolve_policy(input);

        assert!(policy.was_downgraded);
        assert!(!policy.downgrade_reasons.is_empty());
        for reason in &policy.downgrade_reasons {
            assert!(!reason.rule_id.is_empty());
            assert!(!reason.field.is_empty());
            assert!(!reason.before.is_empty());
            assert!(!reason.after.is_empty());
            assert!(!reason.reason.is_empty());
        }
    }

    // ── T12: M10 downgrade invariant — no downgrade = no reasons ─────

    #[test]
    fn no_downgrade_means_no_reasons() {
        let input = light_execute_input();
        let policy = resolve_policy(input);

        assert!(!policy.was_downgraded);
        assert!(policy.downgrade_reasons.is_empty());
    }

    // ── T13: Plan-only already → no Heavy downgrade needed ──────────

    #[test]
    fn heavy_plan_only_no_downgrade_needed() {
        let mut input = heavy_execute_no_approval();
        input.permission_mode = "plan-only".into();

        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert!(!policy.was_downgraded);
        assert!(policy.requires_confirmation_gate);
    }

    // ── T14: Default semantics — absent effort/authority ─────────────

    #[test]
    fn absent_effort_and_authority_default_correctly() {
        let input = TaskPolicyInput {
            executor: "Claude Code".into(),
            runtime_adapter: "claude-code".into(),
            execution_surface: "cli".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "none".into(),
            task_level: "Light".into(),
            execution_effort: None,
            workflow_authority: None,
            approval_source: ApprovalSource::None,
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.execution_effort, "unknown");
        assert!(!policy.is_exhaustive_mode);
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
    }

    // ── T15: Legacy parallelism values (limited, parallel) → None ────

    #[test]
    fn legacy_parallelism_values_map_to_none() {
        for legacy in &["limited", "parallel"] {
            let input = TaskPolicyInput {
                parallelism: legacy.to_string(),
                ..light_execute_input()
            };
            let policy = resolve_policy(input);
            assert_eq!(
                policy.effective_parallelism,
                Parallelism::None,
                "legacy value '{}' should map to None",
                legacy
            );
        }
    }

    // ── T16: Generic adapter with explicit approval → no cap ─────────

    #[test]
    fn generic_adapter_with_explicit_approval_no_cap() {
        let mut input = generic_execute_input();
        input.approval_source = ApprovalSource::CliFlag;

        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);
    }

    // ── T17: Read-only → no launch args at all ──────────────────────

    #[test]
    fn read_only_generates_no_launch_args() {
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::ReadOnly);
        assert!(!policy
            .allowed_launch_args
            .contains(&"--permission-mode".to_string()));
    }

    // ── T18: Plan-only → --permission-mode plan (not write-enabling) ─

    #[test]
    fn plan_only_generates_plan_flag_not_write_flags() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert!(policy
            .allowed_launch_args
            .contains(&"--permission-mode".to_string()));
        assert!(policy.allowed_launch_args.contains(&"plan".to_string()));
        let args_str = policy.allowed_launch_args.join(" ");
        assert!(!args_str.contains("acceptEdits"));
        assert!(!args_str.contains("bypassPermissions"));
    }

    // ── T19: Background agent surface → --headless (only in write modes) ─

    #[test]
    fn background_agent_adds_headless_flag() {
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        // execute-and-verify allows writes, so --headless IS allowed
        assert!(policy
            .allowed_launch_args
            .contains(&"--headless".to_string()));
    }

    // ── T20: Multiple downgrades tracked independently ──────────────

    #[test]
    fn multiple_downgrades_tracked_independently() {
        let input = TaskPolicyInput {
            executor: "Other".into(),
            runtime_adapter: "generic".into(),
            execution_surface: "local-workspace".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "subagent".into(),
            task_level: "Heavy".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        };
        let policy = resolve_policy(input);

        assert!(policy.was_downgraded);
        assert!(policy.downgrade_reasons.len() >= 2);
        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        // Verify all reasons have structured data
        for reason in &policy.downgrade_reasons {
            assert!(!reason.rule_id.is_empty());
            assert!(!reason.field.is_empty());
            assert!(!reason.before.is_empty());
            assert!(!reason.after.is_empty());
        }
    }

    // ── T21: Heavy + plan-only in card body but execute-and-verify in field ─

    #[test]
    fn heavy_already_plan_only_preserves_plan_only() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Heavy".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert!(policy.requires_confirmation_gate);
        assert!(!policy.was_downgraded);
    }

    // ═════════════════════════════════════════════════════════════════════
    // F1: read-only/plan-only + active parallelism → no --parallel/--worktree
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn read_only_plus_worktree_does_not_output_parallel_worktree() {
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            // M7 requires worktree authority NOT "none", so give plan-only
            workflow_authority: Some("plan-only".into()),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::ReadOnly);
        // M5 enforcement: effective_parallelism downgraded to None
        // because read-only forbids filesystem-side-effect parallelism
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        // M5/M6: must NOT contain --parallel or --worktree
        assert!(
            !policy
                .allowed_launch_args
                .contains(&"--parallel".to_string()),
            "read-only must not produce --parallel"
        );
        assert!(
            !policy
                .allowed_launch_args
                .contains(&"--worktree".to_string()),
            "read-only must not produce --worktree"
        );
        // Must have stop reason
        assert!(policy.stop_before_launch);
        assert!(!policy.stop_reasons.is_empty());
        // Must have downgrade record for stripped parallelism
        let stripped_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.rule_id == "M5")
            .collect();
        assert_eq!(
            stripped_reasons.len(),
            1,
            "must record M5 downgrade for stripped parallelism"
        );
        assert_eq!(stripped_reasons[0].field, "parallelism");
    }

    #[test]
    fn plan_only_plus_worktree_stops_with_no_launch_args() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()), // M7: worktree needs NOT none
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert!(policy.stop_before_launch);
        assert!(!policy.stop_reasons.is_empty());
        assert!(
            policy.allowed_launch_args.is_empty(),
            "stopped plan-only + worktree must not expose launch args"
        );
    }

    #[test]
    fn read_only_plus_subagent_no_launch_args() {
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "subagent".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy.allowed_launch_args.is_empty());
        assert!(policy.stop_before_launch);
    }

    #[test]
    fn plan_only_plus_agent_team_no_parallel_flag() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            parallelism: "agent-team".into(),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(
            !policy
                .allowed_launch_args
                .contains(&"--parallel".to_string()),
            "plan-only + agent-team must not produce --parallel"
        );
        assert!(policy.stop_before_launch);
    }

    #[test]
    fn execute_and_verify_plus_worktree_allows_parallel_worktree() {
        let input = TaskPolicyInput {
            permission_mode: "execute-and-verify".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        // execute-and-verify DOES allow writes, so parallelism flags are preserved
        assert!(policy
            .allowed_launch_args
            .contains(&"--parallel".to_string()));
        assert!(policy
            .allowed_launch_args
            .contains(&"--worktree".to_string()));
        assert!(!policy.stop_before_launch);
    }

    // ═════════════════════════════════════════════════════════════════════
    // F3: stop_before_launch / stop_reasons semantics
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn stop_before_launch_false_for_normal_light_execute() {
        let input = light_execute_input();
        let policy = resolve_policy(input);

        assert!(!policy.stop_before_launch);
        assert!(policy.stop_reasons.is_empty());
    }

    #[test]
    fn stop_reason_is_clearly_documented() {
        // Read-only + worktree → stop with writable-parallelism-blocked
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()), // M7: worktree needs NOT none
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy.stop_before_launch);
        let reason = policy.stop_reasons.first().unwrap();
        // Stop reason should surface the key facts
        let reason_str = reason.to_string();
        assert!(reason_str.contains("read-only"));
        assert!(reason_str.contains("worktree"));
        // The Display should be human-readable and non-empty
        assert!(!reason_str.is_empty());
    }

    // ═════════════════════════════════════════════════════════════════════
    // F4: approval source — cannot come from task card text
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn from_fields_always_sets_approval_source_to_none() {
        use std::collections::HashMap;
        let mut fields = HashMap::new();
        fields.insert("Executor:".to_string(), "Claude Code".to_string());
        fields.insert(
            "Permission mode:".to_string(),
            "execute-and-verify".to_string(),
        );
        fields.insert("任务级别：".to_string(), "Heavy".to_string());

        let input = TaskPolicyInput::from_fields(&fields);
        assert_eq!(input.approval_source, ApprovalSource::None);
    }

    #[test]
    fn runner_env_approval_allows_heavy_mutation() {
        let input = TaskPolicyInput {
            approval_source: ApprovalSource::RunnerEnv,
            ..heavy_execute_no_approval()
        };
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);
        assert_eq!(policy.approval_source, ApprovalSource::RunnerEnv);
    }

    // ── Decoupling: structured current-task instruction approval ─────────────

    #[test]
    fn heavy_edit_with_current_task_instruction_no_downgrade() {
        // Heavy + edit-with-confirmation is executable directly (no plan-only
        // round trip, no stop). The current-task-instruction signal is audit/hint
        // only — the declared permission is what makes the card executable.
        let input = TaskPolicyInput {
            permission_mode: "edit-with-confirmation".into(),
            task_level: "Heavy".into(),
            approval_source: ApprovalSource::CurrentTaskInstruction,
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::EditWithConfirmation
        );
        assert!(!policy.was_downgraded);
        assert!(!policy.stop_before_launch);
        assert!(policy.requires_confirmation_gate);
        assert_eq!(
            policy.approval_source,
            ApprovalSource::CurrentTaskInstruction
        );
    }

    #[test]
    fn heavy_execute_with_current_task_instruction_not_capped() {
        // Task level no longer caps execute-and-verify: a Heavy card keeps its
        // declared execute-and-verify regardless of the approval signal. No
        // downgrade, no cap, no stop — only the confirmation gate is added.
        let input = TaskPolicyInput {
            permission_mode: "execute-and-verify".into(),
            task_level: "Heavy".into(),
            approval_source: ApprovalSource::CurrentTaskInstruction,
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);
        assert!(policy.downgrade_reasons.is_empty());
        assert!(!policy.stop_before_launch);
        assert!(policy.requires_confirmation_gate);
    }

    #[test]
    fn heavy_execute_holds_execute_and_verify_regardless_of_approval() {
        // execute-and-verify holds for a Heavy card with OR without an approval
        // signal — task level never caps or downgrades the declared permission.
        let input = TaskPolicyInput {
            permission_mode: "execute-and-verify".into(),
            task_level: "Heavy".into(),
            approval_source: ApprovalSource::CliFlag,
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert!(!policy.was_downgraded);
        assert!(!policy.stop_before_launch);
        assert!(policy.requires_confirmation_gate);
    }

    #[test]
    fn current_task_instruction_serializes_canonical() {
        let input = TaskPolicyInput {
            permission_mode: "edit-with-confirmation".into(),
            task_level: "Heavy".into(),
            approval_source: ApprovalSource::CurrentTaskInstruction,
            ..light_execute_input()
        };
        let json = serde_json::to_string(&resolve_policy(input)).unwrap();
        assert!(
            json.contains("\"current-task-instruction\""),
            "approval_source must serialize canonically: {json}"
        );
    }

    // ═════════════════════════════════════════════════════════════════════
    // F5: JSON canonical values
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn json_uses_canonical_permission_mode_values() {
        let input = light_execute_input();
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        // Should use "execute-and-verify", not "ExecuteAndVerify"
        assert!(
            json.contains("\"execute-and-verify\""),
            "JSON must use canonical strings: {}",
            json
        );
        assert!(
            !json.contains("ExecuteAndVerify"),
            "JSON must NOT contain Rust variant names: {}",
            json
        );
    }

    #[test]
    fn json_uses_canonical_parallelism_values() {
        let input = TaskPolicyInput {
            parallelism: "subagent".into(),
            workflow_authority: Some("within-card".into()),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        assert!(
            json.contains("\"subagent\""),
            "JSON must use canonical parallelism strings: {}",
            json
        );
        assert!(
            !json.contains("\"Subagent\""),
            "JSON must NOT contain Rust variant names: {}",
            json
        );
    }

    #[test]
    fn json_downgrade_reason_has_structured_fields() {
        // Genuine downgrade vehicle: the M9 generic-adapter cap.
        let input = generic_execute_input();
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        assert!(json.contains("\"rule_id\""));
        assert!(json.contains("\"M9\""));
        assert!(json.contains("\"field\""));
        assert!(json.contains("\"permission_mode\""));
        assert!(json.contains("\"before\""));
        assert!(json.contains("\"after\""));
    }

    #[test]
    fn json_approval_source_is_canonical() {
        let input = heavy_edit_with_approval(); // ApprovalSource::CliFlag
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        assert!(
            json.contains("\"cli-flag\""),
            "approval_source must be canonical: {}",
            json
        );
    }

    // ═════════════════════════════════════════════════════════════════════
    // F6: background-agent surface in complete valid chain
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn background_agent_execute_and_verify_allows_headless() {
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        // execute-and-verify allows writes → headless allowed
        assert!(policy
            .allowed_launch_args
            .contains(&"--headless".to_string()));
        assert!(!policy.stop_before_launch);
    }

    #[test]
    fn background_agent_read_only_no_headless() {
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "read-only".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        // read-only forbids writes → headless stripped
        assert!(
            !policy
                .allowed_launch_args
                .contains(&"--headless".to_string()),
            "read-only must not produce --headless"
        );
    }

    // ═════════════════════════════════════════════════════════════════════
    // F7: M8 structured audit trail verification
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn every_downgrade_has_rule_field_before_after_reason() {
        // Trigger multiple downgrades
        let input = TaskPolicyInput {
            executor: "Other".into(),
            runtime_adapter: "generic".into(),
            execution_surface: "local-workspace".into(),
            permission_mode: "execute-and-verify".into(),
            parallelism: "subagent".into(),
            task_level: "Heavy".into(),
            execution_effort: Some("normal".into()),
            workflow_authority: Some("none".into()),
            approval_source: ApprovalSource::None,
        };
        let policy = resolve_policy(input);

        assert!(!policy.downgrade_reasons.is_empty());

        for reason in &policy.downgrade_reasons {
            assert!(
                !reason.rule_id.is_empty(),
                "downgrade missing rule_id: {:?}",
                reason
            );
            assert!(
                !reason.field.is_empty(),
                "downgrade missing field: {:?}",
                reason
            );
            assert!(
                !reason.before.is_empty(),
                "downgrade missing before: {:?}",
                reason
            );
            assert!(
                !reason.after.is_empty(),
                "downgrade missing after: {:?}",
                reason
            );
            assert!(
                !reason.reason.is_empty(),
                "downgrade missing reason: {:?}",
                reason
            );

            // Each Display line should contain the key audit info
            let display = reason.to_string();
            assert!(
                display.contains(&reason.rule_id),
                "Display missing rule_id: {}",
                display
            );
            assert!(
                display.contains(&reason.field),
                "Display missing field: {}",
                display
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════
    // F8: Heavy never stops by task level — confirmation gate only
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn heavy_edit_without_approval_is_executable_no_stop() {
        // Heavy edit-with-confirmation without approval → executable with a
        // confirmation gate. Task level does NOT downgrade or stop.
        let input = TaskPolicyInput {
            permission_mode: "edit-with-confirmation".into(),
            task_level: "Heavy".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::EditWithConfirmation
        );
        assert!(!policy.was_downgraded);
        assert!(policy.requires_confirmation_gate);
        assert!(
            !policy.stop_before_launch,
            "Heavy write card must not stop by task level"
        );
        assert!(policy.stop_reasons.is_empty());
    }

    #[test]
    fn heavy_execute_without_approval_has_no_heavy_write_stop_reason() {
        // Regression guard: a Heavy execute-and-verify card must NOT produce a
        // level-driven "requires write approval" stop. Task level never stops.
        let input = heavy_execute_no_approval();
        let policy = resolve_policy(input);

        assert!(!policy.stop_before_launch);
        assert!(
            policy.stop_reasons.is_empty(),
            "Heavy task level must not introduce a stop reason: {:?}",
            policy.stop_reasons
        );
    }

    #[test]
    fn heavy_plan_only_without_approval_no_stop() {
        // Heavy plan-only is a plan/review card — NOT a write-request stop.
        // It should only have requires_confirmation_gate, not stop_before_launch.
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Heavy".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy.requires_confirmation_gate);
        assert!(
            !policy.stop_before_launch,
            "Heavy plan-only card should NOT stop (it's a plan/review card, not a write request)"
        );
        assert!(policy.stop_reasons.is_empty());
        assert!(!policy.was_downgraded);
    }

    #[test]
    fn heavy_edit_with_approval_no_stop() {
        // Heavy edit-with-confirmation WITH --approve-writes → no stop
        let input = heavy_edit_with_approval(); // ApprovalSource::CliFlag
        let policy = resolve_policy(input);

        assert!(
            !policy.stop_before_launch,
            "Heavy with approval must not stop before launch"
        );
        assert!(policy.stop_reasons.is_empty());
        assert!(!policy.was_downgraded);
        assert!(policy.requires_confirmation_gate);
    }

    #[test]
    fn heavy_execute_with_runner_env_no_stop() {
        // Heavy execute-and-verify with runner-env approval → no stop
        let input = TaskPolicyInput {
            approval_source: ApprovalSource::RunnerEnv,
            ..heavy_execute_no_approval()
        };
        let policy = resolve_policy(input);

        assert!(!policy.stop_before_launch);
        assert!(policy.stop_reasons.is_empty());
        assert!(!policy.was_downgraded);
    }

    // ═════════════════════════════════════════════════════════════════════
    // F9: M5 effective_parallelism updated to None — Finding 3
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn read_only_with_worktree_sets_effective_parallelism_to_none() {
        // M5 stripping must update effective_parallelism to None
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()), // M7: worktree needs NOT none
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_parallelism,
            Parallelism::None,
            "read-only must set effective_parallelism to None when stripping worktree"
        );
        assert!(policy.stop_before_launch);
        assert!(!policy
            .allowed_launch_args
            .contains(&"--parallel".to_string()));
        assert!(!policy
            .allowed_launch_args
            .contains(&"--worktree".to_string()));
    }

    #[test]
    fn plan_only_with_worktree_sets_effective_parallelism_to_none() {
        // plan-only + worktree → effective_parallelism must be None
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_parallelism,
            Parallelism::None,
            "plan-only must set effective_parallelism to None when stripping worktree"
        );
        assert!(policy.stop_before_launch);
    }

    #[test]
    fn plan_only_with_subagent_sets_effective_parallelism_to_none() {
        // plan-only + subagent → effective_parallelism must be None
        // M7 allows through (plan-only authority passes for subagent),
        // but M5 strips because plan-only forbids writes
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            parallelism: "subagent".into(),
            workflow_authority: Some("plan-only".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(
            policy.effective_parallelism,
            Parallelism::None,
            "plan-only must set effective_parallelism to None when stripping subagent"
        );
        assert!(!policy
            .allowed_launch_args
            .contains(&"--parallel".to_string()));
    }

    #[test]
    fn read_only_with_agent_team_sets_effective_parallelism_to_none() {
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "agent-team".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert!(policy.stop_before_launch);
    }

    #[test]
    fn m5_downgrade_after_field_is_canonical_none() {
        // The M5 downgrade reason's `after` field must be "none",
        // not "none (launch args stripped)".
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        let m5_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.rule_id == "M5")
            .collect();
        assert_eq!(m5_reasons.len(), 1);
        assert_eq!(
            m5_reasons[0].after, "none",
            "M5 downgrade after field must be canonical 'none', got '{}'",
            m5_reasons[0].after
        );
        // The `effective_parallelism` must also be None (consistency check)
        assert_eq!(
            policy.effective_parallelism,
            Parallelism::None,
            "effective_parallelism must match downgrade after field"
        );
    }

    #[test]
    fn m5_effective_parallelism_consistent_with_downgrade_after() {
        // For every M5 downgrade, the after field must match
        // effective_parallelism.to_string()
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        for reason in &policy.downgrade_reasons {
            if reason.field == "parallelism" && reason.rule_id == "M5" {
                assert_eq!(
                    reason.after,
                    policy.effective_parallelism.to_string(),
                    "M5 downgrade after='{}' must match effective_parallelism='{}'",
                    reason.after,
                    policy.effective_parallelism
                );
            }
        }
    }

    // ═════════════════════════════════════════════════════════════════════
    // F10: background-agent + read-only/plan-only — audit gap
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn background_agent_read_only_sets_stop_before_launch() {
        // read-only + background-agent must NOT silently strip --headless;
        // must set stop_before_launch with structured stop_reasons + downgrade.
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "read-only".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(!policy
            .allowed_launch_args
            .contains(&"--headless".to_string()));
        assert!(
            policy.stop_before_launch,
            "read-only + background-agent must set stop_before_launch=true"
        );
        assert!(!policy.stop_reasons.is_empty());
        let reason = policy.stop_reasons.first().unwrap();
        let reason_str = reason.to_string();
        assert!(
            reason_str.to_lowercase().contains("background"),
            "stop_reasons must mention background: {}",
            reason_str
        );
        // Must have a downgrade entry for execution_surface
        assert!(policy.was_downgraded);
        let surface_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.field == "execution_surface")
            .collect();
        assert_eq!(
            surface_reasons.len(),
            1,
            "must record downgrade for execution_surface stripping"
        );
        assert_eq!(surface_reasons[0].rule_id, "M5");
        assert_eq!(surface_reasons[0].before, "background-agent");
    }

    #[test]
    fn background_agent_plan_only_sets_stop_before_launch() {
        // plan-only + background-agent must also stop, not silently strip.
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "plan-only".into(),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy.stop_before_launch);
        assert!(!policy.stop_reasons.is_empty());
        let surface_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.field == "execution_surface")
            .collect();
        assert_eq!(surface_reasons.len(), 1);
        assert_eq!(surface_reasons[0].before, "background-agent");
    }

    #[test]
    fn background_agent_execute_and_verify_no_stop() {
        // execute-and-verify + background-agent still allows --headless,
        // no stop, no downgrade for execution_surface.
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy
            .allowed_launch_args
            .contains(&"--headless".to_string()));
        assert!(!policy.stop_before_launch);
        assert!(policy.stop_reasons.is_empty());
        let surface_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.field == "execution_surface")
            .collect();
        assert!(
            surface_reasons.is_empty(),
            "execute-and-verify must not downgrade execution_surface"
        );
    }

    #[test]
    fn background_agent_stop_reason_json_serialization() {
        // Verify the stop_reasons serialize with canonical kind value.
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "read-only".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        assert!(
            json.contains("background-surface-blocked-by-permission"),
            "JSON stop_reasons kind must be canonical: {}",
            json
        );
    }

    #[test]
    fn background_agent_downgrade_reason_has_canonical_after() {
        // The downgrade reason after field must be a canonical protocol string.
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "plan-only".into(),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        let surface_reasons: Vec<_> = policy
            .downgrade_reasons
            .iter()
            .filter(|r| r.field == "execution_surface")
            .collect();
        assert_eq!(surface_reasons.len(), 1);
        // After must be a valid execution surface value, not empty or garbled
        assert!(
            !surface_reasons[0].after.is_empty(),
            "downgrade after must not be empty"
        );
        assert_ne!(surface_reasons[0].after, "background-agent");
    }

    #[test]
    fn combined_worktree_background_agent_preserves_all_stop_reasons() {
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        let json = serde_json::to_string(&policy).unwrap();

        assert!(policy.stop_before_launch);
        assert_eq!(
            policy.stop_reasons.len(),
            2,
            "combined worktree + background-agent blocks must preserve both stop reasons: {}",
            json
        );
        assert!(
            json.contains("writable-parallelism-blocked-by-permission"),
            "missing parallelism stop reason: {}",
            json
        );
        assert!(
            json.contains("background-surface-blocked-by-permission"),
            "missing background surface stop reason: {}",
            json
        );
        assert!(policy
            .downgrade_reasons
            .iter()
            .any(|r| r.field == "parallelism"));
        assert!(policy
            .downgrade_reasons
            .iter()
            .any(|r| r.field == "execution_surface"));
    }

    #[test]
    fn stopped_plan_only_worktree_background_agent_has_no_launch_args() {
        let input = TaskPolicyInput {
            execution_surface: "background-agent".into(),
            permission_mode: "plan-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("plan-only".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert!(policy.stop_before_launch);
        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert_eq!(policy.effective_execution_surface, "cli");
        assert_eq!(
            policy.stop_reasons.len(),
            2,
            "parallelism and background-agent stop reasons must both survive"
        );
        assert!(
            policy.allowed_launch_args.is_empty(),
            "stopped policies must not expose even safe launch args"
        );
    }

    #[test]
    fn stop_before_launch_implies_empty_allowed_launch_args() {
        let cases = [
            TaskPolicyInput {
                permission_mode: "read-only".into(),
                parallelism: "worktree".into(),
                workflow_authority: Some("allowed".into()),
                task_level: "Medium".into(),
                ..light_execute_input()
            },
            TaskPolicyInput {
                permission_mode: "plan-only".into(),
                parallelism: "worktree".into(),
                workflow_authority: Some("plan-only".into()),
                task_level: "Medium".into(),
                ..light_execute_input()
            },
            TaskPolicyInput {
                execution_surface: "background-agent".into(),
                permission_mode: "plan-only".into(),
                task_level: "Medium".into(),
                ..light_execute_input()
            },
        ];

        for input in cases {
            let policy = resolve_policy(input);
            assert!(policy.stop_before_launch);
            assert!(
                policy.allowed_launch_args.is_empty(),
                "stop_before_launch=true must make allowed_launch_args empty: {:?}",
                policy
            );
        }
    }

    // ═════════════════════════════════════════════════════════════════════
    // Regression: all original tests must still pass their assertions
    // ═════════════════════════════════════════════════════════════════════

    // ═════════════════════════════════════════════════════════════════════
    // M3: schema_version in JSON output
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn json_output_has_schema_version() {
        let policy = resolve_policy(light_execute_input());
        let json = serde_json::to_string(&policy).unwrap();
        assert!(
            json.contains("\"schema_version\":\"2.0-m3\""),
            "JSON must include schema_version '2.0-m3': {}",
            json
        );
    }

    // ═════════════════════════════════════════════════════════════════════
    // M3: gate_check integration tests
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn gate_check_light_execute_returns_allow() {
        let input = light_execute_input();
        let output = gate_check(&input);
        assert_eq!(output.decision, GateDecision::Allow);
        assert_eq!(output.schema_version, "2.0-m3");
    }

    #[test]
    fn gate_check_heavy_plan_only_returns_confirm() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Heavy".into(),
            ..light_execute_input()
        };
        let output = gate_check(&input);
        assert_eq!(output.decision, GateDecision::Confirm);
    }

    #[test]
    fn gate_check_writability_stop_returns_stop() {
        // Genuine resolver STOP: read-only + worktree (writability gate). Task
        // level never stops, so Heavy alone is no longer a stop vehicle.
        let input = TaskPolicyInput {
            permission_mode: "read-only".into(),
            parallelism: "worktree".into(),
            workflow_authority: Some("allowed".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let output = gate_check(&input);
        assert_eq!(output.decision, GateDecision::Stop);
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"decision\":\"stop\""));
        assert!(json.contains("\"resolved_policy\""));
    }

    #[test]
    fn gate_check_failed_produces_structured_stop_json() {
        let output = gate_check_failed(
            "protected_path_violation",
            vec!["Cannot modify example-stable-suite".to_string()],
        );
        assert_eq!(output.decision, GateDecision::Stop);
        assert_eq!(output.error_kind, "protected_path_violation");
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"decision\":\"stop\""));
        assert!(json.contains("\"error_kind\":\"protected_path_violation\""));
        assert!(json.contains("example-stable-suite"));
    }

    #[test]
    fn gate_check_json_has_all_required_fields() {
        let output = gate_check(&light_execute_input());
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"schema_version\":\"2.0-m3\""));
        assert!(json.contains("\"decision\""));
        assert!(json.contains("\"resolved_policy\""));
    }

    // ── T22: plan-only claude-code → exhaustive no-write-flag check ──

    #[test]
    fn plan_only_claude_has_no_write_flags() {
        let input = TaskPolicyInput {
            permission_mode: "plan-only".into(),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);

        assert_eq!(policy.effective_permission_mode, PermissionMode::PlanOnly);
        assert!(policy
            .allowed_launch_args
            .contains(&"--permission-mode".to_string()));
        assert!(policy.allowed_launch_args.contains(&"plan".to_string()));

        // M5/M6: plan-only must NEVER produce write-enabling flags
        let args_str = policy.allowed_launch_args.join(" ");
        for forbidden in &["--parallel", "--worktree", "--headless"] {
            assert!(
                !args_str.contains(forbidden),
                "plan-only must not produce {}",
                forbidden
            );
        }
        for forbidden in &["acceptEdits", "bypassPermissions"] {
            assert!(
                !args_str.contains(forbidden),
                "plan-only must not produce permission value {}",
                forbidden
            );
        }
    }

    // ── 0.2.7: neutral exhaustive effort maps to is_exhaustive_mode ────

    #[test]
    fn neutral_exhaustive_effort_sets_exhaustive_mode_no_escalation() {
        let input = TaskPolicyInput {
            execution_effort: Some("exhaustive".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        assert!(policy.is_exhaustive_mode);
        assert_eq!(policy.execution_effort, "exhaustive");
        // No permission / parallelism / launch-arg escalation (M1-M3).
        assert_eq!(
            policy.effective_permission_mode,
            PermissionMode::ExecuteAndVerify
        );
        assert_eq!(policy.effective_parallelism, Parallelism::None);
        assert!(!policy
            .allowed_launch_args
            .contains(&"--permission-mode".to_string()));
    }

    #[test]
    fn ultracode_alias_still_maps_to_exhaustive_mode() {
        // The legacy ultracode alias resolves to the same is_exhaustive_mode as
        // the neutral exhaustive value (parse-compatibility).
        let input = TaskPolicyInput {
            execution_effort: Some("ultracode".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let policy = resolve_policy(input);
        assert!(policy.is_exhaustive_mode);
        assert_eq!(policy.execution_effort, "ultracode");
    }

    #[test]
    fn neutral_low_high_effort_do_not_set_exhaustive_mode() {
        for effort in ["low", "normal", "high"] {
            let input = TaskPolicyInput {
                execution_effort: Some(effort.into()),
                ..light_execute_input()
            };
            let policy = resolve_policy(input);
            assert!(
                !policy.is_exhaustive_mode,
                "effort `{effort}` must not set exhaustive mode"
            );
        }
    }

    #[test]
    fn explain_m1_applied_for_neutral_exhaustive() {
        let input = TaskPolicyInput {
            execution_effort: Some("exhaustive".into()),
            task_level: "Medium".into(),
            ..light_execute_input()
        };
        let output = explain_policy(&input);
        let m1 = output
            .explanations
            .iter()
            .find(|e| e.rule_id == "M1")
            .unwrap();
        assert_eq!(m1.decision, "applied");
    }
}
