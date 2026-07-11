//! Legal-difference allowlist for public-full sanitized and other target types.
//!
//! An allowlist declares which differences between source and target are legally
//! permissible (not drift). This supports the public-full sanitized use case
//! where private paths, local collaboration notes, and machine-specific details
//! are intentionally removed from the public distribution.

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

/// Return the built-in public-full sanitized allowlist.
///
/// This encodes the known legal differences between the private suite and
/// a public distribution:
/// - Machine-specific paths are redacted
/// - Internal workflow sections are removed
pub fn default_public_allowlist() -> Allowlist {
    Allowlist {
        description: "Default allowlist for public-full sanitized distribution".into(),
        files: vec![
            public_rewritten_doc("AGENTS.md"),
            public_rewritten_doc("CLAUDE.md"),
            public_rewritten_doc("WORKSPACE.md"),
            public_rewritten_doc("AGENT_SUITE_PROTOCOL.md"),
            public_rewritten_doc("README.md"),
            public_rewritten_doc("Cargo.lock"),
            public_rewritten_doc("protocol/2.0-baseline.md"),
            public_rewritten_doc("protocol/2.0-roadmap.md"),
            public_rewritten_doc("protocol/mcp-server.md"),
            public_rewritten_doc("protocol/context-memory.md"),
            public_rewritten_doc("protocol/project-profile.md"),
            public_rewritten_doc("protocol/skill-governance.md"),
            public_rewritten_doc("scripts/verify.sh"),
            public_rewritten_doc("governance/skill-sync.md"),
            public_rewritten_doc("governance/skill-adoption-log.yaml"),
            public_rewritten_doc("governance/skill-ignore-list.yaml"),
            public_rewritten_doc("manifests/suite.yaml"),
            public_target_only_file("LICENSE"),
            public_target_only_file("templates/task-card-template.md"),
            public_target_only_file("templates/memory/context-capsule.md"),
            public_target_only_file("templates/memory/task-memory.md"),
            public_target_only_file("templates/memory/archive-index.md"),
            public_target_only_file("templates/memory/task-archive/README.md"),
            public_target_only_file("scripts/install.sh"),
            public_target_only_file("scripts/context-memory.sh"),
            public_target_only_file("scripts/stop-archive-hook.sh"),
            public_target_only_file("docs/skill-recommendations.md"),
            public_target_only_file("manifests/skill-recommendations.yaml"),
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
                    "Agent Task Protocol".to_string(),
                    "Agent Task Protocol > 完整生命周期 > 2. Solution Phase（方案形成）"
                        .to_string(),
                    "Agent Task Protocol > 完整生命周期 > 3.10. Capability Route（能力路由，advisory）"
                        .to_string(),
                    "Agent Task Protocol > Executor 入口规则".to_string(),
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

fn public_rewritten_doc(file: &str) -> FileAllowlistEntry {
    FileAllowlistEntry {
        file: file.to_string(),
        allowed_missing_files: vec![],
        allowed_missing_sections: vec!["*".to_string()],
        allowed_content_drift_sections: vec!["*".to_string()],
        removable_content_patterns: vec![],
        removable_content_regex: vec![],
    }
}

fn public_target_only_file(file: &str) -> FileAllowlistEntry {
    FileAllowlistEntry {
        file: file.to_string(),
        allowed_missing_files: vec![file.to_string()],
        allowed_missing_sections: vec![],
        allowed_content_drift_sections: vec![],
        removable_content_patterns: vec![],
        removable_content_regex: vec![],
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
                if entry.allowed_missing_sections.iter().any(|s| s == "*")
                    || entry.allowed_missing_sections.contains(&path_str)
                {
                    return true;
                }
            }
            DriftKind::ContentDrift => {
                let path_str = crate::types::format_section_path(section_path);
                if entry
                    .allowed_content_drift_sections
                    .iter()
                    .any(|s| s == "*")
                    || entry.allowed_content_drift_sections.contains(&path_str)
                {
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
    fn public_allowlist_requires_root_entry_files() {
        let al = default_public_allowlist();
        assert!(!is_allowed(
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
    fn public_allowlist_allows_rewritten_public_docs() {
        let al = default_public_allowlist();
        assert!(is_allowed(
            &al,
            "CLAUDE.md",
            &["Agent Governance Suite 2.0 — Public Edition".to_string()],
            &DriftKind::ExtraSection
        ));
        assert!(is_allowed(
            &al,
            "scripts/verify.sh",
            &["verify.sh — AGS public edition verification gate".to_string()],
            &DriftKind::ContentDrift
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
