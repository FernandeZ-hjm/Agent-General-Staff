//! Project facts used for deterministic task-card slot filling.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Slot sources ────────────────────────────────────────────────────────

/// Where a slot value was sourced from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlotSource {
    /// Directly from the user intent file.
    Intent,
    /// Filled from project context (CLAUDE.md, WORKSPACE.md, protocol files).
    ProjectContext,
    /// Filled from known workspace identity.
    WorkspaceIdentity,
    /// Filled from local memory paths.
    MemoryPath,
    /// A well-known default value (e.g. Execution topology: single).
    Default,
    /// Deterministically derived from the closed handoff contract.
    Derived,
    /// The slot could not be filled.
    Missing,
}

impl SlotSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SlotSource::Intent => "intent",
            SlotSource::ProjectContext => "project_context",
            SlotSource::WorkspaceIdentity => "workspace_identity",
            SlotSource::MemoryPath => "memory_path",
            SlotSource::Default => "default",
            SlotSource::Derived => "derived",
            SlotSource::Missing => "missing",
        }
    }
}

// ── Compile context ─────────────────────────────────────────────────────

/// Context gathered from the project that the compiler uses for slot filling.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    /// Absolute path to the project root.
    pub project_root: PathBuf,
    /// Workspace identity code (A, S, B, etc.) if detected.
    pub workspace_code: Option<String>,
    /// Workspace role description.
    pub workspace_role: Option<String>,
    /// Path to context-capsule.md, if it exists.
    pub capsule_path: Option<PathBuf>,
    /// Path to task-memory.md, if it exists.
    pub task_memory_path: Option<PathBuf>,
    /// Default memory project slug.
    pub memory_slug: Option<String>,
    /// Whether this is an AGS suite (has protocol/ and crates/).
    pub is_ags_suite: bool,
    /// Paths detected from CLAUDE.md protocol references.
    pub claude_md_protocol_refs: Vec<String>,
}

/// Gather project context from the given root directory.
/// This is a pure read-only function — no files are written.
pub fn gather_project_context(root: &Path) -> ProjectContext {
    let root = absolute_project_root(root);
    let is_ags_suite = root.join("CLAUDE.md").exists()
        && root.join("protocol").is_dir()
        && root.join("crates").is_dir();

    // Workspace identity from WORKSPACE.md
    let (workspace_code, workspace_role) = detect_workspace_identity(&root);

    // Memory paths
    let memory_slug = detect_memory_slug(&root);
    let memory_root = ags_platform::home_dir_or_temp()
        .join(".agents")
        .join("memory")
        .join("projects");
    let capsule_path = memory_slug.as_ref().and_then(|slug| {
        let p = memory_root.join(slug).join("context-capsule.md");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    });
    let task_memory_path = memory_slug.as_ref().and_then(|slug| {
        let p = memory_root.join(slug).join("task-memory.md");
        if p.exists() {
            Some(p)
        } else {
            None
        }
    });

    // Extract CLAUDE.md protocol references
    let claude_md_protocol_refs = extract_claude_md_refs(&root);

    ProjectContext {
        project_root: root,
        workspace_code,
        workspace_role,
        capsule_path,
        task_memory_path,
        memory_slug,
        is_ags_suite,
        claude_md_protocol_refs,
    }
}

fn absolute_project_root(root: &Path) -> PathBuf {
    if let Ok(path) = root.canonicalize() {
        return path;
    }
    if root.is_absolute() {
        return root.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(root))
        .unwrap_or_else(|_| root.to_path_buf())
}

/// Detect workspace identity from WORKSPACE.md or known paths.
fn detect_workspace_identity(root: &Path) -> (Option<String>, Option<String>) {
    let workspace_md = root.join("WORKSPACE.md");
    if workspace_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&workspace_md) {
            // Simple table parser: look for | Code | Role | Path |
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('|') && line.contains('|') {
                    let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
                    if parts.len() >= 4 {
                        let code = parts[1].to_string();
                        let role = parts[2].to_string();
                        let entry_path = parts[3].to_string();
                        // Skip header row
                        if code == "Code" {
                            continue;
                        }
                        // Check if current root matches this row's path
                        let resolved_path = shellexpand_path(&entry_path);
                        if paths_equal(&resolved_path, root) {
                            // Strip backtick formatting from role if present
                            let role_clean = role.trim_matches('`').to_string();
                            return (Some(code), Some(role_clean));
                        }
                    }
                }
            }
        }
    }

    (None, None)
}

/// Expand `~` and `$HOME` in a path string.
fn shellexpand_path(s: &str) -> PathBuf {
    let s = s.trim();
    let home = ags_platform::home_dir_or_temp()
        .to_string_lossy()
        .into_owned();
    if s.starts_with("~/") {
        PathBuf::from(s.replacen("~", &home, 1))
    } else if s.starts_with("$HOME/") {
        PathBuf::from(s.replacen("$HOME", &home, 1))
    } else {
        PathBuf::from(s)
    }
}

/// Compare two paths, normalising both.
fn paths_equal(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}

/// Detect the memory slug for this project.
fn detect_memory_slug(root: &Path) -> Option<String> {
    // The project directory is the portable fallback. Hosts may override it
    // through their generated project profile without embedding maintainer
    // machine topology in the compiler.
    root.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())
}

/// Extract protocol file references from CLAUDE.md.
fn extract_claude_md_refs(root: &Path) -> Vec<String> {
    let claude_md = root.join("CLAUDE.md");
    if !claude_md.exists() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&claude_md) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if (line.starts_with("- `") || line.starts_with("- "))
            && (line.contains(".md") || line.contains("protocol/"))
        {
            refs.push(line.trim_start_matches("- ").to_string());
        }
    }
    refs
}
