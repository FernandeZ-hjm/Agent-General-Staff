//! Workspace identity and routing (contract v3).
//!
//! Single-machine mode: the workspace root is the nearest ancestor directory
//! that contains an `ags.toml`. The daemon / multi-root MCP mode keeps the
//! ordered resolver: explicit workspace context, one unique MCP root, the
//! unique bound workspace containing the adapter cwd, otherwise fail closed
//! (`workspace_required` / `workspace_ambiguous`). HOME, recent-project lists
//! and fuzzy matches are never identity authorities.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

pub const AGS_DIR: &str = ".ags";
pub const AGS_TOML: &str = "ags.toml";

#[derive(Debug, Clone)]
pub struct WorkspaceBinding {
    /// Canonical workspace root (the directory containing `ags.toml`).
    pub root: PathBuf,
    pub slug: String,
    pub role: String,
    pub ags_dir: PathBuf,
    pub evidence_dir: PathBuf,
    pub state_dir: PathBuf,
}

/// Walk upward from `start`; the first directory containing `ags.toml` is the
/// workspace root. Fails closed with `workspace_required` when none exists.
pub fn find_workspace(start: &Path) -> Result<PathBuf> {
    let start = absolutize(start)?;
    let mut current: Option<&Path> = Some(start.as_path());
    while let Some(dir) = current {
        if dir.join(AGS_TOML).is_file() {
            return Ok(dir.to_path_buf());
        }
        current = dir.parent();
    }
    Err(Error::new(
        "workspace_required",
        format!(
            "no {} found in {} or any ancestor directory",
            AGS_TOML,
            start.display()
        ),
    ))
}

fn absolutize(p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .map_err(|e| crate::error::io("cwd_resolve_failed", &e))
    }
}

/// Ordered resolver shared by the MCP adapter and the CLI.
pub fn resolve(
    explicit: Option<&Path>,
    mcp_roots: &[PathBuf],
    adapter_cwd: &Path,
) -> Result<WorkspaceBinding> {
    if let Some(ws) = explicit {
        return bind(ws);
    }
    if mcp_roots.len() == 1 {
        return bind(&mcp_roots[0]);
    }
    let mut hits: Vec<PathBuf> = Vec::new();
    for root in mcp_roots {
        if root.join(AGS_TOML).is_file() && adapter_cwd.starts_with(root) {
            hits.push(root.clone());
        }
    }
    if hits.len() == 1 {
        return bind(&hits[0]);
    }
    if mcp_roots.is_empty() {
        return Err(Error::new(
            "workspace_required",
            "no MCP roots registered and no explicit workspace context provided",
        ));
    }
    Err(Error::new(
        "workspace_ambiguous",
        format!(
            "cannot pick one workspace among {} roots for cwd {}",
            mcp_roots.len(),
            adapter_cwd.display()
        ),
    ))
}

/// Bind a workspace root; the root must contain a parseable `ags.toml`.
pub fn bind(root: &Path) -> Result<WorkspaceBinding> {
    if !root.join(AGS_TOML).is_file() {
        return Err(Error::new(
            "workspace_required",
            format!("{} missing at {}", AGS_TOML, root.display()),
        ));
    }
    let (slug, role) = crate::config::read_identity(root)?;
    let ags_dir = root.join(AGS_DIR);
    Ok(WorkspaceBinding {
        root: root.to_path_buf(),
        slug,
        role,
        evidence_dir: ags_dir.join("evidence"),
        state_dir: ags_dir.join("state"),
        ags_dir,
    })
}

/// Provisional binding for adoption-time operations (`init` plan/apply),
/// where `ags.toml` does not exist yet. The binding hash covers the root
/// path only, so plan and apply agree regardless of adoption progress.
pub fn provisional(root: &Path) -> WorkspaceBinding {
    let ags_dir = root.join(AGS_DIR);
    WorkspaceBinding {
        root: root.to_path_buf(),
        slug: String::new(),
        role: String::new(),
        evidence_dir: ags_dir.join("evidence"),
        state_dir: ags_dir.join("state"),
        ags_dir,
    }
}

/// Content-address the binding; embedded in every sealed action reference.
/// Covers the canonical root path plus the workspace identity (slug + role),
/// so re-sealing under a different workspace identity fails closed. The
/// provisional (pre-init) binding has empty identity fields and hashes the
/// path only.
pub fn binding_hash(binding: &WorkspaceBinding) -> String {
    let mut h = Sha256::new();
    h.update(binding.root.to_string_lossy().as_bytes());
    h.update([0]);
    h.update(binding.slug.as_bytes());
    h.update([0]);
    h.update(binding.role.as_bytes());
    format!("sha256:{}", hex(&h.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}
