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
use super::policy::{
    DowngradeReason, Parallelism, PermissionMode, ResolvedExecutionPolicy, StopReason,
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
//   M1 – change permission mode
//   M2 – enable parallelism
//   M3 – generate any permission-escalating launch arg
//
// `exhaustive` is the neutral canonical value; `ultracode` is the legacy
// parse-compatible alias mapping to the same semantics.

/// Apply exhaustive-effort thinking-intensity rules (M1, M2, M3).
///
/// Sets `is_exhaustive_mode` when effort is the exhaustive tier (`exhaustive` or
/// the legacy `ultracode` alias).  Does NOT touch permission mode, parallelism,
/// or launch args — those are set by other rules based on the original field
/// values, not on effort.
pub(crate) fn apply_ultracode_rules(input: &TaskPolicyInput, policy: &mut ResolvedExecutionPolicy) {
    if input.is_exhaustive_effort() {
        policy.is_exhaustive_mode = true;
    }
}

// ── M4: Heavy → confirmation/review signal (NO permission downgrade) ─────
//
// Task LEVEL (Light / Medium / Heavy) is a risk/review tier — it is NOT the
// execution authority. The permission MODE is the sole execution authority.
// M4 therefore NEVER rewrites the permission mode: a Heavy task keeps its
// declared permission and only gains a confirmation gate (the runner must
// prompt before mutation) plus the Review gate / stop conditions declared
// elsewhere in the card. Consequences:
//   - Heavy + edit-with-confirmation / execute-and-verify → executable
//     (GateDecision Confirm); execute-and-verify is NOT capped to edit.
//   - Heavy + plan-only / read-only → stays a plan/review card.
// The hard STOP boundaries (protected paths, the read-only/plan-only
// writability gate M5/M6, the generic-adapter cap M9, and release / external /
// destructive stop conditions) are enforced by their own rules, independent of
// the task level. Approval sources (current-task instruction / CLI flag /
// runner env) are audit/hint signals — they are no longer a Heavy execution
// unlock (M9 may still consult `approve_writes` as an adapter-capability
// override).

/// Apply the Heavy task confirmation rule (M4).
///
/// Heavy tasks require a confirmation gate. M4 does NOT downgrade the permission
/// mode, does NOT cap execute-and-verify, and does NOT stop — task level is
/// decoupled from execution authority. This is the only field M4 touches.
pub(crate) fn apply_heavy_permission_rule(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if input.task_level != "Heavy" {
        return;
    }
    // Heavy = risk/review tier: set the confirmation gate, leave the declared
    // permission mode untouched. The runner presents a confirmation prompt
    // before mutation; the card's Review gate and stop conditions still apply.
    policy.requires_confirmation_gate = true;
}

// ── M5–M6: read-only / plan-only → no write-type launch args ────────────
//
// When the effective permission mode forbids writes, the allowed launch args
// must not include anything that enables write operations.  This is enforced
// inside `generate_launch_args()` — this function provides a post-check for
// the structural invariant.

/// Ensure read-only and plan-only policies never carry write-type args (M5, M6).
///
/// This is a post-condition check.  `generate_launch_args()` is the primary
/// enforcement point; this function provides a debug-assertion safety net.
/// In test builds, it panics if the invariant is violated.
pub(crate) fn apply_launch_args_writability_gate(policy: &ResolvedExecutionPolicy) {
    if policy.effective_permission_mode.forbids_writes() {
        // Write-enabling flags that must NEVER appear when forbids_writes():
        //   --parallel (enables multi-agent execution)
        //   --worktree (creates a git worktree — filesystem write)
        //   --headless  (background execution may have side effects in read-only)
        //   --permission-mode acceptEdits
        //   --permission-mode bypassPermissions
        //
        // Safe args for plan-only: --permission-mode plan
        // Safe args for read-only:  (none)
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

// ── M7: Parallelism requires Workflow authority ─────────────────────────

/// Apply parallelism vs Workflow authority rule (M7).
///
/// - `subagent`, `multi-session`, `agent-team` require Workflow authority
///   `within-card` or `allowed`.
/// - `worktree` requires Workflow authority NOT `none`.
/// - Without required authority, parallelism is downgraded to `None`.
pub(crate) fn apply_parallelism_authority_rule(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    let authority = input.authority();

    if !policy.effective_parallelism.is_active() {
        return;
    }

    match &policy.effective_parallelism {
        Parallelism::Subagent | Parallelism::MultiSession | Parallelism::AgentTeam => {
            if authority != "within-card" && authority != "allowed" {
                record_downgrade(
                    policy,
                    DowngradeReason::parallelism_requires_workflow_authority(
                        &policy.effective_parallelism.to_string(),
                        authority,
                    ),
                );
                policy.effective_parallelism = Parallelism::None;
            }
        }
        Parallelism::Worktree => {
            if authority == "none" {
                record_downgrade(
                    policy,
                    DowngradeReason::parallelism_requires_workflow_authority("worktree", "none"),
                );
                policy.effective_parallelism = Parallelism::None;
            }
        }
        Parallelism::None => {} // nothing to downgrade
    }
}

// ── M9: Generic runtime adapter caps permission at plan-only ────────────

/// Apply generic runtime adapter permission cap (M9).
///
/// When the runtime adapter is `generic`, the effective permission mode
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

    if policy.effective_permission_mode == PermissionMode::ExecuteAndVerify
        || policy.effective_permission_mode == PermissionMode::EditWithConfirmation
    {
        record_downgrade(
            policy,
            DowngradeReason::generic_adapter_capped_at_plan_only(
                &policy.effective_permission_mode.to_string(),
            ),
        );
        policy.effective_permission_mode = PermissionMode::PlanOnly;
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

// ── M5 enforcement: stop-on-stripped-parallelism ────────────────────────

/// When the effective permission mode forbids writes but active parallelism
/// was requested, set `stop_before_launch = true` with a clear reason.
///
/// This is the "stop" complement to M5/M6: not only do we strip the launch
/// args, we also tell the runner this task cannot safely launch with the
/// requested parallelism.
pub(crate) fn apply_stop_on_stripped_parallelism(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if !policy.effective_permission_mode.forbids_writes() {
        return;
    }
    // Only stop if the ORIGINAL input requested active parallelism AND the
    // effective parallelism still shows it (it hasn't already been downgraded).
    // But note: we haven't downgraded parallelism for writability reasons yet
    // (M7 only downgrades for authority).  The writability gate is enforced in
    // generate_launch_args by stripping the flags.  So we check the input.
    let requested_parallelism = Parallelism::from_str(&input.parallelism);
    if requested_parallelism.has_filesystem_side_effects() {
        // Record a downgrade for the stripped parallelism
        record_downgrade(
            policy,
            DowngradeReason::parallelism_stripped_for_readonly_mode(
                &requested_parallelism.to_string(),
                &policy.effective_permission_mode.to_string(),
            ),
        );
        // M5: When forbids_writes() is true, the effective parallelism must
        // be set to None — the resolution declares no parallelism is allowed,
        // even if the input requested it and M7 allowed it through.
        policy.effective_parallelism = Parallelism::None;
        // Set stop — runner must not launch with the requested parallelism
        record_stop(
            policy,
            StopReason::WritableParallelismBlockedByPermission {
                requested_parallelism: requested_parallelism.to_string(),
                effective_permission: policy.effective_permission_mode.to_string(),
            },
        );
    }
}

// ── M5 enforcement: stop-on-stripped-headless ────────────────────────────

/// When the effective permission mode forbids writes but background-agent
/// execution surface was requested, set `stop_before_launch = true` with a
/// clear reason.
///
/// This is the "stop" complement to M5/M6: not only do we strip the
/// `--headless` launch arg, we also tell the runner this task cannot safely
/// launch with the requested surface.
pub(crate) fn apply_stop_on_stripped_headless(
    input: &TaskPolicyInput,
    policy: &mut ResolvedExecutionPolicy,
) {
    if !policy.effective_permission_mode.forbids_writes() {
        return;
    }
    if input.execution_surface != "background-agent" {
        return;
    }

    // Record a downgrade for the stripped background-agent surface
    record_downgrade(
        policy,
        DowngradeReason::background_surface_stripped_for_readonly_mode(
            &policy.effective_permission_mode.to_string(),
        ),
    );
    // Downgrade the effective surface to cli — safe interactive fallback
    policy.effective_execution_surface = "cli".to_string();
    // Set stop — runner must not launch headless with read-only/plan-only
    record_stop(
        policy,
        StopReason::BackgroundSurfaceBlockedByPermission {
            effective_permission: policy.effective_permission_mode.to_string(),
        },
    );
}

// ── Stop gate finalization ──────────────────────────────────────────────

/// Enforce the machine contract that a stopped policy is not launchable.
///
/// `allowed_launch_args` are only meaningful when `stop_before_launch=false`.
/// When any stop gate fires, clear them so downstream runners cannot
/// accidentally launch with "safe" args from an otherwise stopped policy.
pub(crate) fn apply_stop_before_launch_arg_gate(policy: &mut ResolvedExecutionPolicy) {
    if policy.stop_before_launch {
        policy.allowed_launch_args.clear();
    }
}

// ── Launch-args generation ──────────────────────────────────────────────

/// Generate runtime-specific launch args based on the resolved policy.
///
/// Rules enforced:
/// - PlanOnly + claude-code: `--permission-mode plan` (safe read-only plan flag)
/// - ReadOnly: no args
/// - Write modes (edit-with-confirmation, execute-and-verify): no special args needed
/// - Active parallelism: runtime-specific parallelism flags (stripped if
///   effective permission forbids writes — M5/M6)
/// - Ultracode does NOT inject any launch arg (M3).
pub(crate) fn generate_launch_args(input: &TaskPolicyInput, policy: &mut ResolvedExecutionPolicy) {
    let mut args: Vec<String> = Vec::new();

    let is_claude = input.runtime_adapter == "claude-code";
    let forbids_writes = policy.effective_permission_mode.forbids_writes();

    // Only claude-code currently has CLI flag mapping.
    // codex-local and cursor are IDE-based; generic has no known CLI.

    if is_claude {
        // Permission-mode flag
        match policy.effective_permission_mode {
            PermissionMode::PlanOnly => {
                args.push("--permission-mode".to_string());
                args.push("plan".to_string());
            }
            PermissionMode::ReadOnly => {
                // No CLI flag for read-only in claude-code; the mode
                // is enforced by the task card content.
            }
            PermissionMode::EditWithConfirmation | PermissionMode::ExecuteAndVerify => {
                // Default claude-code behavior — no special flag needed.
            }
        }

        // Parallelism flags — ONLY when writes are NOT forbidden (M5/M6).
        // read-only and plan-only must never produce --parallel or --worktree
        // because those flags enable filesystem side effects.
        if !forbids_writes {
            match policy.effective_parallelism {
                Parallelism::Subagent | Parallelism::MultiSession | Parallelism::AgentTeam => {
                    args.push("--parallel".to_string());
                }
                Parallelism::Worktree => {
                    args.push("--parallel".to_string());
                    args.push("--worktree".to_string());
                }
                Parallelism::None => {}
            }

            // Background-agent surface — only when writes are not forbidden.
            // Background execution in read-only mode could still have side
            // effects (process spawning, resource consumption).
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
        effective_permission_mode: PermissionMode::from_str(&input.permission_mode),
        effective_parallelism: Parallelism::from_str(&input.parallelism),
        effective_execution_surface: input.execution_surface.clone(),
        allowed_launch_args: Vec::new(),
        stop_before_launch: false,
        stop_reasons: Vec::new(),
        was_downgraded: false,
        downgrade_reasons: Vec::new(),
        requires_confirmation_gate: false,
        execution_effort: input.effort().to_string(),
        is_exhaustive_mode: false,
        approval_source: input.approval_source.clone(),
    }
}
