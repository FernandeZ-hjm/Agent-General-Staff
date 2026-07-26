use super::model::{
    ConsistencyCheck, FileStatus, GovernanceFileStatus, SkillCheckResult, SkillEntry, SkillIssue,
    SkillProposalResult, SkillScanResult, SkillScanSummary, SkillStatus, SCHEMA_VERSION,
};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
struct AdoptionLog {
    #[allow(dead_code)]
    schema_version: Option<String>,
    entries: Option<Vec<serde_yaml::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
struct IgnoreList {
    #[allow(dead_code)]
    schema_version: Option<String>,
    entries: Option<Vec<serde_yaml::Value>>,
}

#[derive(Debug, Clone, Deserialize)]
struct SuiteManifest {
    #[allow(dead_code)]
    schema_version: Option<String>,
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
    adopted: Option<String>,
    #[serde(rename = "entry_ref")]
    entry_ref: Option<String>,
}

// ── Scan ──────────────────────────────────────────────────────────────────

/// Scan the suite manifest and governance files for skill status.
///
/// Reads `manifests/suite.yaml` and related governance files to produce
/// a structured inventory of all known skills with their status.
pub fn scan_skills(root: &Path) -> SkillScanResult {
    let manifest_path = root.join("manifests/suite.yaml");
    let adoption_path = root.join("governance/skill-adoption-log.yaml");
    let ignore_path = root.join("governance/skill-ignore-list.yaml");

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
                            adopted: entry.adopted,
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
                            adopted: entry.adopted,
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
                                    adopted: yaml_field(value, "adopted"),
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

    // Check ignore list for disabled skills
    if let Ok(content) = std::fs::read_to_string(&ignore_path) {
        if let Ok(ignore) = serde_yaml::from_str::<IgnoreList>(&content) {
            if let Some(entries) = ignore.entries {
                for entry in entries {
                    if let Some(name) = entry
                        .get("pattern")
                        .or_else(|| entry.get("skill_name"))
                        .and_then(|v| v.as_str())
                    {
                        // Check if this skill is already in the list
                        let already_known = skills.iter().any(|s| s.name == name);
                        if !already_known {
                            let status = entry
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("active");
                            let is_active = status == "active";
                            // Ignored skills always report as Disabled; the
                            // active flag is read for potential future
                            // divergence but does not currently change status.
                            #[allow(clippy::if_same_then_else)]
                            let skill_status = if is_active {
                                SkillStatus::Disabled
                            } else {
                                SkillStatus::Disabled
                            };
                            skills.push(SkillEntry {
                                name: name.to_string(),
                                status: skill_status,
                                profile: "ignored".to_string(),
                                source: None,
                                version: None,
                                hash: None,
                                adopted: None,
                                warnings: vec![format!("Ignore status: {}", status)],
                            });
                        }
                    }
                }
            }
        }
    }

    // Check adoption log for additional context
    if let Ok(content) = std::fs::read_to_string(&adoption_path) {
        if let Ok(adoption) = serde_yaml::from_str::<AdoptionLog>(&content) {
            if let Some(entries) = adoption.entries {
                for entry in entries {
                    if let Some(decision) = entry.get("decision").and_then(|v| v.as_str()) {
                        if let Some(name) = entry.get("skill_name").and_then(|v| v.as_str()) {
                            let already_known = skills.iter().any(|s| s.name == name);
                            if !already_known && decision == "rejected" {
                                skills.push(SkillEntry {
                                    name: name.to_string(),
                                    status: SkillStatus::Disabled,
                                    profile: "rejected".to_string(),
                                    source: None,
                                    version: None,
                                    hash: None,
                                    adopted: None,
                                    warnings: vec![format!(
                                        "Rejected in adoption log (decision: {})",
                                        decision
                                    )],
                                });
                            } else if let Some(existing) =
                                skills.iter_mut().find(|s| s.name == name)
                            {
                                existing
                                    .warnings
                                    .push(format!("Adoption log entry: decision={}", decision));
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
        missing: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Missing)
            .count(),
        disabled: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Disabled)
            .count(),
        degraded: skills
            .iter()
            .filter(|s| s.status == SkillStatus::Degraded)
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

// ── Check ─────────────────────────────────────────────────────────────────

/// Check governance files for schema compliance and consistency.
///
/// Validates YAML parseability, cross-references adoption log entries
/// with manifest entries, and reports issues.
pub fn check_skills(root: &Path) -> SkillCheckResult {
    let adoption_path = root.join("governance/skill-adoption-log.yaml");
    let ignore_path = root.join("governance/skill-ignore-list.yaml");
    let manifest_path = root.join("manifests/suite.yaml");

    let mut consistency_checks: Vec<ConsistencyCheck> = Vec::new();
    let mut issues: Vec<SkillIssue> = Vec::new();

    // Check governance file status
    let adoption_status = check_file_status(
        &adoption_path,
        "governance/skill-adoption-log.yaml",
        counts_entries,
    );
    let ignore_status = check_file_status(
        &ignore_path,
        "governance/skill-ignore-list.yaml",
        counts_entries,
    );
    let manifest_status =
        check_file_status(&manifest_path, "manifests/suite.yaml", counts_suite_entries);

    // Consistency: all manifest required entries should have adoption log refs
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_yaml::from_str::<SuiteManifest>(&content) {
            if let Some(suite) = manifest.suite {
                let mut manifest_skill_names: Vec<String> = Vec::new();

                if let Some(required) = suite.required {
                    for entry in &required {
                        if let Some(ref name) = entry.name {
                            manifest_skill_names.push(name.clone());
                            if entry.entry_ref.is_none() {
                                issues.push(SkillIssue {
                                    severity: "warn".to_string(),
                                    category: "missing_entry_ref".to_string(),
                                    detail: format!(
                                        "Required skill '{}' has no entry_ref in manifest",
                                        name
                                    ),
                                });
                            }
                        }
                    }
                }

                if let Some(optional) = suite.optional {
                    for entry in &optional {
                        if let Some(ref name) = entry.name {
                            manifest_skill_names.push(name.clone());
                        }
                    }
                }

                // Cross-reference: adoption log should contain all manifest skills
                if let Ok(adoption_content) = std::fs::read_to_string(&adoption_path) {
                    if let Ok(adoption) = serde_yaml::from_str::<AdoptionLog>(&adoption_content) {
                        if let Some(entries) = adoption.entries {
                            let adopted_names: Vec<&str> = entries
                                .iter()
                                .filter_map(|e| e.get("skill_name").and_then(|v| v.as_str()))
                                .collect();

                            let missing_from_adoption: Vec<&String> = manifest_skill_names
                                .iter()
                                .filter(|n| !adopted_names.contains(&n.as_str()))
                                .collect();

                            consistency_checks.push(ConsistencyCheck {
                                name: "manifest-to-adoption-log".to_string(),
                                passed: missing_from_adoption.is_empty(),
                                detail: if missing_from_adoption.is_empty() {
                                    "All manifest skills have adoption log entries".to_string()
                                } else {
                                    format!(
                                        "{} manifest skill(s) missing from adoption log: {}",
                                        missing_from_adoption.len(),
                                        missing_from_adoption
                                            .iter()
                                            .map(|s| s.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                },
                            });
                        }
                    }
                }

                // Schema version consistency check
                let versions = [
                    adoption_status.schema_version.clone(),
                    ignore_status.schema_version.clone(),
                    manifest_status.schema_version.clone(),
                ];
                let all_same = versions.iter().all(|v| *v == versions[0]);
                consistency_checks.push(ConsistencyCheck {
                    name: "schema-version-consistency".to_string(),
                    passed: all_same,
                    detail: if all_same {
                        "All governance files use the same schema version".to_string()
                    } else {
                        format!(
                            "Schema version mismatch: adoption={:?}, ignore={:?}, manifest={:?}",
                            adoption_status.schema_version,
                            ignore_status.schema_version,
                            manifest_status.schema_version
                        )
                    },
                });
            }
        }
    }

    // Check ignore list format
    if let Ok(content) = std::fs::read_to_string(&ignore_path) {
        if let Ok(ignore) = serde_yaml::from_str::<IgnoreList>(&content) {
            if let Some(entries) = &ignore.entries {
                for entry in entries {
                    if entry.get("id").is_none() {
                        issues.push(SkillIssue {
                            severity: "warn".to_string(),
                            category: "ignore_list_format".to_string(),
                            detail: "Ignore list entry missing 'id' field".to_string(),
                        });
                    }
                }
            }
        }
    }

    let all_files_present =
        adoption_status.present && ignore_status.present && manifest_status.present;
    let all_parseable =
        adoption_status.parseable && ignore_status.parseable && manifest_status.parseable;
    let no_fail_issues = !issues.iter().any(|i| i.severity == "fail");
    let all_checks_pass = consistency_checks.iter().all(|c| c.passed);

    SkillCheckResult {
        schema_version: SCHEMA_VERSION.to_string(),
        governance_files: GovernanceFileStatus {
            skill_adoption_log: adoption_status,
            skill_ignore_list: ignore_status,
            suite_manifest: manifest_status,
        },
        consistency_checks,
        issues,
        passed: all_files_present && all_parseable && no_fail_issues && all_checks_pass,
    }
}

fn check_file_status(path: &Path, rel_path: &str, count_fn: fn(&str) -> usize) -> FileStatus {
    let present = path.exists();
    let (parseable, schema_version, entry_count) = if present {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let sv = extract_schema_version(&content);
                let count = count_fn(&content);
                (true, sv, count)
            }
            Err(_) => (false, None, 0),
        }
    } else {
        (false, None, 0)
    };

    FileStatus {
        path: rel_path.to_string(),
        present,
        parseable,
        schema_version,
        entry_count,
    }
}

pub(super) fn extract_schema_version(yaml_content: &str) -> Option<String> {
    for line in yaml_content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("schema_version:") {
            let v = value.trim().trim_matches('"');
            return Some(v.to_string());
        }
    }
    None
}

fn counts_entries(content: &str) -> usize {
    if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(content) {
        doc.get("entries")
            .and_then(|e| e.as_sequence())
            .map(|s| s.len())
            .unwrap_or(0)
    } else {
        0
    }
}

fn counts_suite_entries(content: &str) -> usize {
    if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(content) {
        let mut count = 0;
        if let Some(suite) = doc.get("suite") {
            if let Some(required) = suite.get("required").and_then(|r| r.as_sequence()) {
                count += required.len();
            }
            if let Some(optional) = suite.get("optional").and_then(|o| o.as_sequence()) {
                count += optional.len();
            }
            if let Some(personal) = suite.get("personal").and_then(|p| p.as_mapping()) {
                count += personal.len();
            }
        }
        count
    } else {
        0
    }
}

// ── Propose ───────────────────────────────────────────────────────────────

/// Propose skill changes — dry-run ONLY, no files are modified.
///
/// Shows what WOULD change if a skill were adopted, enabled, or disabled.
/// Always returns a dry-run proposal; actual adoption requires human
/// confirmation and explicit task-card authorization.
pub fn propose_skills(root: &Path, action: &str, skill_name: &str) -> SkillProposalResult {
    let _manifest_path = root.join("manifests/suite.yaml");

    let mut target_skills = vec![skill_name.to_string()];
    let mut proposed_changes: Vec<String> = Vec::new();
    let mut blocked_reasons: Vec<String> = Vec::new();

    // Check current state
    let scan = scan_skills(root);
    let existing = scan.skills.iter().find(|s| s.name == skill_name);

    match action {
        "adopt" => {
            if let Some(existing_skill) = existing {
                proposed_changes.push(format!(
                    "Skill '{}' already exists with status: {:?}",
                    skill_name, existing_skill.status
                ));
                blocked_reasons.push("Skill already known — no changes needed".to_string());
            } else {
                proposed_changes.push(format!(
                    "Would add '{}' to suite manifest as optional",
                    skill_name
                ));
                proposed_changes.push(format!(
                    "Would create adoption log entry for '{}' with decision: adopted",
                    skill_name
                ));
            }
        }
        "enable" => match existing {
            Some(entry) if entry.status == SkillStatus::Disabled => {
                proposed_changes.push(format!(
                    "Would enable '{}' — remove from ignore list",
                    skill_name
                ));
                proposed_changes.push(format!(
                    "Would add '{}' to suite manifest as optional",
                    skill_name
                ));
            }
            Some(entry) => {
                blocked_reasons.push(format!(
                    "Skill '{}' is not disabled (current status: {:?})",
                    skill_name, entry.status
                ));
            }
            None => {
                proposed_changes.push(format!(
                    "Skill '{}' not found — would need adoption first",
                    skill_name
                ));
                blocked_reasons.push("Skill not found in any governance file".to_string());
            }
        },
        "disable" => match existing {
            Some(entry) if entry.status != SkillStatus::Disabled => {
                proposed_changes.push(format!(
                    "Would disable '{}' — add to ignore list",
                    skill_name
                ));
                proposed_changes.push(format!("Would remove '{}' from suite manifest", skill_name));
            }
            Some(entry) => {
                blocked_reasons.push(format!(
                    "Skill '{}' is already disabled (current status: {:?})",
                    skill_name, entry.status
                ));
            }
            None => {
                blocked_reasons.push("Skill not found in any governance file".to_string());
            }
        },
        _ => {
            target_skills.clear();
            blocked_reasons.push(format!("Unknown proposal action: '{}'", action));
        }
    }

    SkillProposalResult {
        schema_version: SCHEMA_VERSION.to_string(),
        proposal_type: action.to_string(),
        dry_run: true,
        target_skills,
        proposed_changes,
        blocked_reasons,
        note: "LEGACY DRY-RUN EVALUATION ONLY — this proposal path always returns dry_run and never modifies files. Foreground candidate lifecycle uses `ags skill adopt|ignore|rollback` and the machine-private overlay; AGS-owned thin-index distribution is a separate `skill sync` / `capability sync` operation. External installers/registrars are always advised, never run by AGS.".to_string(),
    }
}
