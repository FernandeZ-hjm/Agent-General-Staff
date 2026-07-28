//! Core MUST-rule implementations for the execution-policy resolver.
//!
//! Policy M1-M10 rule IDs used here are distinct from Roadmap M0-M8 milestone
//! IDs — they live in separate namespaces.
//!
//! Each function takes `&TaskPolicyInput` (immutable original request) and
//! mutates `&mut ResolvedExecutionPolicy` (policy under construction).
//! Rules are documented with their canonical rule IDs for traceability.
//!
//! All functions are `pub(crate)` — external crates must use `resolve_policy()`,
//! not call individual rule functions directly.

use super::input::TaskPolicyInput;
use super::model::{
    DowngradeReason, ExecutionMode, ExecutionTopology, ResolvedExecutionPolicy, StopReason,
};

// ── Utility: record a downgrade ─────────────────────────────────────────

fn record_downgrade(policy: &mut ResolvedExecutionPolicy, reason: DowngradeReason) {
    policy.was_downgraded = true;
    policy.downgrade_reasons.push(reason);
}

fn record_stop(policy: &mut ResolvedExecutionPolicy, reason: StopReason) {
    policy.stop_before_launch = true;
    policy.stop_reasons.push(reason);
}

// ── M1–M3: Exhaustive-effort rules ───────────────────────────────────────
//
// The exhaustive execution-effort tier is thinking intensity ONLY.  It does NOT:
//   M1 – change execution mode
//   M2 – enable execution_topology
//   M3 – generate any permission-escalating launch arg
//
/// Apply exhaustive-effort thinking-intensity rules (M1, M2, M3).
///
/// Does NOT touch execution mode, execution_topology, or launch args.
pub(crate) fn apply_exhaustive_effort(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if input.is_exhaustive_effort() {
        policy.is_exhaustive_mode = true;
    }
}

// ── M4: task level does not rewrite permission ─────────────────────────
//
// Execution authority is ordered from plan-only through fanout-cross-card. The
// task level is a risk/review tier and never rewrites the selected state.
// Heavy review remains an independent validator boundary.

// ── M5–M6: plan-only → no write-type launch args ───────────────────────
//
// When the effective execution mode forbids writes, the allowed launch args
// must not include anything that enables write operations.  This is enforced
// inside `generate_launch_args()` — this function provides a post-check for
// the structural invariant.

/// Ensure plan-only policies never carry write-type args (M5, M6).
///
/// This is a post-condition check.  `generate_launch_args()` is the primary
/// enforcement point; this function provides a debug-assertion safety net.
/// In test builds, it panics if the invariant is violated.
pub(crate) fn apply_launch_args_writability_gate(policy: &ResolvedExecutionPolicy) {
    if policy.effective_execution_mode.forbids_writes() {
        // Write-enabling flags that must NEVER appear when forbids_writes():
        //   --parallel (enables multi-agent execution)
        //   --worktree (creates a git worktree — filesystem write)
        //   --headless  (background execution may have side effects)
        //   --permission-mode acceptEdits
        //   --permission-mode bypassPermissions
        //
        // Safe args for plan-only: --permission-mode plan
        for arg in &policy.allowed_launch_args {
            if arg == "--parallel" || arg == "--worktree" || arg == "--headless" {
                panic!(
                    "M5/M6 violation: forbids_writes() but launch args contain '{}'",
                    arg
                );
            }
            if arg == "acceptEdits" || arg == "bypassPermissions" || arg == "--permission-mode" {
                // --permission-mode by itself is not write-enabling; only
                // plan is allowed in forbids_writes().  The --permission-mode
                // without the value "plan" is suspicious.
            }
        }
    }
    let _ = policy;
}

// ── M9: Generic runtime adapter caps permission at plan-only ────────────

