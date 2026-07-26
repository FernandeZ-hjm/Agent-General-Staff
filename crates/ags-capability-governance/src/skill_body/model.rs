use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "2.0-skill";

/// Status of a discovered skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillStatus {
    /// Skill is adopted and available in the required profile.
    Available,
    /// Skill is adopted and available in the optional profile.
    Optional,
    /// Skill is in the personal profile (not for public distribution).
    Personal,
    /// Skill is in the manifest but not found on disk / not installed.
    Missing,
    /// Skill is explicitly ignored (in ignore-list).
    Disabled,
    /// Skill is present but degraded (version mismatch, missing hash, etc.).
    Degraded,
}

/// A single skill entry from the governance system.
#[derive(Debug, Clone, Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub status: SkillStatus,
    pub profile: String,
    pub source: Option<String>,
    pub version: Option<String>,
    pub hash: Option<String>,
    pub adopted: Option<String>,
    pub warnings: Vec<String>,
}

/// Result of `scan_skills()`.
#[derive(Debug, Clone, Serialize)]
pub struct SkillScanResult {
    pub schema_version: String,
    pub suite_name: String,
    pub suite_version: String,
    pub skills: Vec<SkillEntry>,
    pub summary: SkillScanSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillScanSummary {
    pub total: usize,
    pub available: usize,
    pub optional: usize,
    pub personal: usize,
    pub missing: usize,
    pub disabled: usize,
    pub degraded: usize,
}

/// Result of `check_skills()`.
#[derive(Debug, Clone, Serialize)]
pub struct SkillCheckResult {
    pub schema_version: String,
    pub governance_files: GovernanceFileStatus,
    pub consistency_checks: Vec<ConsistencyCheck>,
    pub issues: Vec<SkillIssue>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GovernanceFileStatus {
    pub skill_adoption_log: FileStatus,
    pub skill_ignore_list: FileStatus,
    pub suite_manifest: FileStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub path: String,
    pub present: bool,
    pub parseable: bool,
    pub schema_version: Option<String>,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsistencyCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillIssue {
    pub severity: String,
    pub category: String,
    pub detail: String,
}

/// Result of `propose_skills()`.
#[derive(Debug, Clone, Serialize)]
pub struct SkillProposalResult {
    pub schema_version: String,
    pub proposal_type: String,
    pub dry_run: bool,
    pub target_skills: Vec<String>,
    pub proposed_changes: Vec<String>,
    pub blocked_reasons: Vec<String>,
    pub note: String,
}
