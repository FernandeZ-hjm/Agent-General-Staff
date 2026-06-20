//! Minimal managed-projects registry.
//!
//! Records every repository that `ags init` has onboarded into AGS, as a small
//! hand-serialized YAML file under the AGS runtime home
//! (`~/.ags/runtime/managed-projects.yaml`, honoring `$AGS_HOME`).
//!
//! - **Write side**: `ags init` upserts (append + dedupe on canonical path) the
//!   project after a successful onboard. Re-running `ags init` refreshes the
//!   entry without losing the first-registration time.
//! - **Query side** (read-only): consumed by `ags update` (sync plan),
//!   `ags doctor` (global scan), and `ags agents verify` (sampled preflight).
//!
//! This is an inventory, NOT a sync ledger. A GitHub/remote-backed project
//! (one with an `origin`) is marked accordingly so downstream `ags update` keeps
//! it local-plan-only and never auto-pushes/fetches.
//!
//! The module has no `serde_yaml` dependency (the workspace does not use one):
//! it round-trips its own flat schema with a small line parser. The functions
//! are pure over their inputs (paths injected) so tests never touch the real
//! `$HOME`.

use std::path::{Path, PathBuf};

/// Registry schema version (the registry file format, not the suite version).
pub const MANAGED_PROJECTS_SCHEMA: &str = "1";

/// Version-control status of a managed project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectVcs {
    Git,
    None,
}

impl ProjectVcs {
    fn as_str(&self) -> &'static str {
        match self {
            ProjectVcs::Git => "git",
            ProjectVcs::None => "none",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "git" => Some(ProjectVcs::Git),
            "none" => Some(ProjectVcs::None),
            _ => None,
        }
    }
}

/// One AGS-onboarded project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProject {
    /// Canonicalized absolute repo root (dedupe key).
    pub path: String,
    /// Project slug used for the memory-capsule directory.
    pub slug: String,
    /// `git` when a git worktree, else `none`.
    pub vcs: ProjectVcs,
    /// `Some(url)` when `git remote get-url origin` succeeded. Presence marks the
    /// project remote/GitHub-backed → sync stays local-plan-only.
    pub origin: Option<String>,
    /// Unix seconds of first registration (stable; not bumped on re-init).
    pub registered_at: u64,
    /// Unix seconds of the most recent `ags init` touch.
    pub last_init_at: u64,
}

/// The full registry.
#[derive(Debug, Clone)]
pub struct ManagedProjectsRegistry {
    pub schema_version: String,
    pub projects: Vec<ManagedProject>,
}

impl Default for ManagedProjectsRegistry {
    fn default() -> Self {
        ManagedProjectsRegistry {
            schema_version: MANAGED_PROJECTS_SCHEMA.to_string(),
            projects: Vec::new(),
        }
    }
}

/// Outcome of an `upsert`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryChange {
    Added,
    Refreshed,
    Unchanged,
}

/// Registry file path under the given AGS runtime home.
pub fn registry_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("managed-projects.yaml")
}

/// Build a `ManagedProject` from explicit facts (pure; caller does git detection).
pub fn describe_project(
    path: String,
    slug: String,
    now: u64,
    vcs: ProjectVcs,
    origin: Option<String>,
) -> ManagedProject {
    ManagedProject {
        path,
        slug,
        vcs,
        origin,
        registered_at: now,
        last_init_at: now,
    }
}

/// True when the project is remote/GitHub-backed (has an `origin`).
pub fn is_remote_backed(p: &ManagedProject) -> bool {
    p.origin.is_some()
}

/// Append-or-refresh one project, deduped on canonical `path`. Does NOT persist.
/// On an existing entry: preserves `registered_at`; refreshes
/// `last_init_at` / `vcs` / `origin`. Returns whether anything changed.
pub fn upsert(reg: &mut ManagedProjectsRegistry, entry: ManagedProject) -> RegistryChange {
    if let Some(existing) = reg.projects.iter_mut().find(|p| p.path == entry.path) {
        // Compute deltas BEFORE mutating; `registered_at` is preserved.
        let content_changed = existing.slug != entry.slug
            || existing.vcs != entry.vcs
            || existing.origin != entry.origin;
        let stamp_advanced = existing.last_init_at != entry.last_init_at;
        existing.slug = entry.slug;
        existing.vcs = entry.vcs;
        existing.origin = entry.origin;
        existing.last_init_at = entry.last_init_at;
        if content_changed || stamp_advanced {
            RegistryChange::Refreshed
        } else {
            RegistryChange::Unchanged
        }
    } else {
        reg.projects.push(entry);
        RegistryChange::Added
    }
}

