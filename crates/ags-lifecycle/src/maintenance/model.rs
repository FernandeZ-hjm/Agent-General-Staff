use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const MAINTENANCE_INTENT_SCHEMA: &str = "0.5.0-maintenance-intent";
pub const MAINTENANCE_PLAN_SCHEMA: &str = "0.5.0-maintenance-plan";
pub const MAINTENANCE_RECEIPT_SCHEMA: &str = "0.5.0-maintenance-receipt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceSubject {
    Ags,
    Runtime,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceOperation {
    Check,
    Install,
    Update,
    Remove,
    Rollback,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceIntent {
    pub schema_version: String,
    pub request_id: String,
    pub subject: MaintenanceSubject,
    pub operation: MaintenanceOperation,
    pub target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MaintenanceSource>,
    /// Closed, subject-specific planning inputs. Values are data only and are
    /// never interpreted as shell commands.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

impl MaintenanceIntent {
    pub fn new(
        request_id: impl Into<String>,
        subject: MaintenanceSubject,
        operation: MaintenanceOperation,
        target: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: MAINTENANCE_INTENT_SCHEMA.to_string(),
            request_id: request_id.into(),
            subject,
            operation,
            target: target.into(),
            target_hosts: Vec::new(),
            requested_channel: None,
            source: None,
            options: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceSource {
    pub kind: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdirectory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_review_status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RiskClass {
    Blocking,
    AcknowledgementRequired,
    Advisory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskFinding {
    pub id: String,
    pub class: RiskClass,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedWrite {
    pub operation: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationStep {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequirement {
    pub host: String,
    pub requires_restart: bool,
    pub requires_repreflight: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_route_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPoint {
    pub id: String,
    pub state_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedMaintenance {
    pub current_version: Option<String>,
    pub target_version: Option<String>,
    pub source: Option<MaintenanceSource>,
    pub planned_writes: Vec<PlannedWrite>,
    pub risks: Vec<RiskFinding>,
    pub verification_steps: Vec<VerificationStep>,
    pub activation: Vec<ActivationRequirement>,
    pub recovery_point: Option<RecoveryPoint>,
    pub metadata: BTreeMap<String, String>,
    pub payload: Option<MaintenancePayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInstallFile {
    pub path: PathBuf,
    pub description: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedRuntimeSetup {
    pub source_root: PathBuf,
    pub runtime_home: PathBuf,
    pub host_home: PathBuf,
    pub force: bool,
    pub files: Vec<RuntimeInstallFile>,
    pub file_before_state_hashes: BTreeMap<PathBuf, String>,
    pub cleanup_paths: Vec<PathBuf>,
    pub cleanup_before_state_hashes: BTreeMap<PathBuf, String>,
    pub suite_skills: crate::suite_skill_projection::PreparedSuiteSkillProjection,
    /// One-way conversion of catalog Skills projected by the retired suite
    /// layout into machine-local InstalledSkillRecords. These changes share
    /// the runtime setup plan, approval, activation and recovery boundary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_skill_migrations:
        Vec<ags_capability_governance::skill_adoption::PreparedSkillChange>,
}

/// Typed, subject-owned facts sealed directly into the one authoritative
/// MaintenancePlan. A subject backend may prepare data, but it may not create
/// a second plan lifecycle, expiry clock, approval set, or receipt identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "subject", content = "change", rename_all = "kebab-case")]
pub enum MaintenancePayload {
    Skill(Box<ags_capability_governance::skill_adoption::PreparedSkillChange>),
    SuiteSkills(crate::suite_skill_projection::PreparedSuiteSkillProjection),
    RuntimeSetup(PreparedRuntimeSetup),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenancePlan {
    pub schema_version: String,
    pub plan_hash: String,
    pub binding_id: String,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub intent: MaintenanceIntent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MaintenanceSource>,
    pub planned_writes: Vec<PlannedWrite>,
    pub risks: Vec<RiskFinding>,
    pub verification_steps: Vec<VerificationStep>,
    pub activation: Vec<ActivationRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_point: Option<RecoveryPoint>,
    pub required_acknowledgements: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<MaintenancePayload>,
}

impl MaintenancePlan {
    pub fn canonical_hash(&self) -> Result<String, String> {
        let mut value = serde_json::to_value(self)
            .map_err(|error| format!("cannot serialize maintenance plan: {error}"))?;
        value
            .as_object_mut()
            .ok_or_else(|| "maintenance plan must be an object".to_string())?
            .remove("plan_hash");
        let bytes = serde_json::to_vec(&value)
            .map_err(|error| format!("cannot canonicalize maintenance plan: {error}"))?;
        Ok(ags_platform::sha256_hex(&bytes))
    }

    pub fn seal(&mut self) -> Result<(), String> {
        self.plan_hash.clear();
        self.plan_hash = self.canonical_hash()?;
        Ok(())
    }

    pub fn verify_hash(&self) -> Result<(), String> {
        let actual = self.canonical_hash()?;
        if actual == self.plan_hash {
            Ok(())
        } else {
            Err("maintenance plan hash mismatch".to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenanceStatus {
    Applied,
    Verified,
    Recovered,
    FailedRecovered,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaintenancePhase {
    Apply,
    Verify,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResult {
    pub id: String,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationResult {
    pub host: String,
    pub activated: bool,
    pub repreflight_passed: bool,
    pub route_verified: bool,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceExecution {
    pub status: MaintenanceStatus,
    pub applied_writes: Vec<PlannedWrite>,
    pub verification_results: Vec<VerificationResult>,
    pub activation_results: Vec<ActivationResult>,
    pub recovery_status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceReceipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub plan_hash: String,
    pub binding_id: String,
    pub completed_at_unix: u64,
    pub phase: MaintenancePhase,
    pub status: MaintenanceStatus,
    pub applied_writes: Vec<PlannedWrite>,
    pub verification_results: Vec<VerificationResult>,
    pub activation_results: Vec<ActivationResult>,
    pub recovery_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
