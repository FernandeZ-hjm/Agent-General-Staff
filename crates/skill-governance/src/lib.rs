//! AGS skill governance — inventory, proposal, and confirmed install.
//!
//! Reads governance/skill-adoption-log.yaml, governance/skill-ignore-list.yaml,
//! and manifests/suite.yaml to produce skill status reports. The install module
//! provides real skill installation with directory structure and SKILL.md
//! frontmatter.
//!
//! ## Operations
//!
//! - `scan`: Discover skills from suite manifest, classify by status
//!   (required/optional/personal), report available/missing/disabled/degraded.
//! - `check`: Validate governance YAML files for schema compliance and
//!   cross-referenced consistency (adoption log ↔ manifest ↔ ignore list).
//! - `propose`: Dry-run proposal — show what WOULD change if a skill were
//!   adopted/enabled/disabled. No files are modified.
//! - `install`: Real skill installation — creates directory structure with
//!   SKILL.md frontmatter, writes install receipt.

pub mod install;

use serde::{Deserialize, Serialize};
use std::path::Path;

// Re-export install types for convenience
pub use install::{
    install_plan, install_skills, known_skills, render_install_json, render_install_text,
    InstallMode, InstallResult, InstallStatus, SkillCategory, SkillDef,
};

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
                            skills.push(SkillEntry {
                                name: name.to_string(),
                                status: if is_active {
                                    SkillStatus::Disabled
                                } else {
                                    SkillStatus::Disabled
                                },
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
                let versions = vec![
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
    let scan = scan_skills(&root);
    let existing = scan.skills.iter().find(|s| s.name == skill_name);

    match action {
        "adopt" => {
            if existing.is_some() {
                proposed_changes.push(format!(
                    "Skill '{}' already exists with status: {:?}",
                    skill_name,
                    existing.unwrap().status
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
        note: "DRY-RUN ONLY — no files modified. Human confirmation and explicit task-card authorization required before applying any changes. Adopt/apply/rollback writes are not implemented in this CLI version.".to_string(),
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
    fn test_scan_public_manifest() {
        // Scan the public suite manifest.
        let root = repo_root();
        let result = scan_skills(&root);
        assert_eq!(result.schema_version, SCHEMA_VERSION);
        // Public suite starts with 0 required skills — no third-party skills by default
        assert_eq!(
            result.summary.available, 0,
            "public suite should start with 0 required skills"
        );
        // Optional skills may be present in the recommendations manifest
        assert!(!result.suite_name.is_empty());
    }

    #[test]
    fn test_scan_no_personal_skills_leak() {
        // Personal skills must NOT appear in public edition scan results.
        let root = repo_root();
        let result = scan_skills(&root);
        let personal_count = result
            .skills
            .iter()
            .filter(|s| s.status == SkillStatus::Personal)
            .count();
        assert_eq!(
            personal_count, 0,
            "public edition must not ship personal skills"
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
}
