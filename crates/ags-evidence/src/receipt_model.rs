use super::*;

pub const RECEIPT_SCHEMA_VERSION: &str = "2.1-m6";
pub const LEGACY_RECEIPT_SCHEMA_VERSION: &str = "2.0-m6";

// ── Data model ──────────────────────────────────────────────────────────────

/// A verification result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub command: String,
    pub exit_code: i32,
    pub output_hash: String,
}

/// Gate result embedded in receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptSkillSelection {
    pub skill_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
}

/// Optional governance evidence added by the 2.1 writer. It deliberately
/// contains only identifiers/hashes and never the raw request or credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct GovernanceEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solution_state: Option<ags_governance_decision::SolutionState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_selection: Option<ReceiptSkillSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_event_id: Option<String>,
}

/// A task run receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub timestamp: String,
    pub task_card_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_card_path: Option<String>,
    pub gate_result: GateResult,
    pub verification_results: Vec<VerificationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_report_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_status: Option<ags_governance_decision::GovernanceStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_evidence: Option<GovernanceEvidence>,
}

/// A single compliance / verification check item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Result of receipt verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub schema_version: String,
    pub receipt_id: String,
    pub valid: bool,
    pub checks: Vec<CheckItem>,
}

/// Result of compliance checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceResult {
    pub schema_version: String,
    pub receipt_id: String,
    pub compliant: bool,
    pub checks: Vec<CheckItem>,
}