/// Hand-serialize to canonical YAML (sorted by path for stable diffs).
pub fn render_yaml(reg: &ManagedProjectsRegistry) -> String {
    let mut projects = reg.projects.clone();
    projects.sort_by(|a, b| a.path.cmp(&b.path));
    let mut out = String::new();
    out.push_str("# AGS managed-projects registry — maintained by `ags init`.\n");
    out.push_str("# Read-only query side: ags update / ags doctor / ags agents verify.\n");
    out.push_str(&format!("schema_version: \"{}\"\n", reg.schema_version));
    if projects.is_empty() {
        out.push_str("projects: []\n");
        return out;
    }
    out.push_str("projects:\n");
    for p in &projects {
        out.push_str(&format!("  - path: {}\n", yaml_quote(&p.path)));
        out.push_str(&format!("    slug: {}\n", yaml_quote(&p.slug)));
        out.push_str(&format!("    vcs: {}\n", p.vcs.as_str()));
        if let Some(origin) = &p.origin {
            out.push_str(&format!("    origin: {}\n", yaml_quote(origin)));
        }
        out.push_str(&format!("    registered_at: {}\n", p.registered_at));
        out.push_str(&format!("    last_init_at: {}\n", p.last_init_at));
    }
    out
}

/// Read + parse the registry. Missing file => empty registry (Ok). Malformed
/// content => Err(detail) so callers report drift instead of clobbering.
pub fn load(path: &Path) -> Result<ManagedProjectsRegistry, String> {
    if !path.exists() {
        return Ok(ManagedProjectsRegistry::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read registry: {e}"))?;
    parse_registry(&text)
}

fn parse_registry(text: &str) -> Result<ManagedProjectsRegistry, String> {
    let mut reg = ManagedProjectsRegistry::default();
    let mut current: Option<PartialProject> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("schema_version:") {
            reg.schema_version = yaml_unquote(v.trim());
            continue;
        }
        if line == "projects:" || line == "projects: []" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            if let Some(p) = current.take() {
                reg.projects.push(p.into_project()?);
            }
            let mut partial = PartialProject::default();
            apply_field(&mut partial, rest)?;
            current = Some(partial);
            continue;
        }
        if line.contains(':') {
            let partial = current
                .as_mut()
                .ok_or_else(|| "registry field outside a project entry".to_string())?;
            apply_field(partial, line)?;
            continue;
        }
        return Err(format!("unrecognized registry line: {line}"));
    }
    if let Some(p) = current.take() {
        reg.projects.push(p.into_project()?);
    }
    Ok(reg)
}

#[derive(Default)]
struct PartialProject {
    path: Option<String>,
    slug: Option<String>,
    vcs: Option<ProjectVcs>,
    origin: Option<String>,
    registered_at: Option<u64>,
    last_init_at: Option<u64>,
}

impl PartialProject {
    fn into_project(self) -> Result<ManagedProject, String> {
        let path = self
            .path
            .ok_or_else(|| "project entry missing path".to_string())?;
        let slug = self
            .slug
            .ok_or_else(|| format!("project entry {path} missing slug"))?;
        Ok(ManagedProject {
            path,
            slug,
            vcs: self.vcs.unwrap_or(ProjectVcs::None),
            origin: self.origin,
            registered_at: self.registered_at.unwrap_or(0),
            last_init_at: self.last_init_at.unwrap_or(0),
        })
    }
}

fn apply_field(p: &mut PartialProject, field: &str) -> Result<(), String> {
    let (key, value) = field
        .split_once(':')
        .ok_or_else(|| format!("malformed registry field: {field}"))?;
    let key = key.trim();
    let value = value.trim();
    match key {
        "path" => p.path = Some(yaml_unquote(value)),
        "slug" => p.slug = Some(yaml_unquote(value)),
        "origin" => p.origin = Some(yaml_unquote(value)),
        "vcs" => {
            p.vcs = Some(ProjectVcs::parse(value).ok_or_else(|| format!("bad vcs value: {value}"))?)
        }
        "registered_at" => {
            p.registered_at = Some(
                value
                    .parse()
                    .map_err(|_| format!("bad registered_at: {value}"))?,
            )
        }
        "last_init_at" => {
            p.last_init_at = Some(
                value
                    .parse()
                    .map_err(|_| format!("bad last_init_at: {value}"))?,
            )
        }
        other => return Err(format!("unknown registry field: {other}")),
    }
    Ok(())
}

/// Split into (existing-on-disk, stale) — stale projects are reported, never
/// auto-removed (no silent deletion of user state).
pub fn partition_existing(
    reg: &ManagedProjectsRegistry,
) -> (Vec<&ManagedProject>, Vec<&ManagedProject>) {
    let mut existing = Vec::new();
    let mut stale = Vec::new();
    for p in &reg.projects {
        if Path::new(&p.path).is_dir() {
            existing.push(p);
        } else {
            stale.push(p);
        }
    }
    (existing, stale)
}

