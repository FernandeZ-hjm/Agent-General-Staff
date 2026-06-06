//! Legal-difference allowlist for public/core-only and other target types.
//!
//! An allowlist declares which differences between source and target are legally
//! permissible (not drift). This supports the public/core-only use case where
//! internal paths, collaboration notes, and machine-specific details are
//! intentionally removed from the public distribution.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::types::{DriftKind, ProjectKind};

// ── Allowlist data model ───────────────────────────────────────────────

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Allowlist {
    /// Human-readable description of this allowlist.
    #[serde(default)]
    pub description: String,

    /// Per-file allowlist entries.
    #[serde(default)]
    pub files: Vec<FileAllowlistEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileAllowlistEntry {
    /// Relative file path, e.g. `protocol/runtime-adapters.md`.
    pub file: String,

    /// Files that may legally be absent from the target.
    #[serde(default)]
    pub allowed_missing_files: Vec<String>,

    /// Section paths (as ` > `-separated strings) that may legally be missing.
    #[serde(default)]
    pub allowed_missing_sections: Vec<String>,

    /// Section paths where content changes are legal.
    #[serde(default)]
    pub allowed_content_drift_sections: Vec<String>,

    /// Substring patterns — lines containing any of these may be legally removed.
    #[serde(default)]
    pub removable_content_patterns: Vec<String>,

    /// Regex patterns (as strings) — lines matching these may be legally removed.
    #[serde(default)]
    pub removable_content_regex: Vec<String>,
}

// ── Built-in allowlists ────────────────────────────────────────────────

/// Return the built-in public/core-only allowlist.
///
/// This encodes the known legal differences between the private suite and
/// a public distribution:
/// - Internal collaboration files are absent
/// - Machine-specific paths are redacted
/// - Internal workflow sections are removed
pub fn default_public_allowlist() -> Allowlist {
    Allowlist {
        description: "Default allowlist for public/core-only distribution".into(),
        files: vec![
            FileAllowlistEntry {
                file: "AGENTS.md".to_string(),
                allowed_missing_files: vec!["AGENTS.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "CLAUDE.md".to_string(),
                allowed_missing_files: vec!["CLAUDE.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "WORKSPACE.md".to_string(),
                allowed_missing_files: vec!["WORKSPACE.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "AGENT_SUITE_PROTOCOL.md".to_string(),
                allowed_missing_files: vec!["AGENT_SUITE_PROTOCOL.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/runtime-adapters.md".to_string(),
                allowed_missing_files: vec![],
                allowed_missing_sections: vec![
                    // Internal Codex-specific execution notes
                    "Runtime Adapters > Default Profiles > Codex Direct Execution".to_string(),
                    "Runtime Adapters > Default Profiles > Claude Code Handoff".to_string(),
                    // Private Rust resolver implementation details. Public
                    // keeps only the safety invariants and runner-facing rule.
                    "Runtime Adapters > Execution-Policy Resolver".to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > CLI".to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Default semantics".to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Execution surface values"
                        .to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Key resolution rules"
                        .to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Relationship with validator and runner"
                        .to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Resolved policy JSON schema"
                        .to_string(),
                    "Runtime Adapters > Execution-Policy Resolver > Stop vs confirmation gate"
                        .to_string(),
                    "Runtime Adapters > Runner Auto Mode > Correct auto-mode flow".to_string(),
                    "Runtime Adapters > Runner Auto Mode > Defaults preserved".to_string(),
                    "Runtime Adapters > Runner Auto Mode > Example: resolved policy → runner flags"
                        .to_string(),
                    "Runtime Adapters > Runner Auto Mode > Why resolver-first".to_string(),
                ],
                allowed_content_drift_sections: vec![
                    "Runtime Adapters > Runner Auto Mode".to_string(),
                ],
                removable_content_patterns: vec![
                    "/Volumes/AI Project/".to_string(),
                    "machine-specific".to_string(),
                ],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/task-card-template.md".to_string(),
                allowed_missing_files: vec![],
                allowed_missing_sections: vec![
                    // Internal skill governance section
                    "Task Card Template > Skill Governance 治理任务补充".to_string(),
                ],
                allowed_content_drift_sections: vec![
                    "Task Card Template > Skill Governance 治理任务补充".to_string(),
                    "Task Card Template > 任务卡".to_string(),
                    "Task Card Template > 使用说明".to_string(),
                ],
                removable_content_patterns: vec![
                    "/Volumes/AI Project/".to_string(),
                    "$HOME/.agents".to_string(),
                ],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/agent-task-protocol.md".to_string(),
                allowed_missing_files: vec![],
                allowed_missing_sections: vec![
                    // Internal hook policy and skill governance
                    "Agent Task Protocol > Skill Governance 治理".to_string(),
                ],
                allowed_content_drift_sections: vec![
                    "Agent Task Protocol > Review Gate 规则".to_string(),
                    "Agent Task Protocol > Runtime Hook Policy".to_string(),
                    "Agent Task Protocol > Skill Governance 治理".to_string(),
                ],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/README.md".to_string(),
                allowed_missing_files: vec!["protocol/README.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/context-memory.md".to_string(),
                allowed_missing_files: vec!["protocol/context-memory.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/cursor-skill-index.md".to_string(),
                allowed_missing_files: vec!["protocol/cursor-skill-index.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
            FileAllowlistEntry {
                file: "protocol/project-profile.md".to_string(),
                allowed_missing_files: vec!["protocol/project-profile.md".to_string()],
                allowed_missing_sections: vec![],
                allowed_content_drift_sections: vec![],
                removable_content_patterns: vec![],
                removable_content_regex: vec![],
            },
        ],
    }
}

/// Return an empty allowlist (all differences are drift).
pub fn empty_allowlist() -> Allowlist {
    Allowlist {
        description: "Empty allowlist — all differences are treated as drift".into(),
        files: vec![],
    }
}

// ── Allowlist loading ──────────────────────────────────────────────────

/// Load an allowlist from a JSON file.
pub fn load_allowlist(path: &Path) -> Result<Allowlist, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("failed to read allowlist file {}: {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse allowlist JSON in {}: {e}", path.display()))
}

/// Get the default allowlist for a project kind.
pub fn default_for(kind: &ProjectKind) -> Allowlist {
    match kind {
        ProjectKind::PublicCoreOnly => default_public_allowlist(),
        _ => empty_allowlist(),
    }
}

// ── Allowlist matching logic ───────────────────────────────────────────

/// Check whether a drift is allowed (legal difference, not action-worthy).
pub fn is_allowed(
    allowlist: &Allowlist,
    file: &str,
    section_path: &[String],
    kind: &DriftKind,
) -> bool {
    for entry in &allowlist.files {
        if entry.file != file {
            continue;
        }

        match kind {
            DriftKind::FileMissingInTarget | DriftKind::FileMissingInSource => {
                if entry.allowed_missing_files.contains(&file.to_string()) {
                    return true;
                }
            }
            DriftKind::SectionMissing | DriftKind::ExtraSection => {
                let path_str = crate::types::format_section_path(section_path);
                if entry.allowed_missing_sections.contains(&path_str) {
                    return true;
                }
            }
            DriftKind::ContentDrift => {
                let path_str = crate::types::format_section_path(section_path);
                if entry.allowed_content_drift_sections.contains(&path_str) {
                    return true;
                }
                // Content drift is also allowed if the only difference is
                // removable content patterns — checked separately at comparison time
            }
            _ => {}
        }
    }

    false
}

/// Check whether a content line is removable per the allowlist.
pub fn is_removable_line(allowlist: &Allowlist, file: &str, line: &str) -> bool {
    for entry in &allowlist.files {
        if entry.file != file {
            continue;
        }
        for pattern in &entry.removable_content_patterns {
            if line.contains(pattern.as_str()) {
                return true;
            }
        }
        for regex_str in &entry.removable_content_regex {
            // Simple substring matching for "regex" patterns
            if line.contains(regex_str.as_str()) {
                return true;
            }
        }
    }
    false
}

/// Get the set of files that are allowed to be missing for a given allowlist + file.
pub fn allowed_missing_files(allowlist: &Allowlist, file: &str) -> BTreeSet<String> {
    allowlist
        .files
        .iter()
        .filter(|e| e.file == file)
        .flat_map(|e| e.allowed_missing_files.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_rejects_all() {
        let al = empty_allowlist();
        assert!(!is_allowed(
            &al,
            "protocol/runtime-adapters.md",
            &["Default Profiles".to_string()],
            &DriftKind::SectionMissing
        ));
    }

    #[test]
    fn public_allowlist_allows_internal_file_missing() {
        let al = default_public_allowlist();
        assert!(is_allowed(
            &al,
            "AGENTS.md",
            &[],
            &DriftKind::FileMissingInTarget
        ));
    }

    #[test]
    fn public_allowlist_allows_codex_section_missing() {
        let al = default_public_allowlist();
        assert!(is_allowed(
            &al,
            "protocol/runtime-adapters.md",
            &[
                "Runtime Adapters".to_string(),
                "Default Profiles".to_string(),
                "Codex Direct Execution".to_string()
            ],
            &DriftKind::SectionMissing
        ));
    }

    #[test]
    fn removable_line_detection() {
        let al = default_public_allowlist();
        assert!(is_removable_line(
            &al,
            "protocol/runtime-adapters.md",
            "some text with /Volumes/AI Project/ path"
        ));
        assert!(!is_removable_line(
            &al,
            "protocol/runtime-adapters.md",
            "normal protocol content"
        ));
    }

    #[test]
    fn roundtrip_allowlist_json() {
        let al = default_public_allowlist();
        let json = serde_json::to_string_pretty(&al).unwrap();
        let parsed: Allowlist = serde_json::from_str(&json).unwrap();
        assert_eq!(al.files.len(), parsed.files.len());
    }
}
