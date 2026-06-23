//! AGS skill governance — skill-body governance face (scan / check /
//! inventory / upstream / propose).
//!
//! This crate owns **skill-body governance**: it reads
//! governance/skill-adoption-log.yaml, governance/skill-ignore-list.yaml,
//! manifests/suite.yaml, and manifests/skills-registry.yaml to report skill
//! status, on-disk inventory, and upstream-comparison proposals. The
//! lifecycle scan/check/inventory/upstream paths are read-only; the
//! management console ([`console`]) additionally exposes a
//! confirmation-protected apply path that writes **only AGS-owned per-host
//! thin-index entries** through transactional replace and never runs external installers.
//!
//! Cross-Agent host visibility (which capability is visible to which host)
//! is exposed through [`console::verify_host`] / [`console::build_inventory`]
//! and is the seam slated to move under the `ags capability` command layer.
//!
//! ## Operations
//!
//! - `scan`: Discover skills from suite manifest, classify by status
//!   (required/optional/personal), report available/missing/disabled/degraded.
//! - `check`: Validate governance YAML files for schema compliance and
//!   cross-referenced consistency (adoption log ↔ manifest ↔ ignore list).
//! - `inventory`: Read-only on-disk scan of `SKILL.md` front-matter.
//! - `upstream`: Read-only upstream-comparison proposal skeleton from
//!   manifests/skills-registry.yaml. No network crawl is performed — it
//!   reports which skills watch which upstream and defers real crawl/diff.
//! - `propose`: Dry-run proposal — show what WOULD change if a skill were
//!   adopted/enabled/disabled. No files are modified.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Third-party skill & MCP management console (unified inventory, host
/// visibility, confirmation-protected proposal/apply).
pub mod console;
pub mod recommendations;

// ── Public types ──────────────────────────────────────────────────────────

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

// ── YAML schema types ─────────────────────────────────────────────────────

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

