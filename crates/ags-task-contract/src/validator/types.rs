//! Parsed-card struct and stable error-code constants.

/// Parsed fields from a validated task card.
///
/// This is the structured output of `parse_validated()`, ready to be
/// consumed by the execution-policy resolver. There is a single canonical
/// task-card format (the classic fixed skeleton), so no card-type
/// discriminator is carried.
#[derive(Debug, Clone)]
pub struct ParsedTaskCard {
    /// Parsed field-name → value map (keys like `"Executor:"`, `"任务级别："`, etc.)
    pub fields: std::collections::HashMap<String, String>,
}

/// Stable identifiers that connect one task card to its delivery report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskClosureContract {
    pub contract_id: String,
    pub handoff_source: String,
    pub goal_ids: Vec<String>,
    pub acceptance_criteria_ids: Vec<String>,
    pub verification_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
}

// ── Error codes ────────────────────────────────────────────────────────

/// Stable error codes for machine-consumable error classification.
pub mod error_code {
    pub const INVALID_FIELD_VALUE: &str = "INVALID_FIELD_VALUE";
    pub const FIELD_COMBINATION_MISMATCH: &str = "FIELD_COMBINATION_MISMATCH";
    pub const PROTECTED_PATH_VIOLATION: &str = "PROTECTED_PATH_VIOLATION";
    pub const RISK_LEVEL_MISMATCH: &str = "RISK_LEVEL_MISMATCH";
    pub const EMPTY_OR_WEAK_SECTION: &str = "EMPTY_OR_WEAK_SECTION";
    pub const CONTRADICTORY_REQUIREMENT: &str = "CONTRADICTORY_REQUIREMENT";
    pub const EXECUTION_EFFORT_POLICY_VIOLATION: &str = "EXECUTION_EFFORT_POLICY_VIOLATION";
    pub const DELEGATION_PLANNING_REQUIRED: &str = "DELEGATION_PLANNING_REQUIRED";
    pub const EXECUTION_MODE_AUTHORITY_VIOLATION: &str = "EXECUTION_MODE_AUTHORITY_VIOLATION";
    pub const EXECUTION_TOPOLOGY_POLICY_VIOLATION: &str = "EXECUTION_TOPOLOGY_POLICY_VIOLATION";
    pub const SUBTASK_ORCHESTRATION_VIOLATION: &str = "SUBTASK_ORCHESTRATION_VIOLATION";
    pub const PLAN_ONLY_DELIVERY_VIOLATION: &str = "PLAN_ONLY_DELIVERY_VIOLATION";
    pub const HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF: &str =
        "HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF";
    pub const HEAVY_EXECUTABLE_MISSING_REVIEW_GATE: &str = "HEAVY_EXECUTABLE_MISSING_REVIEW_GATE";
    pub const UNKNOWN_OR_INACTIVE_SKILL_TAG: &str = "UNKNOWN_OR_INACTIVE_SKILL_TAG";
    pub const CONTRACT_ID_INVALID: &str = "CONTRACT_ID_INVALID";
    pub const HANDOFF_SOURCE_INVALID: &str = "HANDOFF_SOURCE_INVALID";
    pub const CLOSURE_ID_INVALID: &str = "CLOSURE_ID_INVALID";
    pub const CLOSURE_MAPPING_INCOMPLETE: &str = "CLOSURE_MAPPING_INCOMPLETE";
}