/// Render the registry as JSON (machine-readable query output).
pub fn render_registry_json(reg: &ManagedProjectsRegistry) -> String {
    let projects: Vec<serde_json::Value> = reg
        .projects
        .iter()
        .map(|p| {
            let mut obj = serde_json::json!({
                "path": p.path,
                "slug": p.slug,
                "vcs": p.vcs.as_str(),
                "registered_at": p.registered_at,
                "last_init_at": p.last_init_at,
            });
            if let Some(origin) = &p.origin {
                obj["origin"] = serde_json::Value::String(origin.clone());
            }
            obj
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": reg.schema_version,
        "projects": projects,
    }))
    .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {e}"}}"#))
}

/// Render the registry as a short human-readable text summary.
pub fn render_registry_text(reg: &ManagedProjectsRegistry) -> String {
    let mut lines = vec![format!(
        "Managed projects: {} (schema {})",
        reg.projects.len(),
        reg.schema_version
    )];
    let (existing, stale) = partition_existing(reg);
    for p in &existing {
        let origin = p
            .origin
            .as_deref()
            .map(|o| format!(" origin={o}"))
            .unwrap_or_default();
        lines.push(format!("  - {} [{}]{}", p.path, p.vcs.as_str(), origin));
    }
    for p in &stale {
        lines.push(format!("  - {} (stale: path not found)", p.path));
    }
    lines.join("\n")
}

// ── YAML scalar helpers ──────────────────────────────────────────────────────

fn yaml_quote(s: &str) -> String {
    // Always quote string scalars; escape backslash and double-quote.
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn yaml_unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("ags-mp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn registry_path_honors_runtime_home() {
        let dir = temp_home("registry-path");
        let p = registry_path(&dir);
        assert!(p.starts_with(&dir));
        assert_eq!(p.file_name().unwrap(), "managed-projects.yaml");
    }

    #[test]
    fn upsert_dedupes_by_path() {
        let mut reg = ManagedProjectsRegistry::default();
        let a = describe_project("/r".into(), "r".into(), 100, ProjectVcs::Git, None);
        assert_eq!(upsert(&mut reg, a), RegistryChange::Added);
        let mut a2 = describe_project("/r".into(), "r".into(), 200, ProjectVcs::Git, None);
        a2.registered_at = 200; // upsert must preserve the original 100
        let change = upsert(&mut reg, a2);
        assert_eq!(change, RegistryChange::Refreshed);
        assert_eq!(reg.projects.len(), 1);
        assert_eq!(reg.projects[0].registered_at, 100);
        assert_eq!(reg.projects[0].last_init_at, 200);
    }

    #[test]
    fn roundtrip_yaml_is_stable() {
        let mut reg = ManagedProjectsRegistry::default();
        upsert(
            &mut reg,
            describe_project("/b".into(), "b".into(), 1, ProjectVcs::Git, None),
        );
        upsert(
            &mut reg,
            describe_project(
                "/a".into(),
                "a".into(),
                2,
                ProjectVcs::Git,
                Some("git@github.com:x/y.git".into()),
            ),
        );
        let y1 = render_yaml(&reg);
        let parsed = parse_registry(&y1).unwrap();
        let y2 = render_yaml(&parsed);
        assert_eq!(y1, y2);
        // sorted by path: /a before /b
        assert!(y1.find("/a").unwrap() < y1.find("/b").unwrap());
    }

    #[test]
    fn origin_marks_remote_backed() {
        let remote = describe_project(
            "/r".into(),
            "r".into(),
            1,
            ProjectVcs::Git,
            Some("git@github.com:x/y.git".into()),
        );
        assert!(is_remote_backed(&remote));
        let local = describe_project("/l".into(), "l".into(), 1, ProjectVcs::Git, None);
        assert!(!is_remote_backed(&local));
        let yaml = render_yaml(&{
            let mut r = ManagedProjectsRegistry::default();
            upsert(&mut r, remote);
            r
        });
        assert!(yaml.contains("origin:"));
    }

    #[test]
    fn load_missing_is_empty_ok() {
        let dir = temp_home("load-missing");
        let reg = load(&registry_path(&dir)).unwrap();
        assert_eq!(reg.schema_version, MANAGED_PROJECTS_SCHEMA);
        assert!(reg.projects.is_empty());
    }

    #[test]
    fn load_malformed_is_err() {
        let res = parse_registry("schema_version: \"1\"\nprojects:\n  - slug: \"x\"\n");
        assert!(res.is_err(), "entry without path must be drift, not silent");
    }
}
