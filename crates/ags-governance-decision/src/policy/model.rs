//! Execution policy output types.
//!
//! Defines `ResolvedExecutionPolicy` and all supporting enums that describe
//! how a validated task card should actually execute.

use serde::ser::{Serialize, SerializeMap, Serializer};
use std::fmt;

// ── Execution mode ─────────────────────────────────────────────────────

/// Effective writer scope resolved by the execution-policy engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    PlanOnly,
    SingleWriter,
    FanoutInCard,
    FanoutCrossCard,
}

impl ExecutionMode {
    /// Parse from the canonical task-card string value.
    #[allow(clippy::should_implement_trait)] // inherent infallible parser returning Self, not std::str::FromStr
    pub fn from_str(s: &str) -> Self {
        match s {
            "plan-only" => Self::PlanOnly,
            "single-writer" => Self::SingleWriter,
            "fanout-in-card" => Self::FanoutInCard,
            "fanout-cross-card" => Self::FanoutCrossCard,
            _ => Self::PlanOnly, // fail closed if an unvalidated value reaches the resolver
        }
    }

    /// Whether writes are forbidden under this mode.
    pub fn forbids_writes(&self) -> bool {
        matches!(self, Self::PlanOnly)
    }

    /// Ordered writer scope used by delivery closure checks.
    pub fn rank(&self) -> u8 {
        match self {
            Self::PlanOnly => 0,
            Self::SingleWriter => 1,
            Self::FanoutInCard => 2,
            Self::FanoutCrossCard => 3,
        }
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::PlanOnly => "plan-only",
            Self::SingleWriter => "single-writer",
            Self::FanoutInCard => "fanout-in-card",
            Self::FanoutCrossCard => "fanout-cross-card",
        };
        write!(f, "{}", s)
    }
}

impl Serialize for ExecutionMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ── ExecutionTopology ─────────────────────────────────────────────────────────

/// Effective execution_topology resolved by the execution-policy engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionTopology {
    Single,
    Parallel,
    Worktree,
}

impl ExecutionTopology {
    /// Parse from the canonical task-card string value.
    ///
    #[allow(clippy::should_implement_trait)] // inherent infallible parser returning Self, not std::str::FromStr
    pub fn from_str(s: &str) -> Self {
        match s {
            "single" => Self::Single,
            "parallel" => Self::Parallel,
            "worktree" => Self::Worktree,
            _ => Self::Single, // safest fallback
        }
    }

    /// Whether this is a multi-worker topology.
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Single)
    }

    /// Whether this execution_topology mode has filesystem side effects.
    ///
    /// `Worktree` creates git worktrees on disk and `Parallel` spawns
    /// additional workers. Only `Single` is side-effect neutral.
    pub fn has_filesystem_side_effects(&self) -> bool {
        matches!(self, Self::Worktree | Self::Parallel)
    }

    /// Ordered capability set used by delivery closure checks.
    pub fn permits(&self, used: &Self) -> bool {
        match self {
            Self::Single => matches!(used, Self::Single),
            Self::Parallel => matches!(used, Self::Single | Self::Parallel),
            Self::Worktree => true,
        }
    }
}

impl fmt::Display for ExecutionTopology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Single => "single",
            Self::Parallel => "parallel",
            Self::Worktree => "worktree",
        };
        write!(f, "{}", s)
    }
}

impl Serialize for ExecutionTopology {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ── Approval source ─────────────────────────────────────────────────────

/// Where an explicit write-approval signal came from.
///
/// Task-card text is **never** an approval source — the `from_fields()`
/// constructor always returns `None`.  Approval is a STRUCTURED **audit / hint**
/// signal: it is NO LONGER a Heavy execution unlock, because the task LEVEL is
/// decoupled from execution authority (the permission MODE is the authority, and
/// a Heavy task is executable from its declared permission alone, with the
/// Review gate as its guardrail).  The signal is retained for audit and for the
/// generic-adapter capability cap (M9), which may consult `is_approved()` as an
/// adapter-capability override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalSource {
    /// No explicit write-approval signal recorded.
    None,
    /// Structured current-task signal derived from an explicit user execution
    /// instruction on the live request ("实现 / 修复 / 做完 / 一口气做完 /
    /// 做完核验", detected deterministically by the classifier, NEVER from the
    /// task-card text).  Audit/hint only — it does not change permission or gate
    /// a Heavy task.
    CurrentTaskInstruction,
    /// Signal from the `--approve-writes` CLI flag.  Audit/hint; may act as the
    /// generic-adapter (M9) capability override.
    CliFlag,
    /// Signal from a runner environment variable (`AGS_APPROVE_WRITES=1` or
    /// equivalent).  Audit/hint; may act as the generic-adapter (M9) capability
    /// override.
    RunnerEnv,
}