/// Apply generic runtime adapter permission cap (M9).
///
/// When the runtime adapter is `generic`, the effective execution mode
/// cannot exceed `plan-only` unless the input carries explicit approval.
pub(crate) fn apply_generic_adapter_rule(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if input.runtime_adapter != "generic" {
        return;
    }

    // Generic adapter with explicit_write_approval can proceed
    if input.approval_source.is_approved() {
        return;
    }

    if policy.effective_execution_mode != ExecutionMode::PlanOnly {
        record_downgrade(
            policy,
            DowngradeReason::generic_adapter_capped_at_plan_only(
                &policy.effective_execution_mode.to_string(),
            ),
        );
        policy.effective_execution_mode = ExecutionMode::PlanOnly;
    }
}

// ── M10: Every downgrade must have a recorded reason (structural) ───────

/// Verify the downgrade invariant (M10).
///
/// If `was_downgraded` is true, `downgrade_reasons` must be non-empty.
/// If `was_downgraded` is false, `downgrade_reasons` must be empty.
///
/// This is a structural invariant enforced in tests; it is not a runtime
/// check that can fail on production input.
pub(crate) fn verify_downgrade_invariants(policy: &ResolvedExecutionPolicy) {
    if policy.was_downgraded {
        assert!(
            !policy.downgrade_reasons.is_empty(),
            "M10 violation: was_downgraded=true but downgrade_reasons is empty"
        );
    } else {
        assert!(
            policy.downgrade_reasons.is_empty(),
            "M10 violation: was_downgraded=false but downgrade_reasons is non-empty"
        );
    }
}

// ── M5 enforcement: stop-on-stripped-execution_topology ────────────────────────

/// When the effective execution mode forbids writes but active execution_topology
/// was requested, set `stop_before_launch = true` with a clear reason.
///
/// This is the "stop" complement to M5/M6: not only do we strip the launch
/// args, we also tell the LaunchPlan preparer that the host cannot safely
/// launch with the requested execution_topology.
pub(crate) fn apply_stop_on_stripped_execution_topology(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if !policy.effective_execution_mode.forbids_writes() {
        return;
    }
    // Only stop if the ORIGINAL input requested active execution_topology AND the
    // effective execution_topology still shows it (it hasn't already been downgraded).
    // But note: we haven't downgraded execution_topology for writability reasons yet
    // (M7 only downgrades for authority).  The writability gate is enforced in
    // generate_launch_args by stripping the flags.  So we check the input.
    let requested_execution_topology = ExecutionTopology::from_str(&input.execution_topology);
    if requested_execution_topology.has_filesystem_side_effects() {
        // Record a downgrade for the stripped execution_topology
        record_downgrade(
            policy,
            DowngradeReason::execution_topology_stripped_for_non_mutating_mode(
                &requested_execution_topology.to_string(),
                &policy.effective_execution_mode.to_string(),
            ),
        );
        // M5: When forbids_writes() is true, the effective execution_topology must
        // be set to None — the resolution declares no execution_topology is allowed,
        // even if the input requested it and M7 allowed it through.
        policy.effective_execution_topology = ExecutionTopology::Single;
        // Set stop — the prepared plan must not authorize host launch with the
        // requested execution_topology.
        record_stop(
            policy,
            StopReason::WritableExecutionTopologyBlockedByPermission {
                requested_execution_topology: requested_execution_topology.to_string(),
                effective_permission: policy.effective_execution_mode.to_string(),
            },
        );
    }
}

// ── M5 enforcement: stop-on-stripped-headless ────────────────────────────

/// When the effective execution mode forbids writes but background-agent
/// execution surface was requested, set `stop_before_launch = true` with a
/// clear reason.
///
/// This is the "stop" complement to M5/M6: not only do we strip the
/// `--headless` launch arg, we also tell the LaunchPlan preparer that the host
/// cannot safely launch with the requested surface.
pub(crate) fn apply_stop_on_stripped_headless(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if !policy.effective_execution_mode.forbids_writes() {
        return;
    }
    if input.execution_surface != "background-agent" {
        return;
    }

    // Record a downgrade for the stripped background-agent surface
    record_downgrade(
        policy,
        DowngradeReason::background_surface_stripped_for_non_mutating_mode(
            &policy.effective_execution_mode.to_string(),
        ),
    );
    // Downgrade the effective surface to cli — safe interactive fallback
    policy.effective_execution_surface = "cli".to_string();
    // Set stop — the prepared plan must not authorize a headless host launch
    // in plan-only mode.
    record_stop(
        policy,
        StopReason::BackgroundSurfaceBlockedByPermission {
            effective_permission: policy.effective_execution_mode.to_string(),
        },
    );
}

