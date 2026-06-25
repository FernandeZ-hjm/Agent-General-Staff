//! Execution policy input types.
//!
//! `TaskPolicyInput` is the structured bridge between the validator's parsed
//! field map and the execution-policy resolver.  It accepts already-parsed
//! string values — no raw task-card text parsing happens here.

use std::collections::HashMap;

use super::policy::ApprovalSource;

/// Structured input to the execution-policy resolver.
///
/// All fields come from a validated task card.  String values match the
/// canonical field-value sets from `protocol/runtime-adapters.md`.
///
/// # Default semantics
///
/// | Field | When absent / empty |
/// |---|---|
/// | `execution_effort` | `"unknown"` |
/// | `workflow_authority` | `"none"` |
/// | `approval_source` | `ApprovalSource::None` |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPolicyInput {
    /// Executor: "Codex" | "Claude Code" | "Cursor" | "Other"
    pub executor: String,

    /// Runtime adapter: "codex-local" | "claude-code" | "cursor" | "generic"
    pub runtime_adapter: String,

    /// Execution surface: "local-workspace" | "cli" | "ide" | "web"
    ///   | "remote-control" | "background-agent"
    pub execution_surface: String,

    /// Permission mode: "read-only" | "plan-only" | "edit-with-confirmation" | "execute-and-verify"
    pub permission_mode: String,

    /// Parallelism: "none" | "subagent" | "worktree" | "multi-session" | "agent-team"
    pub parallelism: String,

    /// Task level: "Light" | "Medium" | "Heavy"
    pub task_level: String,

    /// Execution effort: neutral tiers "low" | "normal" | "high" | "exhaustive"
    /// (absent → "unknown"). "ultracode" is a legacy parse-compatible alias that
    /// maps to the same exhaustive semantics as "exhaustive".
    pub execution_effort: Option<String>,

    /// Workflow authority: "none" | "within-card" | "plan-only" | "allowed" (absent → "none")
    pub workflow_authority: Option<String>,

    /// Structured write-approval **audit / hint** signal source.
    ///
    /// Task-card text is **never** an approval source.  Only a CLI flag
    /// (`--approve-writes`) or runner environment override can set this to
    /// `CliFlag` or `RunnerEnv`, and the `from_fields()` constructor always
    /// returns `ApprovalSource::None`.  This signal is audit/hint only — it is
    /// NOT a task-level execution unlock (the task LEVEL is decoupled from
    /// execution authority; the permission MODE is the authority).  The M9
    /// generic-adapter cap may consult `is_approved()` as an adapter-capability
    /// override.
    pub approval_source: ApprovalSource,
}

impl TaskPolicyInput {
    /// Build a TaskPolicyInput from a parsed task-card field map.
    ///
    /// This is the bridge from `task_card_validator::parse_validated()` output.
    /// It maps the parsed `HashMap<String, String>` field-name→value pairs
    /// to the structured `TaskPolicyInput` fields.
    ///
    /// Fields that are absent in the map get default values:
    /// - `execution_effort` → `None` (resolves to `"unknown"`)
    /// - `workflow_authority` → `None` (resolves to `"none"`)
    /// - `approval_source` → `ApprovalSource::None` (task card text is never approval)
    pub fn from_fields(fields: &HashMap<String, String>) -> Self {
        Self {
            executor: fields.get("Executor:").cloned().unwrap_or_default(),
            runtime_adapter: fields.get("Runtime adapter:").cloned().unwrap_or_default(),
            execution_surface: fields
                .get("Execution surface:")
                .cloned()
                .unwrap_or_default(),
            permission_mode: fields.get("Permission mode:").cloned().unwrap_or_default(),
            parallelism: fields.get("Parallelism:").cloned().unwrap_or_default(),
            task_level: fields.get("任务级别：").cloned().unwrap_or_default(),
            execution_effort: fields.get("Execution effort:").cloned(),
            workflow_authority: fields.get("Workflow authority:").cloned(),
            approval_source: ApprovalSource::None, // never from task card text
        }
    }

    /// Build a TaskPolicyInput from parsed fields plus STRUCTURED approval
    /// signals. Approval is never read from task-card text (`from_fields` always
    /// yields `None`); callers pass the explicit signals they detected:
    /// - `approve_writes` (CLI `--approve-writes` / runner env) → `CliFlag`.
    /// - `current_task_approval` (host detected an explicit live execution
    ///   instruction) → `CurrentTaskInstruction`.
    ///
    /// These are audit/hint signals, NOT a Heavy execution unlock: the resolver
    /// no longer downgrades a card by task level, so a Heavy card is executable
    /// from its declared permission mode alone (gated by the confirmation/review
    /// gate). `approve_writes` may still act as the M9 generic-adapter capability
    /// override. The stronger source wins when both are set. This is the single
    /// canonical mapping shared by the CLI gate and the AGS MCP
    /// `ags_policy_resolve` tool, so CLI and MCP hosts resolve identical policy.
    pub fn from_fields_with_approval(
        fields: &HashMap<String, String>,
        approve_writes: bool,
        current_task_approval: bool,
    ) -> Self {
        let mut input = Self::from_fields(fields);
        if approve_writes {
            input.approval_source = ApprovalSource::CliFlag;
        } else if current_task_approval {
            input.approval_source = ApprovalSource::CurrentTaskInstruction;
        }
        input
    }

    /// Return the effective execution effort, defaulting to `"unknown"`.
    pub fn effort(&self) -> &str {
        self.execution_effort.as_deref().unwrap_or("unknown")
    }

    /// Whether the effective execution effort is the exhaustive tier. `exhaustive`
    /// is the neutral canonical value; `ultracode` is the legacy parse-compatible
    /// alias mapping to the same exhaustive semantics. Effort is thinking-intensity
    /// only — being exhaustive never escalates permission, parallelism, or args.
    pub fn is_exhaustive_effort(&self) -> bool {
        matches!(self.effort(), "exhaustive" | "ultracode")
    }

    /// Return the effective workflow authority, defaulting to `"none"`.
    pub fn authority(&self) -> &str {
        self.workflow_authority.as_deref().unwrap_or("none")
    }

    /// Build a TaskPolicyInput with defaults for every field.
    /// Useful for tests that only need to vary a few fields.
    pub fn minimal() -> Self {
        Self {
            executor: String::new(),
            runtime_adapter: String::new(),
            execution_surface: String::new(),
            permission_mode: String::new(),
            parallelism: String::new(),
            task_level: String::new(),
            execution_effort: None,
            workflow_authority: None,
            approval_source: ApprovalSource::None,
        }
    }
}