impl ApprovalSource {
    /// Whether an explicit write-approval signal is present. Used by the
    /// generic-adapter capability cap (M9) as an adapter-capability override;
    /// it is NOT a task-level execution unlock.
    pub fn is_approved(&self) -> bool {
        !matches!(self, Self::None)
    }
}

impl fmt::Display for ApprovalSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::CurrentTaskInstruction => write!(f, "current-task-instruction"),
            Self::CliFlag => write!(f, "cli-flag"),
            Self::RunnerEnv => write!(f, "runner-env"),
        }
    }
}

impl Serialize for ApprovalSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ── Stop reason ─────────────────────────────────────────────────────────

/// Why the executor should stop before launch.
///
/// `stop_before_launch=true` means **do not launch at all** — the task card
/// must be rewritten or the environment corrected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Active execution_topology requested but effective execution mode forbids
    /// writes — any execution_topology flag would create filesystem side effects.
    WritableExecutionTopologyBlockedByPermission {
        requested_execution_topology: String,
        effective_permission: String,
    },
    /// Runtime adapter cannot support the declared execution mode and no
    /// safe downgrade is possible.
    RuntimePermissionGap { adapter: String, requested: String },
    /// Background-agent execution surface requested but effective execution mode
    /// mode forbids writes — background/headless execution could have side
    /// effects incompatible with plan-only.
    BackgroundSurfaceBlockedByPermission { effective_permission: String },
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WritableExecutionTopologyBlockedByPermission {
                requested_execution_topology,
                effective_permission,
            } => {
                write!(
                    f,
                    "ExecutionTopology {} requires filesystem writes but effective execution mode is {}",
                    requested_execution_topology, effective_permission
                )
            }
            Self::RuntimePermissionGap { adapter, requested } => {
                write!(
                    f,
                    "Runtime adapter {} cannot support execution mode {}",
                    adapter, requested
                )
            }
            Self::BackgroundSurfaceBlockedByPermission {
                effective_permission,
            } => {
                write!(
                    f,
                    "Background-agent execution surface requires write-capable execution mode but effective execution mode is {}",
                    effective_permission
                )
            }
        }
    }
}

impl Serialize for StopReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(5))?;
        match self {
            Self::WritableExecutionTopologyBlockedByPermission {
                requested_execution_topology,
                effective_permission,
            } => {
                map.serialize_entry("kind", "writable-execution_topology-blocked-by-permission")?;
                map.serialize_entry("requested_execution_topology", requested_execution_topology)?;
                map.serialize_entry("effective_permission", effective_permission)?;
            }
            Self::RuntimePermissionGap { adapter, requested } => {
                map.serialize_entry("kind", "runtime-permission-gap")?;
                map.serialize_entry("adapter", adapter)?;
                map.serialize_entry("requested", requested)?;
            }
            Self::BackgroundSurfaceBlockedByPermission {
                effective_permission,
            } => {
                map.serialize_entry("kind", "background-surface-blocked-by-permission")?;
                map.serialize_entry("effective_permission", effective_permission)?;
            }
        }
        map.end()
    }
}

// ── Downgrade reason (structured audit trail, M8) ───────────────────────

/// A single downgrade entry supplying the M8-mandated audit trail.
///
/// Every downgrade must record: rule id, affected field, before value,
/// after value, and the human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DowngradeReason {
    /// Canonical rule ID (e.g. "M4", "M7", "M9").
    pub rule_id: String,
    /// The field that was downgraded (e.g. "execution_mode", "execution_topology").
    pub field: String,
    /// Value before downgrade.
    pub before: String,
    /// Value after downgrade.
    pub after: String,
    /// Human-readable explanation.
    pub reason: String,
}