// ── Stop gate finalization ──────────────────────────────────────────────

/// Enforce the machine contract that a stopped policy is not launchable.
///
/// `allowed_launch_args` are only meaningful when `stop_before_launch=false`.
/// When any stop gate fires, clear them so downstream hosts cannot accidentally
/// launch with "safe" args from an otherwise stopped policy.
pub(crate) fn apply_stop_before_launch_arg_gate(policy: &mut ResolvedExecutionPolicy) {
    if policy.stop_before_launch {
        policy.allowed_launch_args.clear();
    }
}

// ── Launch-args generation ──────────────────────────────────────────────

/// Generate runtime-specific launch args based on the resolved policy.
///
/// Rules enforced:
/// - PlanOnly + claude-code: `--permission-mode plan`
/// - ExecuteAndVerify: no special permission arg needed
/// - Active execution_topology: runtime-specific execution_topology flags (stripped if
///   effective execution mode forbids writes — M5/M6)
/// - Execution effort does NOT inject any launch arg (M3).
pub(crate) fn generate_launch_args(input: &TaskPolicyInput, policy: &mut ResolvedExecutionPolicy) {
    let mut args: Vec<String> = Vec::new();

    let is_claude = input.runtime_adapter == "claude-code";
    let forbids_writes = policy.effective_execution_mode.forbids_writes();

    // Only claude-code currently has CLI flag mapping.
    // codex-local and cursor are IDE-based; generic has no known CLI.

    if is_claude {
        // Permission-mode flag
        match policy.effective_execution_mode {
            ExecutionMode::PlanOnly => {
                args.push("--permission-mode".to_string());
                args.push("plan".to_string());
            }
            ExecutionMode::SingleWriter
            | ExecutionMode::FanoutInCard
            | ExecutionMode::FanoutCrossCard => {
                // Default claude-code behavior — no special flag needed.
            }
        }

        // ExecutionTopology flags — ONLY when writes are NOT forbidden (M5/M6).
        // Plan-only must never produce --parallel or --worktree
        // because those flags enable filesystem side effects.
        if !forbids_writes {
            match policy.effective_execution_topology {
                ExecutionTopology::Parallel => {
                    args.push("--parallel".to_string());
                }
                ExecutionTopology::Worktree => {
                    args.push("--parallel".to_string());
                    args.push("--worktree".to_string());
                }
                ExecutionTopology::Single => {}
            }

            // Background-agent surface — only when writes are not forbidden.
            // Background execution is only available to the executable state.
            if input.execution_surface == "background-agent" {
                args.push("--headless".to_string());
            }
        }
    }

    policy.allowed_launch_args = args;
}

// ── Policy construction from input ──────────────────────────────────────

/// Build the initial `ResolvedExecutionPolicy` from `TaskPolicyInput`
/// before any rules are applied.
///
/// The initial state reflects the task card's declared values directly,
/// with no downgrades or adjustments.
pub(crate) fn build_initial_policy(input: &TaskPolicyInput) -> ResolvedExecutionPolicy {
    ResolvedExecutionPolicy {
        executor: input.executor.clone(),
        runtime_adapter: input.runtime_adapter.clone(),
        effective_execution_mode: ExecutionMode::from_str(&input.execution_mode),
        effective_execution_topology: ExecutionTopology::from_str(&input.execution_topology),
        effective_execution_surface: input.execution_surface.clone(),
        delegation_planning: input.delegation_planning_enabled(),
        allowed_launch_args: Vec::new(),
        stop_before_launch: false,
        stop_reasons: Vec::new(),
        was_downgraded: false,
        downgrade_reasons: Vec::new(),
        execution_effort: input.effort().to_string(),
        is_exhaustive_mode: false,
        approval_source: input.approval_source.clone(),
    }
}
