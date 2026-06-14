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
    pub const WORKFLOW_AUTHORITY_REQUIRED: &str = "WORKFLOW_AUTHORITY_REQUIRED";
    pub const WORKFLOW_AUTHORITY_VIOLATION: &str = "WORKFLOW_AUTHORITY_VIOLATION";
    pub const PARALLELISM_POLICY_VIOLATION: &str = "PARALLELISM_POLICY_VIOLATION";
    pub const ULTRACODE_AUTHORITY_ABUSE: &str = "ULTRACODE_AUTHORITY_ABUSE";
    pub const PLAN_ONLY_DELIVERY_VIOLATION: &str = "PLAN_ONLY_DELIVERY_VIOLATION";
    pub const HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF: &str =
        "HEAVY_PLAN_ONLY_MISSING_REVIEW_HANDOFF";
    pub const PLAN_ONLY_EXECUTION_VERB_DETECTED: &str = "PLAN_ONLY_EXECUTION_VERB_DETECTED";
    pub const FIELD_ABUSE_DETECTED: &str = "FIELD_ABUSE_DETECTED";
    pub const AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE: &str =
        "AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE";
}