impl DowngradeReason {
    /// Generic runtime adapter caps execution at plan-only.
    pub fn generic_adapter_capped_at_plan_only(requested: &str) -> Self {
        Self {
            rule_id: "M9".to_string(),
            field: "execution_mode".to_string(),
            before: requested.to_string(),
            after: "plan-only".to_string(),
            reason: format!(
                "Generic runtime adapter caps execution at plan-only, requested {requested}"
            ),
        }
    }

    /// Runtime adapter does not support the declared execution_topology mode.
    pub fn runtime_execution_topology_unsupported(adapter: &str, requested: &str) -> Self {
        Self {
            rule_id: "M7".to_string(),
            field: "execution_topology".to_string(),
            before: requested.to_string(),
            after: "single".to_string(),
            reason: format!(
                "Runtime adapter {} does not support execution_topology mode {}",
                adapter, requested
            ),
        }
    }

    /// Write-mode execution_topology stripped because effective execution mode forbids writes.
    pub fn execution_topology_stripped_for_non_mutating_mode(
        requested: &str,
        permission: &str,
    ) -> Self {
        Self {
            rule_id: "M5".to_string(),
            field: "execution_topology".to_string(),
            before: requested.to_string(),
            after: "single".to_string(),
            reason: format!(
                "ExecutionTopology {} stripped because effective execution mode {} forbids filesystem side effects",
                requested, permission
            ),
        }
    }

    /// Background-agent execution surface stripped because effective execution mode
    /// forbids writes — headless background execution could have side effects.
    pub fn background_surface_stripped_for_non_mutating_mode(permission: &str) -> Self {
        Self {
            rule_id: "M5".to_string(),
            field: "execution_surface".to_string(),
            before: "background-agent".to_string(),
            after: "cli".to_string(),
            reason: format!(
                "Background-agent execution surface stripped because effective execution mode {} forbids headless side effects; effective surface falls back to cli",
                permission
            ),
        }
    }
}

impl fmt::Display for DowngradeReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}: {} → {} — {}",
            self.rule_id, self.field, self.before, self.after, self.reason
        )
    }
}

impl Serialize for DowngradeReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(5))?;
        map.serialize_entry("rule_id", &self.rule_id)?;
        map.serialize_entry("field", &self.field)?;
        map.serialize_entry("before", &self.before)?;
        map.serialize_entry("after", &self.after)?;
        map.serialize_entry("reason", &self.reason)?;
        map.end()
    }
}

// ── Resolved execution policy ───────────────────────────────────────────

/// The fully resolved execution policy for a validated task card.
///
/// This is the output of `resolve_policy()`.  It tells the runner (or human)
/// exactly how the task should be launched, what guardrails apply, and what
/// was downgraded from the original request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutionPolicy {
    /// Confirmed executor from the task card.
    pub executor: String,

    /// Confirmed runtime adapter from the task card.
    pub runtime_adapter: String,

    /// Effective execution mode after rule resolution (may be downgraded).
    pub effective_execution_mode: ExecutionMode,

    /// Effective execution_topology after rule resolution (may be downgraded).
    pub effective_execution_topology: ExecutionTopology,

    /// Effective execution surface after rule resolution (may be downgraded).
    /// When background-agent is blocked by plan-only, this falls
    /// back to "cli" — the task can still run interactively.
    pub effective_execution_surface: String,

    /// Whether the card permits planning a delegation structure. This does
    /// not grant writers and never changes execution mode or topology.
    pub delegation_planning: bool,

    /// CLI arguments that are safe to pass to the executor.
    /// For plan-only, this will never include write-enabling flags.
    pub allowed_launch_args: Vec<String>,

    /// If true, a LaunchPlan must not authorize host launch and must expose
    /// `stop_reasons`. The field name is the current wire contract.
    pub stop_before_launch: bool,

    /// Why the launch was stopped (only meaningful if `stop_before_launch`).
    /// Multiple independent gates can stop the same task; preserve all of them.
    pub stop_reasons: Vec<StopReason>,

    /// Whether any field was downgraded from the original request.
    pub was_downgraded: bool,

    /// One entry per downgrade, for audit trail (M8).
    pub downgrade_reasons: Vec<DowngradeReason>,

    /// The declared execution effort from the task card.
    pub execution_effort: String,

    /// If true, the executor should use exhaustive/deep reasoning.
    /// Set only when `execution_effort == "exhaustive"`.
    pub is_exhaustive_mode: bool,

    /// Where explicit write approval came from (never from task card text).
    pub approval_source: ApprovalSource,
}