fn yaml_field(value: &serde_yaml::Value, field: &str) -> Option<String> {
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

    // Consistency: all adopted suite entries should have adoption log refs.
    // Optional public recommendations are metadata only and may intentionally
    // have no adoption log entry until a human confirms adoption.
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_yaml::from_str::<SuiteManifest>(&content) {
            if let Some(suite) = manifest.suite {
                let mut adopted_manifest_skill_names: Vec<String> = Vec::new();

                if let Some(required) = suite.required {
                    for entry in &required {
                        if let Some(ref name) = entry.name {
                            adopted_manifest_skill_names.push(name.clone());
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

                if let Some(personal) = suite.personal {
                    if let Some(personal_map) = personal.as_mapping() {
                        for (key, value) in personal_map {
                            if let Some(name) = key.as_str() {
                                adopted_manifest_skill_names.push(name.to_string());
                                if !value
                                    .as_mapping()
                                    .is_some_and(|m| m.contains_key("entry_ref"))
                                {
                                    issues.push(SkillIssue {
                                        severity: "warn".to_string(),
                                        category: "missing_entry_ref".to_string(),
                                        detail: format!(
                                            "Personal skill '{}' has no entry_ref in manifest",
                                            name
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }

                // Cross-reference: adoption log should contain adopted manifest
                // skills. Optional recommendations are excluded.
                if let Ok(adoption_content) = std::fs::read_to_string(&adoption_path) {
                    if let Ok(adoption) = serde_yaml::from_str::<AdoptionLog>(&adoption_content) {
                        if let Some(entries) = adoption.entries {
                            let adopted_names: Vec<&str> = entries
                                .iter()
                                .filter_map(|e| e.get("skill_name").and_then(|v| v.as_str()))
                                .collect();

                            let missing_from_adoption: Vec<&String> = adopted_manifest_skill_names
                                .iter()
                                .filter(|n| !adopted_names.contains(&n.as_str()))
                                .collect();

                            consistency_checks.push(ConsistencyCheck {
                                name: "manifest-to-adoption-log".to_string(),
                                passed: missing_from_adoption.is_empty(),
                                detail: if missing_from_adoption.is_empty() {
                                    "All adopted manifest skills have adoption log entries"
                                        .to_string()
                                } else {
                                    format!(
                                        "{} adopted manifest skill(s) missing from adoption log: {}",
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

fn extract_schema_version(yaml_content: &str) -> Option<String> {
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
        note: "DRY-RUN ONLY — this evaluate-only proposal path always returns dry_run and never modifies files. Real AGS-owned thin-index writes happen elsewhere via the console module (skill propose --apply / capability sync --apply) using transactional replace with receipt; dedupe quarantines still use governance backups. External installers/registrars are always advised, never run by AGS. Human confirmation + explicit task-card authorization required before any apply.".to_string(),
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// Render scan result as human-readable text.
pub fn render_scan_text(result: &SkillScanResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Scan Report".to_string());
    lines.push("================================".to_string());
    lines.push(format!("Schema:       {}", result.schema_version));
    lines.push(format!("Suite:        {}", result.suite_name));
    lines.push(format!("Version:      {}", result.suite_version));
    lines.push(String::new());

    lines.push("─ Summary ─".to_string());
    lines.push(format!(
        "  Total:     {} (available: {}, optional: {}, personal: {}, missing: {}, disabled: {}, degraded: {})",
        result.summary.total,
        result.summary.available,
        result.summary.optional,
        result.summary.personal,
        result.summary.missing,
        result.summary.disabled,
        result.summary.degraded,
    ));
    lines.push(String::new());

    if result.skills.is_empty() {
        lines.push("No skills found in suite manifest.".to_string());
        lines.push("(This is expected for a Phase 1 suite with an empty manifest.)".to_string());
    } else {
        lines.push("─ Skills ─".to_string());
        for skill in &result.skills {
            let status_icon = match skill.status {
                SkillStatus::Available => "[AVAILABLE]",
                SkillStatus::Optional => "[OPTIONAL]",
                SkillStatus::Personal => "[PERSONAL]",
                SkillStatus::Missing => "[MISSING]",
                SkillStatus::Disabled => "[DISABLED]",
                SkillStatus::Degraded => "[DEGRADED]",
            };
            lines.push(format!(
                "  {} {} (profile: {})",
                status_icon, skill.name, skill.profile
            ));
            if let Some(ref version) = skill.version {
                lines.push(format!("    version: {}", version));
            }
            if let Some(ref source) = skill.source {
                lines.push(format!("    source:  {}", source));
            }
            for warning in &skill.warnings {
                lines.push(format!("    ⚠ {}", warning));
            }
        }
    }

    lines.join("\n")
}

/// Render scan result as JSON.
pub fn render_scan_json(result: &SkillScanResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render check result as human-readable text.
pub fn render_check_text(result: &SkillCheckResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Check Report".to_string());
    lines.push("=================================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Passed:  {}", result.passed));
    lines.push(String::new());

    // Governance files
    lines.push("─ Governance Files ─".to_string());
    let render_file = |label: &str, fs: &FileStatus| {
        let status = if fs.present && fs.parseable {
            "OK"
        } else if fs.present {
            "PARSE_ERROR"
        } else {
            "MISSING"
        };
        format!(
            "  {}: {} (entries: {}, schema: {:?})",
            label,
            status,
            fs.entry_count,
            fs.schema_version.as_deref().unwrap_or("?")
        )
    };
    lines.push(render_file(
        "skill-adoption-log",
        &result.governance_files.skill_adoption_log,
    ));
    lines.push(render_file(
        "skill-ignore-list",
        &result.governance_files.skill_ignore_list,
    ));
    lines.push(render_file(
        "suite-manifest",
        &result.governance_files.suite_manifest,
    ));
    lines.push(String::new());

    // Consistency checks
    lines.push("─ Consistency Checks ─".to_string());
    for check in &result.consistency_checks {
        let icon = if check.passed { "✓" } else { "✗" };
        lines.push(format!("  {} {}: {}", icon, check.name, check.detail));
    }
    lines.push(String::new());

    // Issues
    if result.issues.is_empty() {
        lines.push("─ Issues ─".to_string());
        lines.push("  None".to_string());
    } else {
        lines.push("─ Issues ─".to_string());
        for issue in &result.issues {
            lines.push(format!(
                "  [{}] [{}] {}",
                issue.severity.to_uppercase(),
                issue.category,
                issue.detail
            ));
        }
    }

    lines.join("\n")
}

/// Render check result as JSON.
pub fn render_check_json(result: &SkillCheckResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render proposal result as human-readable text.
pub fn render_proposal_text(result: &SkillProposalResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Proposal".to_string());
    lines.push("===========================".to_string());
    lines.push(format!("Schema:     {}", result.schema_version));
    lines.push(format!("Action:     {}", result.proposal_type));
    lines.push(format!("Dry run:    {}", result.dry_run));
    lines.push(String::new());

    lines.push("─ Target Skills ─".to_string());
    if result.target_skills.is_empty() {
        lines.push("  None".to_string());
    } else {
        for skill in &result.target_skills {
            lines.push(format!("  - {}", skill));
        }
    }
    lines.push(String::new());

    lines.push("─ Proposed Changes ─".to_string());
    if result.proposed_changes.is_empty() {
        lines.push("  No changes proposed.".to_string());
    } else {
        for change in &result.proposed_changes {
            lines.push(format!("  + {}", change));
        }
    }
    lines.push(String::new());

    if !result.blocked_reasons.is_empty() {
        lines.push("─ Blocked ─".to_string());
        for reason in &result.blocked_reasons {
            lines.push(format!("  ✗ {}", reason));
        }
        lines.push(String::new());
    }

    lines.push(format!("NOTE: {}", result.note));

    lines.join("\n")
}

/// Render proposal result as JSON.
pub fn render_proposal_json(result: &SkillProposalResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Skill asset inventory ───────────────────────────────────────────────────
//
// On-disk inventory of skill assets under `global-skills/` and `skill-packs/`.
// Distinct from `scan_skills` (which reads the suite *manifest*): the inventory
// walks the actual skill directories and reads each `SKILL.md` front-matter.
// It is strictly read-only over `SKILL.md` files — it never reads `.env`,
// tokens, credentials, caches, or runtime state.

/// A single skill discovered on disk.
#[derive(Debug, Clone, Serialize)]
pub struct SkillInventoryEntry {
    pub name: String,
    pub path: String,
    /// "global" | "optional" | "personal"
    pub source_category: String,
    pub has_skill_md: bool,
    pub description_present: bool,
    pub risk_hints: Vec<String>,
    pub public_allowed_guess: bool,
    /// SKILL.md last-modified marker (`epoch:<secs>`), when available.
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInventorySummary {
    pub total: usize,
    pub global: usize,
    pub optional: usize,
    pub personal: usize,
    pub with_skill_md: usize,
    pub with_description: usize,
    pub public_allowed: usize,
    pub flagged_risk: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInventoryResult {
    pub schema_version: String,
    pub roots_scanned: Vec<String>,
    pub entries: Vec<SkillInventoryEntry>,
    pub summary: SkillInventorySummary,
}

/// High-signal substrings hinting a skill may carry elevated risk or be
/// unsuitable for public distribution. Scanned only against SKILL.md text.
const RISK_HINT_KEYWORDS: &[&str] = &[
    "secret",
    "token",
    "credential",
    "password",
    "api key",
    "api_key",
    "bearer",
    "node-local secret",
    "rm -rf",
    "sudo ",
    "git reset --hard",
    "--force",
    "force push",
    "drop table",
];

/// Scan `global-skills/` and `skill-packs/{optional,personal}/` for skill
/// assets, reading each `SKILL.md` front-matter. Read-only over SKILL.md.
pub fn scan_skill_inventory(root: &Path) -> SkillInventoryResult {
    let roots: [(&str, std::path::PathBuf); 3] = [
        ("global", root.join("global-skills")),
        ("optional", root.join("skill-packs/optional")),
        ("personal", root.join("skill-packs/personal")),
    ];

    let mut entries: Vec<SkillInventoryEntry> = Vec::new();
    let mut roots_scanned: Vec<String> = Vec::new();

    for (category, dir) in &roots {
        if !dir.is_dir() {
            continue;
        }
        roots_scanned.push(dir.display().to_string());
        let mut skill_dirs: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(_) => continue,
        };
        skill_dirs.sort();
        for skill_dir in skill_dirs {
            entries.push(inventory_entry(category, &skill_dir));
        }
    }

    let summary = SkillInventorySummary {
        total: entries.len(),
        global: entries
            .iter()
            .filter(|e| e.source_category == "global")
            .count(),
        optional: entries
            .iter()
            .filter(|e| e.source_category == "optional")
            .count(),
        personal: entries
            .iter()
            .filter(|e| e.source_category == "personal")
            .count(),
        with_skill_md: entries.iter().filter(|e| e.has_skill_md).count(),
        with_description: entries.iter().filter(|e| e.description_present).count(),
        public_allowed: entries.iter().filter(|e| e.public_allowed_guess).count(),
        flagged_risk: entries.iter().filter(|e| !e.risk_hints.is_empty()).count(),
    };

    SkillInventoryResult {
        schema_version: SCHEMA_VERSION.to_string(),
        roots_scanned,
        entries,
        summary,
    }
}

fn inventory_entry(category: &str, skill_dir: &Path) -> SkillInventoryEntry {
    let dir_name = skill_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    let skill_md = skill_dir.join("SKILL.md");
    let has_skill_md = skill_md.is_file();

    let mut name = dir_name;
    let mut description_present = false;
    let mut risk_hints: Vec<String> = Vec::new();
    let mut last_seen: Option<String> = None;

    if has_skill_md {
        if let Ok(meta) = std::fs::metadata(&skill_md) {
            if let Ok(modified) = meta.modified() {
                last_seen = format_system_time(modified);
            }
        }
        // Read ONLY SKILL.md — never other files in the skill directory.
        if let Ok(text) = std::fs::read_to_string(&skill_md) {
            let (fm_name, fm_desc) = parse_front_matter(&text);
            if let Some(n) = fm_name {
                if !n.trim().is_empty() {
                    name = n.trim().to_string();
                }
            }
            description_present = fm_desc.map(|d| !d.trim().is_empty()).unwrap_or(false);
            let lower = text.to_lowercase();
            for kw in RISK_HINT_KEYWORDS {
                if lower.contains(kw) {
                    risk_hints.push((*kw).trim().to_string());
                }
            }
        }
    }

    // public_allowed guess: personal skills are never public; others are
    // public-safe candidates only when no risk hints were detected.
    let public_allowed_guess = category != "personal" && risk_hints.is_empty();

    SkillInventoryEntry {
        name,
        path: skill_dir.display().to_string(),
        source_category: category.to_string(),
        has_skill_md,
        description_present,
        risk_hints,
        public_allowed_guess,
        last_seen,
    }
}

/// Extract `name:` and `description:` from a leading `--- ... ---` YAML
/// front-matter block. Returns (name, description); robust to absent fields.
pub(crate) fn parse_front_matter(text: &str) -> (Option<String>, Option<String>) {
    let trimmed = text.trim_start();
    let Some(after_open) = trimmed.strip_prefix("---") else {
        return (None, None);
    };
    let Some(end) = after_open.find("\n---") else {
        return (None, None);
    };
    let yaml = &after_open[..end];
    match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
        Ok(value) => {
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let description = value
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (name, description)
        }
        Err(_) => (None, None),
    }
}

/// Format a SystemTime as an epoch-seconds marker (avoids extra date deps).
fn format_system_time(t: std::time::SystemTime) -> Option<String> {
    t.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| format!("epoch:{}", d.as_secs()))
}

/// Render inventory as human-readable text.
pub fn render_inventory_text(result: &SkillInventoryResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Asset Inventory".to_string());
    lines.push("=====================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Roots:   {}", result.roots_scanned.join(", ")));
    lines.push(String::new());
    lines.push(format!(
        "Summary: total {} (global {}, optional {}, personal {}); with SKILL.md {}, with description {}, public-allowed guess {}, risk-flagged {}",
        result.summary.total,
        result.summary.global,
        result.summary.optional,
        result.summary.personal,
        result.summary.with_skill_md,
        result.summary.with_description,
        result.summary.public_allowed,
        result.summary.flagged_risk,
    ));
    lines.push(String::new());
    for e in &result.entries {
        let md = if e.has_skill_md { "md" } else { "NO-md" };
        let desc = if e.description_present {
            "desc"
        } else {
            "no-desc"
        };
        let public = if e.public_allowed_guess {
            "public?"
        } else {
            "private"
        };
        lines.push(format!(
            "  [{}] {} ({}, {}, {})",
            e.source_category, e.name, md, desc, public
        ));
        if !e.risk_hints.is_empty() {
            lines.push(format!("      risk hints: {}", e.risk_hints.join(", ")));
        }
    }
    lines.join("\n")
}

/// Render inventory as JSON.
pub fn render_inventory_json(result: &SkillInventoryResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render inventory as a Markdown report (for governance/skills-inventory.md).
pub fn render_inventory_markdown(result: &SkillInventoryResult) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push("# Skill Asset Inventory".to_string());
    out.push(String::new());
    out.push(format!(
        "_Generated by `ags skill inventory --write`. Schema `{}`. Read-only scan of `SKILL.md` files; no secrets, tokens, or runtime files are read._",
        result.schema_version
    ));
    out.push(String::new());
    out.push("## Summary".to_string());
    out.push(String::new());
    out.push("| Metric | Count |".to_string());
    out.push("|---|---|".to_string());
    out.push(format!("| Total skills | {} |", result.summary.total));
    out.push(format!("| global-skills | {} |", result.summary.global));
    out.push(format!(
        "| skill-packs/optional | {} |",
        result.summary.optional
    ));
    out.push(format!(
        "| skill-packs/personal | {} |",
        result.summary.personal
    ));
    out.push(format!(
        "| With SKILL.md | {} |",
        result.summary.with_skill_md
    ));
    out.push(format!(
        "| With description | {} |",
        result.summary.with_description
    ));
    out.push(format!(
        "| Public-allowed (guess) | {} |",
        result.summary.public_allowed
    ));
    out.push(format!(
        "| Risk-flagged | {} |",
        result.summary.flagged_risk
    ));
    out.push(String::new());
    out.push("## Skills".to_string());
    out.push(String::new());
    out.push(
        "| Category | Name | SKILL.md | Description | Public-allowed (guess) | Risk hints |"
            .to_string(),
    );
    out.push("|---|---|---|---|---|---|".to_string());
    for e in &result.entries {
        let risk = if e.risk_hints.is_empty() {
            "—".to_string()
        } else {
            e.risk_hints.join(", ")
        };
        out.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            e.source_category,
            e.name,
            if e.has_skill_md { "yes" } else { "no" },
            if e.description_present { "yes" } else { "no" },
            if e.public_allowed_guess { "yes" } else { "no" },
            risk,
        ));
    }
    out.push(String::new());
    out.join("\n")
}

// ── Upstream update proposal (stub) ─────────────────────────────────────────
//
// Reads `manifests/skills-registry.yaml` and reports which suite skills watch
// which upstream comparison source, plus declared candidate skills. This is a
// PLANNING SKELETON only: it performs NO network crawl, clone, or fetch, and
// proposes no concrete diff. Real `crawl_then_diff_proposal` lives in a future
// task. Local suite files always remain the canonical source of truth.

#[derive(Debug, Clone, Deserialize)]
struct SkillsRegistryDoc {
    registry: Option<RegistrySection>,
    skills: Option<Vec<RegistrySkill>>,
    candidate_skills: Option<Vec<RegistryCandidate>>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySection {
    #[allow(dead_code)]
    version: Option<serde_yaml::Value>,
    update_policy: Option<String>,
    upstreams: Option<serde_yaml::Mapping>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySkill {
    name: Option<String>,
    profile: Option<String>,
    source: Option<RegistrySource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryCandidate {
    name: Option<String>,
    adoption_priority: Option<String>,
    adoption_mode: Option<String>,
    source: Option<RegistrySource>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistrySource {
    #[serde(rename = "type")]
    source_type: Option<String>,
    upstream: Option<String>,
    path: Option<String>,
    relationship: Option<String>,
    update_policy: Option<String>,
}

/// A declared upstream comparison source (read-only crawl seed).
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamSourceInfo {
    pub name: String,
    pub kind: Option<String>,
    pub url: Option<String>,
    pub web: Option<String>,
    pub reference: Option<String>,
    pub cli: Option<String>,
    pub crawl: bool,
}

/// A suite skill that tracks an upstream comparison source.
#[derive(Debug, Clone, Serialize)]
pub struct WatchedSkill {
    pub name: String,
    pub profile: Option<String>,
    pub source_type: Option<String>,
    pub upstream: Option<String>,
    pub upstream_path: Option<String>,
    pub relationship: Option<String>,
    pub update_policy: Option<String>,
}

/// A declared candidate skill (evaluate-only; not yet adopted).
#[derive(Debug, Clone, Serialize)]
pub struct CandidateSkillInfo {
    pub name: String,
    pub upstream: Option<String>,
    pub adoption_priority: Option<String>,
    pub adoption_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpstreamProposalSummary {
    pub upstreams: usize,
    pub watched_skills: usize,
    pub candidates: usize,
    /// Always `false` in this stub — no crawl/fetch is performed.
    pub crawl_performed: bool,
}

/// Result of [`upstream_proposal`].
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamProposalResult {
    pub schema_version: String,
    pub registry_present: bool,
    pub registry_parseable: bool,
    pub registry_path: String,
    pub update_policy: Option<String>,
    pub upstreams: Vec<UpstreamSourceInfo>,
    pub watched_skills: Vec<WatchedSkill>,
    pub candidates: Vec<CandidateSkillInfo>,
    pub summary: UpstreamProposalSummary,
    pub note: String,
}

/// Build a read-only upstream-comparison proposal skeleton from
/// `manifests/skills-registry.yaml`. Performs NO network access.
pub fn upstream_proposal(root: &Path) -> UpstreamProposalResult {
    let registry_path = root.join("manifests/skills-registry.yaml");
    let rel_path = "manifests/skills-registry.yaml".to_string();

    let mut result = UpstreamProposalResult {
        schema_version: SCHEMA_VERSION.to_string(),
        registry_present: registry_path.exists(),
        registry_parseable: false,
        registry_path: rel_path,
        update_policy: None,
        upstreams: Vec::new(),
        watched_skills: Vec::new(),
        candidates: Vec::new(),
        summary: UpstreamProposalSummary {
            upstreams: 0,
            watched_skills: 0,
            candidates: 0,
            crawl_performed: false,
        },
        note: UPSTREAM_STUB_NOTE.to_string(),
    };

    let Ok(content) = std::fs::read_to_string(&registry_path) else {
        return result;
    };
    let Ok(doc) = serde_yaml::from_str::<SkillsRegistryDoc>(&content) else {
        return result;
    };
    result.registry_parseable = true;

    if let Some(registry) = doc.registry {
        result.update_policy = registry.update_policy;
        if let Some(upstreams) = registry.upstreams {
            for (key, value) in &upstreams {
                let Some(name) = key.as_str() else { continue };
                result.upstreams.push(UpstreamSourceInfo {
                    name: name.to_string(),
                    kind: yaml_field(value, "type"),
                    url: yaml_field(value, "url"),
                    web: yaml_field(value, "web"),
                    reference: yaml_field(value, "ref"),
                    cli: yaml_field(value, "cli"),
                    crawl: value
                        .get("crawl")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                });
            }
        }
    }
    result.upstreams.sort_by(|a, b| a.name.cmp(&b.name));

    // A skill is "watched" when its source declares an upstream comparison
    // source (i.e. it is not a purely local canonical skill).
    if let Some(skills) = doc.skills {
        for skill in skills {
            let Some(name) = skill.name else { continue };
            if let Some(source) = skill.source {
                if source.upstream.is_some() {
                    result.watched_skills.push(WatchedSkill {
                        name,
                        profile: skill.profile,
                        source_type: source.source_type,
                        upstream: source.upstream,
                        upstream_path: source.path,
                        relationship: source.relationship,
                        update_policy: source.update_policy,
                    });
                }
            }
        }
    }

    if let Some(candidates) = doc.candidate_skills {
        for candidate in candidates {
            let Some(name) = candidate.name else { continue };
            result.candidates.push(CandidateSkillInfo {
                name,
                upstream: candidate.source.and_then(|s| s.upstream),
                adoption_priority: candidate.adoption_priority,
                adoption_mode: candidate.adoption_mode,
            });
        }
    }

    result.summary = UpstreamProposalSummary {
        upstreams: result.upstreams.len(),
        watched_skills: result.watched_skills.len(),
        candidates: result.candidates.len(),
        crawl_performed: false,
    };
    result
}

const UPSTREAM_STUB_NOTE: &str = "STUB — no network crawl, clone, or fetch was performed and no concrete diff is proposed. This lists the upstream comparison sources and the suite skills that watch them, per manifests/skills-registry.yaml. Local suite files remain canonical; real crawl_then_diff_proposal is deferred to a future task. AGS never runs `npx skills` or auto-installs from upstream.";

/// Render upstream proposal as human-readable text.
pub fn render_upstream_text(result: &UpstreamProposalResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Governance — Upstream Update Proposal (stub)".to_string());
    lines.push("=================================================".to_string());
    lines.push(format!("Schema:        {}", result.schema_version));
    lines.push(format!("Registry:      {}", result.registry_path));
    let status = if !result.registry_present {
        "MISSING"
    } else if !result.registry_parseable {
        "PARSE_ERROR"
    } else {
        "OK"
    };
    lines.push(format!("Status:        {status}"));
    lines.push(format!(
        "Update policy: {}",
        result.update_policy.as_deref().unwrap_or("?")
    ));
    lines.push(format!(
        "Summary:       upstreams {}, watched skills {}, candidates {}, crawl performed {}",
        result.summary.upstreams,
        result.summary.watched_skills,
        result.summary.candidates,
        result.summary.crawl_performed,
    ));
    lines.push(String::new());

    lines.push("─ Upstream Sources ─".to_string());
    if result.upstreams.is_empty() {
        lines.push("  None declared.".to_string());
    } else {
        for u in &result.upstreams {
            let crawl = if u.crawl { "crawl" } else { "no-crawl" };
            lines.push(format!(
                "  - {} ({}, {})",
                u.name,
                u.kind.as_deref().unwrap_or("?"),
                crawl
            ));
            if let Some(ref web) = u.web {
                lines.push(format!("      web: {web}"));
            }
        }
    }
    lines.push(String::new());

    lines.push("─ Watched Skills ─".to_string());
    if result.watched_skills.is_empty() {
        lines.push("  None.".to_string());
    } else {
        for s in &result.watched_skills {
            lines.push(format!(
                "  - {} → upstream {} (policy: {})",
                s.name,
                s.upstream.as_deref().unwrap_or("?"),
                s.update_policy.as_deref().unwrap_or("?"),
            ));
        }
    }
    lines.push(String::new());

    lines.push("─ Candidate Skills ─".to_string());
    if result.candidates.is_empty() {
        lines.push("  None.".to_string());
    } else {
        for c in &result.candidates {
            lines.push(format!(
                "  - {} (upstream: {}, priority: {}, mode: {})",
                c.name,
                c.upstream.as_deref().unwrap_or("?"),
                c.adoption_priority.as_deref().unwrap_or("?"),
                c.adoption_mode.as_deref().unwrap_or("?"),
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

/// Render upstream proposal as JSON.
pub fn render_upstream_json(result: &UpstreamProposalResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        // Tests run from crates/skill-governance/, so ../.. reaches the repo root
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest_dir.join("../..")
    }

    #[test]
    fn test_schema_version() {
        assert_eq!(SCHEMA_VERSION, "2.0-skill");
    }

    #[test]
    fn test_scan_migrated_manifest() {
        // Scan the public suite manifest in the repo.
        let root = repo_root();
        let result = scan_skills(&root);
        assert_eq!(result.schema_version, SCHEMA_VERSION);
        // The public edition exposes installable skill entries as optional
        // metadata only; private/personal skill profiles are not distributed.
        // auto-brainstorm/auto-debug/auto-verify were retired in 2.7 (11 → 8),
        // then the caveman-commit/caveman-review local aliases were removed in
        // the upstream-name alignment (8 → 6).
        assert_eq!(result.summary.available, 0);
        assert_eq!(result.summary.optional, 6);
        assert_eq!(result.summary.personal, 0);
        assert_eq!(result.summary.disabled, 0);
    }

    #[test]
    fn test_scan_public_manifest_excludes_personal_profile() {
        let root = repo_root();
        let result = scan_skills(&root);
        assert!(
            result
                .skills
                .iter()
                .all(|skill| skill.status != SkillStatus::Personal),
            "public suite manifest must not expose personal skill metadata"
        );
    }

    #[test]
    fn test_scan_finds_suite_name() {
        let root = repo_root();
        let result = scan_skills(&root);
        assert!(!result.suite_name.is_empty());
    }

    #[test]
    fn test_check_governance_files() {
        let root = repo_root();
        let result = check_skills(&root);
        assert_eq!(result.schema_version, SCHEMA_VERSION);
        assert!(result.governance_files.suite_manifest.present);
        assert!(result.governance_files.suite_manifest.parseable);
    }

    #[test]
    fn test_propose_unknown_skill() {
        let root = repo_root();
        let result = propose_skills(&root, "adopt", "nonexistent-skill");
        assert_eq!(result.proposal_type, "adopt");
        assert!(result.dry_run);
        assert!(!result.proposed_changes.is_empty());
    }

    #[test]
    fn test_propose_unknown_action_is_blocked() {
        let root = repo_root();
        let result = propose_skills(&root, "unknown-action", "test-skill");
        assert!(result.target_skills.is_empty());
        assert!(!result.blocked_reasons.is_empty());
    }

    #[test]
    fn test_inventory_on_fixture() {
        // Temporary skill tree so the test is independent of the repo's actual
        // skill directories (the public edition ships none).
        let base = std::env::temp_dir().join(format!("ags-skill-inv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let gs = base.join("global-skills/demo-skill");
        std::fs::create_dir_all(&gs).unwrap();
        std::fs::write(
            gs.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: A demo skill.\n---\nbody\n",
        )
        .unwrap();
        let personal = base.join("skill-packs/personal/secret-skill");
        std::fs::create_dir_all(&personal).unwrap();
        std::fs::write(
            personal.join("SKILL.md"),
            "---\nname: secret-skill\ndescription: manages an API token secret.\n---\n",
        )
        .unwrap();

        let result = scan_skill_inventory(&base);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(result.summary.total, 2);
        assert_eq!(result.summary.global, 1);
        assert_eq!(result.summary.personal, 1);

        let demo = result
            .entries
            .iter()
            .find(|e| e.name == "demo-skill")
            .expect("demo-skill discovered via front-matter name");
        assert!(demo.has_skill_md && demo.description_present);
        assert!(demo.public_allowed_guess); // global, no risk hints

        let secret = result
            .entries
            .iter()
            .find(|e| e.name == "secret-skill")
            .expect("secret-skill discovered");
        assert!(!secret.public_allowed_guess); // personal + risk hints
        assert!(!secret.risk_hints.is_empty());
    }

    #[test]
    fn test_inventory_empty_tree_renders() {
        // No skill dirs (mirrors the public edition) → total 0, still renders.
        let base = std::env::temp_dir().join(format!("ags-skill-inv-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let result = scan_skill_inventory(&base);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(result.summary.total, 0);
        assert!(render_inventory_text(&result).contains("Skill Asset Inventory"));
        assert!(render_inventory_markdown(&result).contains("# Skill Asset Inventory"));
        assert!(render_inventory_json(&result).contains("\"total\": 0"));
    }

    #[test]
    fn test_render_scan_text() {
        let root = repo_root();
        let result = scan_skills(&root);
        let text = render_scan_text(&result);
        assert!(text.contains("Skill Governance"));
        assert!(text.contains("Suite:"));
    }

    #[test]
    fn test_render_scan_json() {
        let root = repo_root();
        let result = scan_skills(&root);
        let json = render_scan_json(&result);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_render_check_text() {
        let root = repo_root();
        let result = check_skills(&root);
        let text = render_check_text(&result);
        assert!(text.contains("Check Report"));
    }

    #[test]
    fn test_render_check_json() {
        let root = repo_root();
        let result = check_skills(&root);
        let json = render_check_json(&result);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_render_proposal_text() {
        let root = repo_root();
        let result = propose_skills(&root, "adopt", "test-skill");
        let text = render_proposal_text(&result);
        assert!(text.contains("Proposal"));
        assert!(text.contains("DRY-RUN"));
    }

    #[test]
    fn test_render_proposal_json() {
        let root = repo_root();
        let result = propose_skills(&root, "adopt", "test-skill");
        let json = render_proposal_json(&result);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok());
        assert_eq!(parsed.unwrap()["dry_run"], true);
    }

    #[test]
    fn test_extract_schema_version() {
        assert_eq!(
            extract_schema_version("schema_version: \"1.0\"\nentries: []"),
            Some("1.0".to_string())
        );
        assert_eq!(
            extract_schema_version("# comment\nschema_version: \"2.0\"\n"),
            Some("2.0".to_string())
        );
        assert_eq!(extract_schema_version("entries: []"), None);
    }

    #[test]
    fn test_upstream_proposal_on_repo_registry() {
        let root = repo_root();
        let result = upstream_proposal(&root);
        assert!(result.registry_present, "repo ships skills-registry.yaml");
        assert!(result.registry_parseable);
        assert_eq!(
            result.update_policy.as_deref(),
            Some("read_only_crawl_then_diff_proposal")
        );
        // Declared upstream comparison sources.
        assert!(result
            .upstreams
            .iter()
            .any(|u| u.name == "mattpocock_skills" && u.crawl));
        assert!(result.upstreams.iter().any(|u| u.name == "graphify"));
        // Skills that track an upstream are surfaced; purely-local ones are not.
        assert!(result
            .watched_skills
            .iter()
            .any(|s| s.name == "diagnosing-bugs"));
        assert!(!result
            .watched_skills
            .iter()
            .any(|s| s.name == "prompt-maker"));
        // Candidates are declared but not adopted.
        assert!(result
            .candidates
            .iter()
            .any(|c| c.name == "git-guardrails-claude-code"));
        // This is a stub — no crawl is ever performed.
        assert!(!result.summary.crawl_performed);
        assert_eq!(result.summary.watched_skills, result.watched_skills.len());
    }

    #[test]
    fn test_upstream_proposal_missing_registry() {
        let base =
            std::env::temp_dir().join(format!("ags-upstream-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let result = upstream_proposal(&base);
        let _ = std::fs::remove_dir_all(&base);
        assert!(!result.registry_present);
        assert!(!result.registry_parseable);
        assert!(result.upstreams.is_empty());
        assert!(result.watched_skills.is_empty());
        assert!(!result.summary.crawl_performed);
    }

    #[test]
    fn test_render_upstream_text_and_json() {
        let root = repo_root();
        let result = upstream_proposal(&root);
        let text = render_upstream_text(&result);
        assert!(text.contains("Upstream Update Proposal"));
        assert!(text.contains("STUB"));
        let json = render_upstream_json(&result);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["summary"]["crawl_performed"], false);
    }
}
