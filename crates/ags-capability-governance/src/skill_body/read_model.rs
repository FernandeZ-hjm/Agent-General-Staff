use super::model::{SkillEntry, SkillScanResult, SkillScanSummary, SkillStatus, SCHEMA_VERSION};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
struct SuiteManifest {
    suite: Option<SuiteSection>,
}

#[derive(Debug, Clone, Deserialize)]
struct SuiteSection {
    name: Option<String>,
    version: Option<String>,
    required: Option<Vec<SkillManifestEntry>>,
    optional: Option<Vec<SkillManifestEntry>>,
    personal: Option<serde_yaml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillManifestEntry {
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
    hash: Option<String>,
}

// ── Scan ──────────────────────────────────────────────────────────────────

/// Scan the static suite manifest for skill status.
///
/// Reads `manifests/suite.yaml` to produce a structured inventory of all
/// suite-owned skills with their status.
pub fn scan_skills(root: &Path) -> SkillScanResult {
    let manifest_path = root.join("manifests/suite.yaml");

    let mut skills: Vec<SkillEntry> = Vec::new();
    let mut suite_name = "unknown".to_string();
    let mut suite_version = "unknown".to_string();

    // Parse manifest
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_yaml::from_str::<SuiteManifest>(&content) {
            if let Some(suite) = manifest.suite {
                suite_name = suite.name.unwrap_or_else(|| "unknown".to_string());
                suite_version = suite.version.unwrap_or_else(|| "unknown".to_string());

                // Required skills
                if let Some(required) = suite.required {
                    for entry in required {
                        let name = entry.name.unwrap_or_else(|| "unnamed".to_string());
                        skills.push(SkillEntry {
                            name,
                            status: SkillStatus::Available,
                            profile: "required".to_string(),
                            source: entry.source,
                            version: entry.version,
                            hash: entry.hash,
                            warnings: Vec::new(),
                        });
                    }
                }

                // Optional skills
                if let Some(optional) = suite.optional {
                    for entry in optional {
                        let name = entry.name.unwrap_or_else(|| "unnamed".to_string());
                        skills.push(SkillEntry {
                            name,
                            status: SkillStatus::Optional,
                            profile: "optional".to_string(),
                            source: entry.source,
                            version: entry.version,
                            hash: entry.hash,
                            warnings: Vec::new(),
                        });
                    }
                }

                // Personal skills.  The manifest stores these as a mapping so
                // the key remains a stable profile-scoped skill name while the
                // value may carry the same metadata used by required/optional.
                if let Some(personal) = suite.personal {
                    if let Some(personal_map) = personal.as_mapping() {
                        for (key, value) in personal_map {
                            if let Some(name) = key.as_str() {
                                skills.push(SkillEntry {
                                    name: name.to_string(),
                                    status: SkillStatus::Personal,
                                    profile: "personal".to_string(),
                                    source: yaml_field(value, "source"),
                                    version: yaml_field(value, "version"),
                                    hash: yaml_field(value, "hash"),
                                    warnings: vec![
                                        "Personal profile — excluded from public distribution"
                                            .to_string(),
                                    ],
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Build summary
    let summary = SkillScanSummary {
        total: skills.len(),
        available: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Available)
            .count(),
        optional: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Optional)
            .count(),
        personal: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Personal)
            .count(),
    };

    SkillScanResult {
        schema_version: SCHEMA_VERSION.to_string(),
        suite_name,
        suite_version,
        skills,
        summary,
    }
}

pub(super) fn yaml_field(value: &serde_yaml::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}