impl Serialize for ResolvedExecutionPolicy {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry(
            "schema_version",
            "ags://schema/contract/v2/execution-policy",
        )?;
        map.serialize_entry("executor", &self.executor)?;
        map.serialize_entry("runtime_adapter", &self.runtime_adapter)?;
        map.serialize_entry("effective_execution_mode", &self.effective_execution_mode)?;
        map.serialize_entry(
            "effective_execution_topology",
            &self.effective_execution_topology,
        )?;
        map.serialize_entry(
            "effective_execution_surface",
            &self.effective_execution_surface,
        )?;
        map.serialize_entry("delegation_planning", &self.delegation_planning)?;
        map.serialize_entry("allowed_launch_args", &self.allowed_launch_args)?;
        map.serialize_entry("stop_before_launch", &self.stop_before_launch)?;
        map.serialize_entry("stop_reasons", &self.stop_reasons)?;
        map.serialize_entry("was_downgraded", &self.was_downgraded)?;
        map.serialize_entry("downgrade_reasons", &self.downgrade_reasons)?;
        map.serialize_entry("execution_effort", &self.execution_effort)?;
        map.serialize_entry("is_exhaustive_mode", &self.is_exhaustive_mode)?;
        map.serialize_entry("approval_source", &self.approval_source)?;
        map.end()
    }
}

// ── Gate decision ─────────────────────────────────────────────────────────

/// Runner-level gate decision derived from the resolved policy.
///
/// This is the canonical machine-contract between the resolver and any
/// runner: runners must check this decision, not interpret raw policy fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Safe to launch under the resolved execution mode.
    Allow,
    /// Do not launch — task card must be rewritten or approval obtained.
    Stop,
}

impl fmt::Display for GateDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Allow => "allow",
            Self::Stop => "stop",
        };
        write!(f, "{}", s)
    }
}

impl Serialize for GateDecision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

// ── Task summary (lightweight, for explain output) ────────────────────────

/// Lightweight task summary used in explain output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TaskSummary {
    pub executor: String,
    pub task_level: String,
    pub execution_effort: String,
    pub execution_mode: String,
    pub execution_topology: String,
    pub execution_surface: String,
}

// ── Policy explanation (single rule) ──────────────────────────────────────

/// Explanation of a single M1-M10 policy rule's application.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PolicyExplanation {
    /// Canonical rule ID (e.g. "M1", "M4", "M7").
    pub rule_id: String,
    /// Human-readable rule name.
    pub rule_name: String,
    /// Decision: "applied" (rule changed something), "passed" (checked, no
    /// change needed), or "not_applicable" (rule was irrelevant to this input).
    pub decision: String,
    /// The field this rule affected, if any (e.g. "execution_mode").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Human-readable explanation of what happened and why.
    pub detail: String,
}

// ── Policy explain output ─────────────────────────────────────────────────

/// Complete output for `ags govern policy`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PolicyExplainOutput {
    pub schema_version: String,
    pub task_summary: TaskSummary,
    pub explanations: Vec<PolicyExplanation>,
    pub safety_assertions: Vec<String>,
    pub resolved_policy: ResolvedExecutionPolicy,
}

// ── Gate check output ─────────────────────────────────────────────────────

/// Output for `ags govern gate` on a valid task card.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GateCheckOutput {
    pub schema_version: String,
    pub decision: GateDecision,
    pub resolved_policy: ResolvedExecutionPolicy,
}

// ── Gate error output (validation/protected-path failure) ─────────────────

/// Structured error output for `ags govern gate` when validation or
/// protected-path checks fail.  Always carries `decision: stop`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GateErrorOutput {
    pub schema_version: String,
    pub decision: GateDecision,
    /// Machine-readable error category (e.g. "validation_failed",
    /// "protected_path_violation").
    pub error_kind: String,
    /// Human-readable error messages.
    pub errors: Vec<String>,
}
