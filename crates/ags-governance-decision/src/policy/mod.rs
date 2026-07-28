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
//!                          ├─ apply_exhaustive_effort()      (M1-M3)
//!                          ├─ apply_generic_adapter_rule()   (M9)
//!                          ├─ preserve declared permission state (M4)
//!                          ├─ generate_launch_args()          (M5/M6 enforced)
//!                          ├─ apply_stop_on_stripped_execution_topology()
//!                          ├─ apply_launch_args_writability_gate() (M5-M6 post-check)
//!                          └─ verify_downgrade_invariants()  (M10)
//! ```
//!
//! # Example
//!
//! ```rust
//! use ags_governance_decision::policy::{resolve_policy, TaskPolicyInput, ApprovalSource};
//!
//! let input = TaskPolicyInput {
//!     executor: "Claude Code".into(),
//!     runtime_adapter: "claude-code".into(),
//!     execution_surface: "cli".into(),
//!     execution_mode: "single-writer".into(),
//!     execution_topology: "single".into(),
//!     task_level: "Heavy".into(),
//!     execution_effort: Some("normal".into()),
//!     delegation_planning: Some("no".into()),
//!     approval_source: ApprovalSource::None,
//! };
//!
//! let policy = resolve_policy(input);
//! // Task level is a risk/review tier: a Heavy single-writer card runs
//! // directly — no level-driven planning round or permission downgrade.
//! assert_eq!(policy.effective_execution_mode.to_string(), "single-writer");
//! assert!(!policy.was_downgraded);
//! ```

mod explain;
mod input;
mod model;
mod rules;

pub use explain::{explain_policy, gate_check, gate_check_failed};
pub use input::TaskPolicyInput;
pub use model::{
    ApprovalSource, DowngradeReason, ExecutionMode, ExecutionTopology, GateCheckOutput,
    GateDecision, GateErrorOutput, PolicyExplainOutput, PolicyExplanation, ResolvedExecutionPolicy,
    StopReason, TaskSummary,
};

use rules::{
    apply_exhaustive_effort, apply_generic_adapter_rule, apply_launch_args_writability_gate,
    apply_stop_before_launch_arg_gate, apply_stop_on_stripped_execution_topology,
    apply_stop_on_stripped_headless, build_initial_policy, generate_launch_args,
    verify_downgrade_invariants,
};

/// Resolve execution policy from a validated task card's structured fields.
///
/// Applies all MUST rules in order:
/// 1. Build initial policy from input
/// 2. M1-M3: Exhaustive-effort thinking-intensity rules
/// 3. M9: Generic adapter permission cap
/// 4. M4: Preserve the declared two-state permission independent of task level
/// 5. Generate runtime-specific launch args (M5/M6 enforced inline)
/// 6. M5 enforcement: stop if writability-violating execution topology was stripped
/// 7. M5 enforcement: stop if background-agent surface was stripped
/// 8. Stop finalization: stopped policies expose no launch args
/// 9. M5-M6 post-check: structural invariant on launch args
/// 10. M10: Verify downgrade invariants
pub fn resolve_policy(input: TaskPolicyInput) -> ResolvedExecutionPolicy {
    let mut policy = build_initial_policy(&input);

    // M1-M3: exhaustive effort → thinking intensity only
    apply_exhaustive_effort(&input, &mut policy);

    // M9: generic adapter permission cap (may downgrade the execution mode)
    apply_generic_adapter_rule(&input, &mut policy);

    // Generate runtime-specific launch args (M5/M6 enforced here)
    generate_launch_args(&input, &mut policy);

    // M5 enforcement: stop if execution_topology was stripped due to writability gate
    apply_stop_on_stripped_execution_topology(&input, &mut policy);

    // M5 enforcement: stop if background-agent surface was stripped
    apply_stop_on_stripped_headless(&input, &mut policy);

    // Stop finalization: no launch args are consumable once launch is blocked.
    apply_stop_before_launch_arg_gate(&mut policy);

    // M5-M6: plan-only structural invariant (verified in tests)
    apply_launch_args_writability_gate(&policy);

    // M10: downgrade invariants
    verify_downgrade_invariants(&policy);

    policy
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
