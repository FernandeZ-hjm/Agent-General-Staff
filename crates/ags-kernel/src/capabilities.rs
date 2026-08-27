//! Capability integrity (contract v3 §7.7).
//!
//! Project `.ags/capabilities.lock` is an audit view of project-local Skill
//! leaves. It is never consulted by runtime routing; the machine skill lock
//! owns body readiness. `ags update` refreshes source-derived audit entries,
//! while `ags check capabilities` reports audit drift explicitly.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::workspace::{sha256_hex, WorkspaceBinding};

pub const LOCK_FILE: &str = "capabilities.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesLock {
    pub version: u32,
    pub entries: Vec<LockEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResolution {
    Exact,
    HashMismatch,
    NotFound,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteCheck {
    pub id: String,
    pub status: String, // "exact" | "hash-mismatch" | "missing"
    pub detail: String,
}

impl CapabilitiesLock {
    pub fn load(binding: &WorkspaceBinding) -> Result<CapabilitiesLock> {
        let path = binding.ags_dir.join(LOCK_FILE);
        if !path.is_file() {
            // Missing lock is a normal empty state, never an error.
            return Ok(CapabilitiesLock::default());
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| crate::error::io("capabilities_lock_read_failed", &e))?;
        serde_json::from_str(&text)
            .map_err(|e| Error::new("capabilities_lock_parse_failed", e.to_string()))
    }

    /// Resolve an exact route: id must exist and the recorded hash must match
    /// the current body hash. No fuzzy or fallback route exists.
    pub fn resolve(&self, id: &str, current_hash: &str) -> RouteResolution {
        match self.entries.iter().find(|e| e.id == id) {
            None => RouteResolution::NotFound,
            Some(entry) => {
                if entry.sha256 == current_hash {
                    RouteResolution::Exact
                } else {
                    RouteResolution::HashMismatch
                }
            }
        }
    }

    /// Check every entry against the live tree: consistent (exact), drifted
    /// (hash mismatch) or missing. Used by `ags check`; a drifted/missing
    /// entry is a hard finding, never a silent refresh.
    pub fn check_routes(&self, root: &Path) -> Vec<RouteCheck> {
        self.entries
            .iter()
            .map(|entry| {
                let full = root.join(&entry.path);
                if !full.exists() {
                    return RouteCheck {
                        id: entry.id.clone(),
                        status: "missing".to_string(),
                        detail: format!("{} does not exist", entry.path),
                    };
                }
                match dir_sha256(&full) {
                    Ok(hash) if hash == entry.sha256 => RouteCheck {
                        id: entry.id.clone(),
                        status: "exact".to_string(),
                        detail: String::new(),
                    },
                    Ok(_) => RouteCheck {
                        id: entry.id.clone(),
                        status: "hash-mismatch".to_string(),
                        detail: format!("{} content drifted from pinned hash", entry.path),
                    },
                    Err(e) => RouteCheck {
                        id: entry.id.clone(),
                        status: "hash-mismatch".to_string(),
                        detail: e.message,
                    },
                }
            })
            .collect()
    }
}

/// Content hash of a capability body directory: every file (relative path +
/// content) participates, sorted for determinism. Empty directories
/// contribute only their path.
pub fn dir_sha256(path: &Path) -> Result<String> {
    let mut parts: Vec<String> = Vec::new();
    if path.is_file() {
        let bytes = fs::read(path).map_err(|e| crate::error::io("capability_read_failed", &e))?;
        return Ok(sha256_hex(&bytes));
    }
    if !path.is_dir() {
        return Err(Error::new(
            "capability_missing",
            format!("{} is not a directory", path.display()),
        ));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(path, &mut files)?;
    files.sort();
    for file in files {
        let rel = file
            .strip_prefix(path)
            .map_err(|e| Error::new("capability_path_invalid", e.to_string()))?;
        let bytes = fs::read(&file).map_err(|e| crate::error::io("capability_read_failed", &e))?;
        parts.push(rel.to_string_lossy().to_string());
        parts.push(sha256_hex(&bytes));
    }
    Ok(sha256_hex(parts.join("\u{0}").as_bytes()))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(|e| crate::error::io("capability_scan_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("capability_scan_failed", &e))?;
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|e| crate::error::io("capability_scan_failed", &e))?;
        if ft.is_symlink() {
            return Err(Error::new(
                "capability_symlink_rejected",
                format!(
                    "symlinks are rejected inside capability bodies: {}",
                    path.display()
                ),
            ));
        }
        if ft.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_skill_body_dirs(base: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(base).map_err(|e| crate::error::io("capability_scan_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("capability_scan_failed", &e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            out.push(path);
        } else {
            collect_skill_body_dirs(&path, out)?;
        }
    }
    Ok(())
}

/// Refresh the project audit lock from real skill leaves. Category directories
/// such as `skill-packs/optional` are never recorded as fake skills. Explicit
/// declarations outside configured sources are retained for audit continuity.
pub fn refresh(binding: &WorkspaceBinding, sources: &[String]) -> Result<CapabilitiesLock> {
    let previous = CapabilitiesLock::load(binding).unwrap_or_default();
    let mut entries: Vec<LockEntry> = Vec::new();
    let mut source_roots = Vec::new();
    for source in sources {
        let base = binding.root.join(source);
        if !base.is_dir() {
            continue;
        }
        source_roots.push(base.clone());
        let mut bodies = Vec::new();
        collect_skill_body_dirs(&base, &mut bodies)?;
        for path in bodies {
            let id = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let hash = dir_sha256(&path)?;
            entries.retain(|e| e.id != id);
            entries.push(LockEntry {
                id,
                kind: "skill".to_string(),
                path: path
                    .strip_prefix(&binding.root)
                    .map_err(|e| Error::new("capability_path_invalid", e.to_string()))?
                    .to_string_lossy()
                    .to_string(),
                sha256: hash,
                hosts: vec![],
            });
        }
    }
    for entry in previous.entries {
        let path = binding.root.join(&entry.path);
        let is_source_derived = source_roots.iter().any(|root| path.starts_with(root));
        if !is_source_derived && path.is_dir() && !entries.iter().any(|e| e.id == entry.id) {
            entries.push(entry);
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    let lock = CapabilitiesLock {
        version: 1,
        entries,
    };
    let text = serde_json::to_string_pretty(&lock)
        .map_err(|e| Error::new("capabilities_lock_encode_failed", e.to_string()))?;
    fs::create_dir_all(&binding.ags_dir)
        .map_err(|e| crate::error::io("ags_dir_create_failed", &e))?;
    let path = binding.ags_dir.join(LOCK_FILE);
    let tmp = binding.ags_dir.join(format!("{LOCK_FILE}.tmp"));
    fs::write(&tmp, text).map_err(|e| crate::error::io("capabilities_lock_write_failed", &e))?;
    fs::rename(&tmp, &path).map_err(|e| crate::error::io("capabilities_lock_write_failed", &e))?;
    Ok(lock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::bind;

    fn make_ws(tmp: &tempfile::TempDir) -> WorkspaceBinding {
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n",
        )
        .unwrap();
        bind(&root).unwrap()
    }

    #[test]
    fn refresh_and_exact_route() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        let skill = ws.root.join("ags-skills/demo");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Demo\n").unwrap();
        let lock = refresh(&ws, &["ags-skills".to_string()]).unwrap();
        assert_eq!(lock.entries.len(), 1);
        let hash = dir_sha256(&skill).unwrap();
        assert_eq!(lock.resolve("demo", &hash), RouteResolution::Exact);
        assert_eq!(
            lock.resolve("demo", "otherhash"),
            RouteResolution::HashMismatch
        );
        assert_eq!(lock.resolve("nope", &hash), RouteResolution::NotFound);
    }

    #[test]
    fn refresh_records_nested_skill_leaves_not_category_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        for rel in ["skill-packs/optional/a", "skill-packs/personal/b"] {
            let skill = ws.root.join(rel);
            fs::create_dir_all(&skill).unwrap();
            fs::write(skill.join("SKILL.md"), "# Skill\n").unwrap();
        }
        let lock = refresh(&ws, &["skill-packs".to_string()]).unwrap();
        let ids: Vec<&str> = lock.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(!ids.contains(&"optional"));
        assert!(!ids.contains(&"personal"));
    }

    #[test]
    fn refresh_retains_explicit_audit_entries_outside_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        let manual = ws.root.join("vendor/manual");
        let source = ws.root.join("skill-packs/optional/a");
        fs::create_dir_all(&manual).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(manual.join("SKILL.md"), "# Manual\n").unwrap();
        fs::write(source.join("SKILL.md"), "# A\n").unwrap();
        fs::create_dir_all(&ws.ags_dir).unwrap();
        let existing = CapabilitiesLock {
            version: 3,
            entries: vec![LockEntry {
                id: "manual".to_string(),
                kind: "skill".to_string(),
                path: "vendor/manual".to_string(),
                sha256: dir_sha256(&manual).unwrap(),
                hosts: vec![],
            }],
        };
        fs::write(
            ws.ags_dir.join(LOCK_FILE),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        let lock = refresh(&ws, &["skill-packs".to_string()]).unwrap();
        let ids: Vec<&str> = lock.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "manual"]);
    }

    #[test]
    fn drifted_tree_is_detected_not_silently_refreshed() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        let skill = ws.root.join("ags-skills/demo");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Demo\n").unwrap();
        let lock = refresh(&ws, &["ags-skills".to_string()]).unwrap();
        fs::write(skill.join("SKILL.md"), "# Demo v2\n").unwrap();
        let checks = lock.check_routes(&ws.root);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, "hash-mismatch");
    }

    #[test]
    fn symlinks_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        let skill = ws.root.join("ags-skills/demo");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Demo\n").unwrap();
        let outside = tmp.path().join("outside.txt");
        fs::write(&outside, "x").unwrap();
        std::os::unix::fs::symlink(&outside, skill.join("link")).unwrap();
        assert!(dir_sha256(&skill).is_err());
    }

    #[test]
    fn missing_lock_is_empty_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = make_ws(&tmp);
        let lock = CapabilitiesLock::load(&ws).unwrap();
        assert!(lock.entries.is_empty());
    }
}
