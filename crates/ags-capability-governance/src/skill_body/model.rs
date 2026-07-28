use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "0.3.5-skill-inventory";

/// Status of a discovered skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillStatus {
    /// Skill is declared in the required profile.
    Available,
    /// Skill is declared in the optional profile.
    Optional,
    /// Skill is in the personal profile (not for public distribution).
    Personal,
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
}
