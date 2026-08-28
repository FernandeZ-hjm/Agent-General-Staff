//! Entry-ecosystem sync — the "update together" mechanism (v0.4.21).
//!
//! AGS updates must move the whole entry ecosystem in one transaction:
//! host entry files (AGENTS.md / CLAUDE.md / HERMES.md / CODEBUDDY.md)
//! managed blocks, global rules (`~/.agents/rules/`), installed AGS skills
//! (`~/.agents/skills/ags-*`), and protocol files. Managed blocks delimit user
//! content; product-owned skills are replaced from source on every update so
//! same-version fixes converge. `ags update` is the single entry point and
//! `ags doctor` reports runtime drift.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Serializes tests that override the process-global HOME.
#[cfg(test)]
pub(crate) static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Product version every managed fragment is stamped with.
pub const AGS_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// Managed-block marker names (kept from the contract-v2 convention so old
/// blocks are cleanly replaced, not duplicated).
pub const BLOCK_BEGIN: &str = "<!-- BEGIN AGS MANAGED BLOCK -->";
pub const BLOCK_END: &str = "<!-- END AGS MANAGED BLOCK -->";

/// Registry of AGS-managed projects (machine-local, `~/.ags/v3/`).
const REGISTRY_PATH: &str = ".ags/v3/managed.json";
const INSTALL_PATH: &str = ".ags/v3/install.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagedRegistry {
    pub schema_version: String,
    pub projects: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallInfo {
    pub schema_version: String,
    /// Checkout that is the fragment source (stable by default).
    pub source_root: PathBuf,
}

pub fn machine_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("runtime_home_missing", "HOME or USERPROFILE is not set"))
}

fn runtime_home() -> Result<PathBuf> {
    machine_home()
}

/// The host-entry managed block for the current version. Short, factual,
/// points into the workspace policy — never a protocol encyclopedia.
pub fn entry_block() -> String {
    format!(
        "{BLOCK_BEGIN}\n## Agent Governance Suite\n\n\
This project is governed by AGS {AGS_VERSION} (contract v3).\n\n\
- Policy lives in `ags.toml` (permission matrix, write boundaries, sealed ops, verify commands).\n\
- Health check: `ags doctor --workspace .`\n\
- Tasks: `ags run --task <card>` (up to 13 fields); sealed ops need `ags apply <ACTION_REF>`.\n\
- Skills: host-selected wins; else `ags route <need>`. Use `ags skill list|recommend`; ask before install.\n\
- Evidence: `.ags/evidence/events.jsonl`; project capability audit: `.ags/capabilities.lock`.\n\
- This managed block is a local Git projection and is never committed.\n\
- This block is maintained by `ags update` — edit only outside the markers.\n\
{BLOCK_END}"
    )
}

/// Rules file content (`~/.agents/rules/ags-core.md`).
pub fn rules_core() -> String {
    format!(
        "# AGS Core Agent Rules — v{AGS_VERSION} (contract v3)\n\n\
## First-principles priority\n\n\
Derive decisions from goals, facts, constraints, invariants, cost, and\n\
verifiable evidence. Skills and MCP servers provide methods and execution\n\
surfaces; they do not expand authority or replace host judgment.\n\n\
## Governance entry (contract v3)\n\n\
- The host interprets natural language once; AGS consumes typed Operations and\n\
  validated task cards. Do not send raw prose to AGS.\n\
- CLI: `ags init|run|apply|check|test|log|status|doctor|setup|upgrade|update|skill|route|govern|schema`.\n\
- Policy: `ags.toml` (allow/ask/deny matrix, write boundaries, sealed ops).\n\
- Skill selection: a skill already selected by the host wins. Only when the\n\
  host has no clear match, call `ags route <need>` once and use a unique\n\
  verified result; abstain on ties or an unready candidate.\n\
- Skill lifecycle: `ags skill list|recommend` is read-only. If a recommended\n\
  Skill is missing, ask before obtaining it or planning sealed install/remove\n\
  under `ags govern skill`; `ags apply` consumes the plan exactly once.\n\
- Default task flow: `ags run --task <card>` → host executes → `--verify` → `--close`.\n\
- Health: `ags doctor --workspace .`. Evidence: `.ags/evidence/events.jsonl`.\n\
- This file is maintained by `ags update` together with the product.\n"
    )
}

/// Rules shim (`~/.agents/rules/core.md`).
pub fn rules_core_shim() -> String {
    "# Shared Agent Rules\n\n\
AGS 核心规则由 `ags update` 安装到同目录 `ags-core.md`。宿主入口应直接引用\n\
`ags-core.md`；本文件保留为旧入口兼容提示，不再承载协议正文。\n"
        .to_string()
}

/// Protocol reference used to refresh version-stamped project copies.
pub fn protocol_doc() -> &'static str {
    include_str!("../templates/AGENT_SUITE_PROTOCOL.md")
}

// ── registry ────────────────────────────────────────────────────────────

pub fn registry_path() -> Result<PathBuf> {
    Ok(runtime_home()?.join(REGISTRY_PATH))
}

pub fn load_registry() -> Result<ManagedRegistry> {
    let path = registry_path()?;
    if !path.is_file() {
        return Ok(ManagedRegistry {
            schema_version: "ags://schema/contract/v3/managed-registry".to_string(),
            projects: vec![],
        });
    }
    let text =
        fs::read_to_string(&path).map_err(|e| crate::error::io("registry_read_failed", &e))?;
    serde_json::from_str(&text).map_err(|e| Error::new("registry_parse_failed", e.to_string()))
}

pub fn save_registry(registry: &ManagedRegistry) -> Result<()> {
    let path = registry_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io("registry_dir_failed", &e))?;
    }
    let text = serde_json::to_string_pretty(registry)
        .map_err(|e| Error::new("registry_encode_failed", e.to_string()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| crate::error::io("registry_write_failed", &e))?;
    fs::rename(&tmp, &path).map_err(|e| crate::error::io("registry_write_failed", &e))?;
    Ok(())
}

pub fn register_project(root: &Path) -> Result<()> {
    let mut registry = load_registry()?;
    let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !registry.projects.contains(&canonical) {
        registry.projects.push(canonical);
        save_registry(&registry)?;
    }
    Ok(())
}

pub fn install_info() -> Result<InstallInfo> {
    let explicit = std::env::var("AGS_SOURCE_ROOT").ok();
    if let Some(root) = explicit {
        return Ok(InstallInfo {
            schema_version: "ags://schema/contract/v3/install".to_string(),
            source_root: PathBuf::from(root),
        });
    }
    let path = runtime_home()?.join(INSTALL_PATH);
    let text = fs::read_to_string(&path)
        .map_err(|_| Error::new(
            "install_info_missing",
            "AGS install record (~/.ags/v3/install.json) is missing; run `ags setup --source-root <dir>` first",
        ))?;
    serde_json::from_str(&text).map_err(|e| {
        Error::new(
            "install_info_corrupted",
            format!("~/.ags/v3/install.json is unreadable ({e}); re-run `ags setup --source-root <dir>`"),
        )
    })
}

/// Machine-level install: record the official content source and converge
/// official skills plus the machine lock. Idempotent; this is the only place
/// `install.json` is written.
pub fn setup(source_root: &Path) -> Result<Vec<String>> {
    let canonical = source_root.canonicalize().map_err(|e| {
        Error::new(
            "setup_source_missing",
            format!("cannot resolve source root {}: {e}", source_root.display()),
        )
    })?;
    if !canonical.join("ags-skills").is_dir() {
        return Err(Error::new(
            "setup_source_invalid",
            format!(
                "{} has no ags-skills/ directory; point --source-root at an AGS checkout",
                canonical.display()
            ),
        ));
    }
    let mut wrote = sync_rules()?;
    wrote.extend(sync_skills(&canonical)?);
    let (lock, problems) = sync_bodies()?;
    wrote.push(format!(
        "machine-capabilities:{} bodies",
        lock.entries.len()
    ));
    for problem in problems {
        wrote.push(format!("machine-capabilities-problem:{problem}"));
    }
    // The install record is the setup commit marker. Write it only after every
    // required machine artifact has converged, so a partial setup never claims
    // that the runtime is installed.
    save_install_info(&InstallInfo {
        schema_version: "ags://schema/contract/v3/install".to_string(),
        source_root: canonical,
    })?;
    Ok(wrote)
}

pub fn save_install_info(info: &InstallInfo) -> Result<()> {
    let path = runtime_home()?.join(INSTALL_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io("registry_dir_failed", &e))?;
    }
    let text = serde_json::to_string_pretty(info)
        .map_err(|e| Error::new("install_encode_failed", e.to_string()))?;
    fs::write(path, text).map_err(|e| crate::error::io("install_write_failed", &e))?;
    Ok(())
}

// ── entry blocks ────────────────────────────────────────────────────────

/// Replace the AGS managed block in `file`, or append it when absent.
/// Never touches content outside the markers. Returns whether the block is
/// present with the current version afterwards.
pub fn sync_entry_block(file: &Path) -> Result<bool> {
    if !file.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(file).map_err(|e| crate::error::io("entry_read_failed", &e))?;
    let new_text = render_entry_text(&text);
    if new_text != text {
        fs::write(file, new_text).map_err(|e| crate::error::io("entry_write_failed", &e))?;
    }
    Ok(true)
}

pub fn render_entry_text(text: &str) -> String {
    let user_text = strip_legacy_entry_sections(&strip_entry_text(text));
    if user_text.is_empty() {
        format!("{}\n", entry_block())
    } else {
        format!("{}\n{}", entry_block(), user_text)
    }
}

pub fn strip_entry_text(text: &str) -> String {
    let Some((begin, end)) = entry_range(text) else {
        return text.to_string();
    };
    let before = &text[..begin];
    let mut after = &text[end..];
    if after.starts_with('\n') && (begin == 0 || before.ends_with('\n')) {
        after = &after[1..];
    }
    format!("{before}{after}")
}

fn entry_range(text: &str) -> Option<(usize, usize)> {
    let begin = text.find(BLOCK_BEGIN)?;
    let end = text[begin..].find(BLOCK_END)? + begin + BLOCK_END.len();
    Some((begin, end))
}

fn strip_legacy_entry_sections(text: &str) -> String {
    let mut out = text.to_string();
    loop {
        let mut legacy_start = None;
        let mut cursor = 0;
        for line in out.split_inclusive('\n') {
            if line.trim_end_matches(['\r', '\n']) == "## Agent Governance Suite" {
                legacy_start = Some(cursor);
                break;
            }
            cursor += line.len();
        }
        let Some(start) = legacy_start else {
            return out;
        };

        let mut end = out.len();
        let mut cursor = start;
        for (index, line) in out[start..].split_inclusive('\n').enumerate() {
            if index > 0 && line.starts_with("## ") {
                end = cursor;
                break;
            }
            cursor += line.len();
        }

        let before = out[..start].trim_end_matches(['\r', '\n']);
        let after = out[end..].trim_start_matches(['\r', '\n']);
        out = match (before.is_empty(), after.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!("{before}\n"),
            (true, false) => after.to_string(),
            (false, false) => format!("{before}\n\n{after}"),
        };
    }
}

/// Refresh the version-stamped protocol copy when the project already
/// carries one that AGS owns (first heading match). Never injects into
/// projects that never had it.
pub fn sync_protocol_copy(root: &Path) -> Result<bool> {
    let path = root.join("AGENT_SUITE_PROTOCOL.md");
    if !path.is_file() {
        return Ok(false);
    }
    let text =
        fs::read_to_string(&path).map_err(|e| crate::error::io("protocol_read_failed", &e))?;
    let first_heading = text.lines().find(|l| !l.trim().is_empty()).map(str::trim);
    let owned = matches!(first_heading, Some(line) if
        line.starts_with("# Agent Governance Suite Protocol")
            || (line == "# AGENT_SUITE_PROTOCOL.md"
                && text.contains("Agent Governance Suite")));
    if !owned {
        return Ok(false);
    }
    fs::write(&path, protocol_doc()).map_err(|e| crate::error::io("protocol_write_failed", &e))?;
    Ok(true)
}

/// Host entry files that receive the managed block.
pub const ENTRY_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "HERMES.md",
    "CODEBUDDY.md",
    "codebuddy.md",
];

/// Sync one registered project: entry blocks + owned protocol copy.
pub fn sync_project(root: &Path) -> Result<Vec<String>> {
    let mut wrote = Vec::new();
    for name in ENTRY_FILES {
        let path = root.join(name);
        if sync_entry_block(&path)? {
            wrote.push(name.to_string());
        }
    }
    if sync_protocol_copy(root)? {
        wrote.push("AGENT_SUITE_PROTOCOL.md".to_string());
    }
    Ok(wrote)
}

pub fn preflight_project(root: &Path) -> Result<()> {
    for name in ENTRY_FILES
        .iter()
        .copied()
        .chain(std::iter::once("AGENT_SUITE_PROTOCOL.md"))
    {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        fs::read_to_string(&path).map_err(|e| crate::error::io("entry_read_failed", &e))?;
        if path
            .metadata()
            .map_err(|e| crate::error::io("entry_metadata_failed", &e))?
            .permissions()
            .readonly()
        {
            return Err(crate::Error::new(
                "entry_not_writable",
                format!("{} is read-only", path.display()),
            ));
        }
    }
    Ok(())
}

// ── rules and skills ────────────────────────────────────────────────────

pub fn rules_dir() -> Result<PathBuf> {
    Ok(runtime_home()?.join(".agents/rules"))
}

pub fn skills_dir() -> Result<PathBuf> {
    Ok(runtime_home()?.join(".agents/skills"))
}

/// Refresh the global rules files.
pub fn sync_rules() -> Result<Vec<String>> {
    let dir = rules_dir()?;
    fs::create_dir_all(&dir).map_err(|e| crate::error::io("rules_dir_failed", &e))?;
    let mut wrote = Vec::new();
    for (name, content) in [
        ("ags-core.md", rules_core()),
        ("core.md", rules_core_shim()),
    ] {
        let path = dir.join(name);
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current != content {
            fs::write(&path, content).map_err(|e| crate::error::io("rules_write_failed", &e))?;
            wrote.push(name.to_string());
        }
    }
    Ok(wrote)
}

fn official_skill_owned(target: &Path, name: &str) -> bool {
    let marker = fs::read_to_string(target.join(".ags-skill-version"));
    let declared_name = fs::read_to_string(target.join("SKILL.md"))
        .ok()
        .and_then(|text| crate::route::parse_skill_frontmatter(&text).name);
    marker.map(|text| !text.trim().is_empty()).unwrap_or(false)
        && declared_name.as_deref() == Some(name)
}

fn remove_tree_entry(path: &Path) -> Result<()> {
    let meta =
        fs::symlink_metadata(path).map_err(|e| crate::error::io("skill_replace_failed", &e))?;
    if directory_link_target(path)
        .map_err(|e| crate::error::io("skill_replace_failed", &e))?
        .is_some()
    {
        remove_directory_link(path).map_err(|e| crate::error::io("skill_replace_failed", &e))
    } else if !meta.is_dir() {
        fs::remove_file(path).map_err(|e| crate::error::io("skill_replace_failed", &e))
    } else {
        fs::remove_dir_all(path).map_err(|e| crate::error::io("skill_replace_failed", &e))
    }
}

/// Refresh installed AGS skills from the source checkout. The official set is
/// tiny and product-owned, so update always replaces it from a fully prepared
/// staging directory; a global version stamp never hides a same-version fix.
/// Third-party skills remain untouched and are installed on demand.
pub fn sync_skills(source_root: &Path) -> Result<Vec<String>> {
    let src = source_root.join("ags-skills");
    let dst = skills_dir()?;
    let staging_root = dst
        .parent()
        .ok_or_else(|| Error::new("skills_dir_invalid", "skills directory has no parent"))?
        .join(".ags-skill-staging");
    let mut wrote = Vec::new();
    if !src.is_dir() {
        return Err(Error::new(
            "skill_source_missing",
            format!(
                "official skill source {} does not exist; run `ags setup --source-root <dir>` first",
                src.display()
            ),
        ));
    }
    fs::create_dir_all(&dst).map_err(|e| crate::error::io("skills_dir_failed", &e))?;
    for entry in fs::read_dir(&src).map_err(|e| crate::error::io("skills_scan_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("skills_scan_failed", &e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let from = entry.path();
        if !name.starts_with("ags-") || !from.is_dir() {
            continue;
        }
        validate_skill_source(&name, &from)?;
        let target = dst.join(&name);
        if fs::symlink_metadata(&target).is_ok() {
            let is_link = directory_link_target(&target)
                .map_err(|e| crate::error::io("skill_symlink_repair_failed", &e))?
                .is_some();
            if is_link && fs::canonicalize(&target).is_err() {
                // A dangling link contains no body to preserve and cannot
                // prove ownership; repair it like third-party reinstall does.
                remove_directory_link(&target)
                    .map_err(|e| crate::error::io("skill_symlink_repair_failed", &e))?;
            } else if !official_skill_owned(&target, &name) {
                return Err(Error::new(
                    "official_skill_conflict",
                    format!(
                        "`{name}` already exists in ~/.agents/skills without a matching AGS ownership marker; refusing to replace it"
                    ),
                ));
            }
        }

        let staging = staging_root.join(&name);
        if fs::symlink_metadata(&staging).is_ok() {
            remove_tree_entry(&staging)?;
        }
        copy_dir(&from, &staging)?;
        fs::write(
            staging.join(".ags-skill-version"),
            format!("ags-official:{name}:{AGS_VERSION}\n"),
        )
        .map_err(|e| crate::error::io("skill_stamp_failed", &e))?;

        if fs::symlink_metadata(&target).is_ok() {
            remove_tree_entry(&target)?;
        }
        fs::rename(&staging, &target).map_err(|e| crate::error::io("skill_replace_failed", &e))?;
        wrote.push(name);
    }
    let _ = fs::remove_dir(&staging_root);
    Ok(wrote)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| crate::error::io("skill_dir_failed", &e))?;
    for entry in fs::read_dir(from).map_err(|e| crate::error::io("skill_scan_failed", &e))? {
        let entry = entry.map_err(|e| crate::error::io("skill_scan_failed", &e))?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if directory_link_target(&src)
            .map_err(|e| crate::error::io("skill_scan_failed", &e))?
            .is_some()
        {
            return Err(Error::new(
                "skill_symlink_refused",
                format!(
                    "official skill source contains a directory link: {}",
                    src.display()
                ),
            ));
        }
        if entry
            .file_type()
            .map_err(|e| crate::error::io("skill_scan_failed", &e))?
            .is_dir()
        {
            copy_dir(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).map_err(|e| crate::error::io("skill_copy_failed", &e))?;
        }
    }
    Ok(())
}

/// Compare product-owned source files directly. No second digest or lock is
/// created; this only prevents a same-version stale body from looking healthy.
fn source_tree_matches(from: &Path, to: &Path) -> bool {
    let Ok(entries) = fs::read_dir(from) else {
        return false;
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let Ok(kind) = entry.file_type() else {
            return false;
        };
        if directory_link_target(&src)
            .map(|target| target.is_some())
            .unwrap_or(true)
        {
            return false;
        }
        if kind.is_dir() {
            if !source_tree_matches(&src, &dst) {
                return false;
            }
        } else if fs::read(&src).ok() != fs::read(&dst).ok() {
            return false;
        }
    }
    true
}

// ── drift reporting ─────────────────────────────────────────────────────

/// Everything `ags doctor` flags when the entry ecosystem falls behind.
pub fn drift_report() -> Result<Vec<String>> {
    let mut findings = Vec::new();
    let registry = load_registry()?;
    for project in &registry.projects {
        for name in ENTRY_FILES {
            let path = project.join(name);
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let block_ok = text.contains(BLOCK_BEGIN)
                && text
                    .find(BLOCK_END)
                    .map(|end| text[..end].contains(&format!("AGS {AGS_VERSION}")))
                    .unwrap_or(false);
            if !block_ok {
                findings.push(format!(
                    "entry drift: {} missing the v{AGS_VERSION} managed block",
                    path.display()
                ));
            }
        }
    }
    let rules = rules_dir()?;
    let rules_ok = fs::read_to_string(rules.join("ags-core.md"))
        .map(|t| t.contains(&format!("v{AGS_VERSION}")))
        .unwrap_or(false);
    if !rules_ok {
        findings.push(format!("rules drift: ags-core.md is not at v{AGS_VERSION}"));
    }
    let skills = skills_dir()?;
    // Drift only applies to product-owned skills present in the installed
    // source checkout. Check source bytes as well as the display stamp so a
    // same-version fix cannot remain silently stale.
    let owned_root = install_info()?.source_root.join("ags-skills");
    if let Ok(entries) = fs::read_dir(&owned_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("ags-") || !entry.path().is_dir() {
                continue;
            }
            let installed = skills.join(&name);
            if !official_skill_owned(&installed, &name)
                || !source_tree_matches(&entry.path(), &installed)
            {
                findings.push(format!(
                    "skill drift: {name} differs from the installed source; run `ags update`"
                ));
            }
        }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn block_replace_keeps_user_content() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("AGENTS.md");
        fs::write(
            &file,
            "# My Project\nuser content\n<!-- BEGIN AGS MANAGED BLOCK -->\nold block\n<!-- END AGS MANAGED BLOCK -->\ntail\n",
        )
        .unwrap();
        sync_entry_block(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("# My Project\nuser content\n"));
        assert!(text.contains(&format!("AGS {AGS_VERSION}")));
        assert!(!text.contains("old block"));
        assert!(text.contains("tail\n"));
    }

    #[test]
    fn entry_clean_smudge_roundtrip_preserves_user_content() {
        for base in [
            "",
            "# Project",
            "# Project\n",
            "# Project\n\n",
            "# Project\nuser line\n",
        ] {
            let projected = render_entry_text(base);
            assert!(projected.contains(BLOCK_BEGIN));
            assert_eq!(strip_entry_text(&projected), base);
            assert_eq!(strip_entry_text(base), base);
        }

        let with_tail = format!("# Project\n{}\ntail line\n", entry_block().trim_end());
        assert_eq!(strip_entry_text(&with_tail), "# Project\ntail line\n");
    }

    #[test]
    fn current_projection_replaces_unmarked_legacy_sections() {
        let legacy_only = "# Project\n\n## Agent Governance Suite\n\nOld AGS 0.4.16 text.\n";
        let projected = render_entry_text(legacy_only);
        assert!(projected.contains(&format!("AGS {AGS_VERSION}")));
        assert!(!projected.contains("Old AGS 0.4.16 text."));
        assert_eq!(strip_entry_text(&projected), "# Project\n");

        let legacy_middle = "# Project\n\n## Agent Governance Suite\nold\n\n## Verify\nrun tests\n";
        let projected = render_entry_text(legacy_middle);
        assert_eq!(
            strip_entry_text(&projected),
            "# Project\n\n## Verify\nrun tests\n"
        );

        let duplicated = format!("{}\n{}", entry_block(), legacy_only);
        let projected = render_entry_text(&duplicated);
        assert_eq!(projected.matches(BLOCK_BEGIN).count(), 1);
        assert!(!projected.contains("Old AGS 0.4.16 text."));
    }

    #[test]
    fn block_appends_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("CLAUDE.md");
        fs::write(&file, "user content\n").unwrap();
        sync_entry_block(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.ends_with("user content\n"));
        assert!(text.contains(BLOCK_BEGIN));
        assert!(text.contains(&format!("AGS {AGS_VERSION}")));
    }

    #[test]
    fn protocol_copy_only_refreshes_owned() {
        let tmp = tempfile::tempdir().unwrap();
        let owned = tmp.path().join("AGENT_SUITE_PROTOCOL.md");
        fs::write(
            &owned,
            "# Agent Governance Suite Protocol\n\nold version text\n",
        )
        .unwrap();
        assert!(sync_protocol_copy(tmp.path()).unwrap());
        assert!(fs::read_to_string(&owned).unwrap().contains("**v0.4.21**"));

        fs::write(
            &owned,
            "# AGENT_SUITE_PROTOCOL.md\n\nThis project is integrated with Agent Governance Suite 0.4.16.\n",
        )
        .unwrap();
        assert!(sync_protocol_copy(tmp.path()).unwrap());
        assert!(fs::read_to_string(&owned).unwrap().contains("**v0.4.21**"));
        let user = tmp.path().join("user.md");
        fs::write(&user, "# Not AGS\n").unwrap();
        fs::write(tmp.path().join("AGENT_SUITE_PROTOCOL.md"), "# User Doc\n").unwrap();
        assert!(!sync_protocol_copy(tmp.path()).unwrap());
    }

    #[test]
    fn registry_roundtrip() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let reg = ManagedRegistry {
            schema_version: "ags://schema/contract/v3/managed-registry".to_string(),
            projects: vec![PathBuf::from("/tmp/x")],
        };
        save_registry(&reg).unwrap();
        let loaded = load_registry().unwrap();
        assert_eq!(loaded.projects, reg.projects);
    }
}

// ── machine-level capability bodies ──────────────────────────────────────

/// Machine lock path: `~/.ags/v3/capabilities.json`.
pub const MACHINE_LOCK_PATH: &str = ".ags/v3/capabilities.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineEntry {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MachineLock {
    pub schema_version: String,
    pub entries: Vec<MachineEntry>,
    /// Persistent body problems (dangling symlinks, unhashable bodies) so
    /// doctor keeps reporting them between updates.
    pub problems: Vec<String>,
}

fn machine_lock_path() -> Result<PathBuf> {
    Ok(runtime_home()?.join(MACHINE_LOCK_PATH))
}

pub fn load_machine_lock() -> Result<MachineLock> {
    let path = machine_lock_path()?;
    if !path.is_file() {
        return Ok(MachineLock::default());
    }
    let text =
        fs::read_to_string(&path).map_err(|e| crate::error::io("machine_lock_read_failed", &e))?;
    serde_json::from_str(&text).map_err(|e| Error::new("machine_lock_parse_failed", e.to_string()))
}

/// Pin every skill body under `~/.agents/skills/` into the machine lock.
/// Top-level symlinks are resolved and the real body hashed; dangling or
/// unhashable bodies are reported as problems, never silently dropped.
pub fn sync_bodies() -> Result<(MachineLock, Vec<String>)> {
    let dir = skills_dir()?;
    let mut entries: Vec<MachineEntry> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).map_err(|e| crate::error::io("skills_scan_failed", &e))? {
            let entry = entry.map_err(|e| crate::error::io("skills_scan_failed", &e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            let is_symlink = directory_link_target(&path)
                .map(|target| target.is_some())
                .unwrap_or(false);
            let resolved = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !resolved.is_dir() {
                if is_symlink {
                    problems.push(format!("{name}: dangling symlink"));
                }
                continue; // loose files are not bodies
            }
            match crate::capabilities::dir_sha256(&resolved) {
                Ok(sha256) => entries.push(MachineEntry {
                    id: name.clone(),
                    kind: "skill".to_string(),
                    path: resolved.to_string_lossy().to_string(),
                    sha256,
                }),
                Err(e) => problems.push(format!("{name}: {}", e.message)),
            }
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    let lock = MachineLock {
        schema_version: "ags://schema/contract/v3/machine-capabilities".to_string(),
        entries,
        problems: problems.clone(),
    };
    let path = machine_lock_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| crate::error::io("registry_dir_failed", &e))?;
    }
    let text = serde_json::to_string_pretty(&lock)
        .map_err(|e| Error::new("machine_lock_encode_failed", e.to_string()))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| crate::error::io("machine_lock_write_failed", &e))?;
    fs::rename(&tmp, &path).map_err(|e| crate::error::io("machine_lock_write_failed", &e))?;
    Ok((lock, problems))
}

/// Third-party body integrity: every pinned entry must still resolve to the
/// pinned hash. This is the governance that was missing for legacy installed
/// bodies (superpowers etc.) — tamper or drift is a hard doctor finding.
pub fn bodies_drift() -> Result<Vec<String>> {
    let lock = load_machine_lock()?;
    let mut findings = lock.problems.clone();
    for entry in &lock.entries {
        let resolved = Path::new(&entry.path);
        if !resolved.exists() {
            findings.push(format!(
                "third-party body missing: {} ({})",
                entry.id, entry.path
            ));
            continue;
        }
        match crate::capabilities::dir_sha256(resolved) {
            Ok(sha) if sha == entry.sha256 => {}
            Ok(_) => findings.push(format!("third-party body drift: {}", entry.id)),
            Err(e) => findings.push(format!(
                "third-party body unhashable: {} ({})",
                entry.id, e.message
            )),
        }
    }
    Ok(findings)
}

/// Host skill directories whose bodies are owned by the host ecosystem, not
/// by AGS. `install` treats a skill found ONLY in these dirs as not-installed
/// (host-owned), so third-party detection stays pure.
pub const HOST_SKILL_DIRS: &[&str] = &[
    ".claude/skills",
    ".codex/skills",
    ".cursor/skills",
    ".codebuddy/skills",
];

/// Detect where a skill id already lives on this machine.
///
/// Returns (ags_installed, host_locations): `ags_installed` is true when the
/// machine lock pins the id (body present under `~/.agents/skills/`);
/// `host_locations` lists host-owned directories (relative to HOME) that also
/// carry the id — informative only, they never count as installed.
pub fn detect_skill_anywhere(id: &str) -> Result<(bool, Vec<String>)> {
    let home = runtime_home()?;
    let skills = skills_dir()?;
    let mut host_locations = Vec::new();
    // Host-owned dirs never count as installed (pure third-party rule).
    for rel in HOST_SKILL_DIRS {
        let dir = home.join(rel);
        if dir.join(id).join("SKILL.md").is_file() {
            host_locations.push(rel.to_string());
        }
    }
    // AGS-installed = pinned in the machine lock with the body present.
    let lock = load_machine_lock()?;
    let ags_installed = lock
        .entries
        .iter()
        .find(|e| e.id == id)
        .map(|e| Path::new(&e.path).is_dir() && skills.join(id).exists())
        .unwrap_or(false);
    Ok((ags_installed, host_locations))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillRouting {
    Routable,
    HostOnly,
}

impl SkillRouting {
    pub fn label(self) -> &'static str {
        match self {
            Self::Routable => "routable",
            Self::HostOnly => "host-only",
        }
    }
}

/// Validate the semantic shape needed by both hosts and AGS routing. This is
/// deliberately small: one SKILL.md, matching name; triggers are optional
/// because a host may still select the skill from its description.
pub fn validate_skill_source(id: &str, source: &Path) -> Result<SkillRouting> {
    crate::skill_adoption::validate_skill_id(id)?;
    let skill_md = source.join("SKILL.md");
    let text = fs::read_to_string(&skill_md).map_err(|e| {
        Error::new(
            "skill_source_invalid",
            format!("{} must contain a readable SKILL.md: {e}", source.display()),
        )
    })?;
    let fm = crate::route::parse_skill_frontmatter(&text);
    match fm.name.as_deref() {
        Some(name) if name == id => {}
        Some(name) => {
            return Err(Error::new(
                "skill_name_mismatch",
                format!("requested skill id `{id}` but SKILL.md declares `{name}`"),
            ));
        }
        None => {
            return Err(Error::new(
                "skill_name_missing",
                "SKILL.md frontmatter must declare `name`",
            ));
        }
    }
    Ok(if fm.triggers.is_empty() {
        SkillRouting::HostOnly
    } else {
        SkillRouting::Routable
    })
}

/// Materialize a third-party skill body into `~/.agents/skills/<id>` as a
/// symlink to the given source dir, then refresh the machine lock so routing
/// verifies immediately. Refuses to overwrite an existing valid body with a
/// different body. An explicit reinstall repairs a dangling symlink and an
/// idempotent reinstall refreshes a missing/stale machine lock.
pub fn install_skill_body(id: &str, source: &Path) -> Result<SkillRouting> {
    let routing = validate_skill_source(id, source)?;
    let canonical = fs::canonicalize(source).map_err(|e| {
        Error::new(
            "skill_source_missing",
            format!("cannot resolve skill source {}: {e}", source.display()),
        )
    })?;
    let skills = skills_dir()?;
    fs::create_dir_all(&skills).map_err(|e| crate::error::io("skills_dir_failed", &e))?;
    let target = skills.join(id);
    if fs::symlink_metadata(&target).is_ok() {
        let is_link = directory_link_target(&target)
            .map_err(|e| crate::error::io("skill_symlink_repair_failed", &e))?
            .is_some();
        if is_link && fs::canonicalize(&target).is_err() {
            remove_directory_link(&target)
                .map_err(|e| crate::error::io("skill_symlink_repair_failed", &e))?;
        } else {
            let current_source = fs::canonicalize(&target)
                .map_err(|e| crate::error::io("skill_body_resolve_failed", &e))?;
            if current_source != canonical {
                let current = crate::capabilities::dir_sha256(&target)?;
                let incoming = crate::capabilities::dir_sha256(&canonical)?;
                if current != incoming {
                    return Err(Error::new(
                        "skill_body_conflict",
                        format!(
                            "`{id}` already exists in ~/.agents/skills with a different body; refusing to overwrite"
                        ),
                    ));
                }
            }
            sync_bodies()?;
            return Ok(routing);
        }
    }
    create_directory_link(&canonical, &target)
        .map_err(|e| crate::error::io("skill_symlink_failed", &e))?;
    sync_bodies()?;
    Ok(routing)
}

#[cfg(unix)]
pub(crate) fn directory_link_target(path: &Path) -> std::io::Result<Option<PathBuf>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::read_link(path).map(Some),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
pub(crate) fn directory_link_target(path: &Path) -> std::io::Result<Option<PathBuf>> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::read_link(path).map(Some),
        Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
            junction::get_target(path).map(Some)
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
pub(crate) fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub(crate) fn create_directory_link(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}

#[cfg(unix)]
pub(crate) fn remove_directory_link(link: &Path) -> std::io::Result<()> {
    fs::remove_file(link)
}

#[cfg(windows)]
pub(crate) fn remove_directory_link(link: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let metadata = fs::symlink_metadata(link)?;
    if metadata.file_type().is_symlink() {
        fs::remove_dir(link)
    } else if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        junction::get_target(link)?;
        junction::delete(link)?;
        fs::remove_dir(link)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to remove unmanaged directory: {}", link.display()),
        ))
    }
}

/// Uninstall one AGS-managed third-party body. Product-owned and unmanaged
/// real directories are never recursively deleted by this command.
pub fn remove_skill_body(id: &str) -> Result<bool> {
    let target = skills_dir()?.join(id);
    if fs::symlink_metadata(&target).is_err() {
        sync_bodies()?;
        return Ok(false);
    }
    if directory_link_target(&target)
        .map_err(|e| crate::error::io("skill_remove_failed", &e))?
        .is_none()
    {
        return Err(Error::new(
            "skill_body_not_managed_symlink",
            format!(
                "`{id}` is a real directory in ~/.agents/skills; AGS only uninstalls symlinks it manages"
            ),
        ));
    }
    remove_directory_link(&target).map_err(|e| crate::error::io("skill_remove_failed", &e))?;
    sync_bodies()?;
    Ok(true)
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use std::fs;

    fn home_with_skills(tmp: &tempfile::TempDir) {
        let home = tmp.path();
        let skills = home.join(".agents/skills");
        fs::create_dir_all(&skills).unwrap();
        for (name, content) in [("third-party-a", "# A\n"), ("third-party-b", "# B\n")] {
            fs::create_dir_all(skills.join(name)).unwrap();
            fs::write(skills.join(name).join("SKILL.md"), content).unwrap();
        }
        std::env::set_var("HOME", home);
    }

    #[test]
    fn official_skills_refresh_even_with_same_version_stamp() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let source = tmp.path().join("source/ags-skills/ags-demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: ags-demo\ndescription: first\n---\n",
        )
        .unwrap();
        sync_skills(&tmp.path().join("source")).unwrap();
        let installed = tmp.path().join(".agents/skills/ags-demo/SKILL.md");
        assert!(fs::read_to_string(&installed).unwrap().contains("first"));
        // Legacy official installs stored only the version; name + non-empty
        // marker migrates safely to the stronger marker format.
        fs::write(
            installed.parent().unwrap().join(".ags-skill-version"),
            AGS_VERSION,
        )
        .unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: ags-demo\ndescription: same-version-fix\n---\n",
        )
        .unwrap();
        sync_skills(&tmp.path().join("source")).unwrap();
        assert!(fs::read_to_string(&installed)
            .unwrap()
            .contains("same-version-fix"));
        assert!(
            fs::read_to_string(installed.parent().unwrap().join(".ags-skill-version"))
                .unwrap()
                .starts_with("ags-official:ags-demo:")
        );
    }

    #[test]
    fn setup_installs_record_and_official_skills() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("AGS_SOURCE_ROOT");
        let source = tmp.path().join("source");
        fs::create_dir_all(source.join("ags-skills/ags-demo")).unwrap();
        fs::write(
            source.join("ags-skills/ags-demo/SKILL.md"),
            "---\nname: ags-demo\ndescription: Official.\n---\n",
        )
        .unwrap();
        let wrote = setup(&source).unwrap();
        assert!(wrote.iter().any(|w| w == "ags-demo"));
        assert_eq!(
            install_info().unwrap().source_root,
            source.canonicalize().unwrap()
        );
        assert!(tmp
            .path()
            .join(".agents/skills/ags-demo/SKILL.md")
            .is_file());
    }

    #[test]
    fn install_info_without_record_fails_closed() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("AGS_SOURCE_ROOT");
        let err = install_info().unwrap_err();
        assert_eq!(err.code, "install_info_missing");
        // setup from a source without ags-skills/ must also fail closed.
        let bad = tmp.path().join("not-a-checkout");
        fs::create_dir_all(&bad).unwrap();
        let err = setup(&bad).unwrap_err();
        assert_eq!(err.code, "setup_source_invalid");
    }

    #[test]
    fn official_sync_recovers_stale_staging_and_dangling_link() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let source = tmp.path().join("source/ags-skills/ags-demo");
        let staging = tmp.path().join(".agents/.ags-skill-staging/ags-demo");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&staging).unwrap();
        let target = tmp.path().join(".agents/skills/ags-demo");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("missing"), &target).unwrap();
        fs::write(staging.join("partial"), "stale").unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: ags-demo\ndescription: Demo.\n---\n# Official\n",
        )
        .unwrap();
        sync_skills(&tmp.path().join("source")).unwrap();
        assert!(tmp
            .path()
            .join(".agents/skills/ags-demo/SKILL.md")
            .is_file());
        assert!(!tmp.path().join(".agents/.ags-skill-staging").exists());
    }

    #[test]
    fn official_sync_refuses_unmanaged_name_conflict() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let source = tmp.path().join("source/ags-skills/ags-demo");
        let occupied = tmp.path().join(".agents/skills/ags-demo");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&occupied).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: ags-demo\ndescription: Demo.\n---\n# Official\n",
        )
        .unwrap();
        fs::write(occupied.join("SKILL.md"), "# User\n").unwrap();
        // A filename alone is not enough to claim product ownership.
        fs::write(occupied.join(".ags-skill-version"), AGS_VERSION).unwrap();
        let err = sync_skills(&tmp.path().join("source")).unwrap_err();
        assert_eq!(err.code, "official_skill_conflict");
        assert_eq!(
            fs::read_to_string(occupied.join("SKILL.md")).unwrap(),
            "# User\n"
        );
    }

    #[test]
    fn machine_lock_pins_and_detects_drift() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        home_with_skills(&tmp);
        let (lock, problems) = sync_bodies().unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let ids: Vec<&str> = lock.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"third-party-a"));
        assert!(ids.contains(&"third-party-b"));
        assert!(bodies_drift().unwrap().is_empty());
        // tamper a body → drift is a hard finding
        fs::write(
            tmp.path().join(".agents/skills/third-party-a/SKILL.md"),
            "# A tampered\n",
        )
        .unwrap();
        let drift = bodies_drift().unwrap();
        assert!(
            drift.iter().any(|f| f.contains("third-party-a")),
            "{drift:?}"
        );
    }

    #[test]
    fn reinstall_repairs_machine_lock_and_dangling_symlink() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let source = tmp.path().join("source/demo");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo need\n---\n",
        )
        .unwrap();
        assert_eq!(
            install_skill_body("demo", &source).unwrap(),
            SkillRouting::Routable
        );
        fs::remove_file(machine_lock_path().unwrap()).unwrap();
        assert_eq!(
            install_skill_body("demo", &source).unwrap(),
            SkillRouting::Routable
        );
        assert!(load_machine_lock()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.id == "demo"));
        fs::remove_file(tmp.path().join(".agents/skills/demo")).unwrap();
        std::os::unix::fs::symlink(
            tmp.path().join("missing"),
            tmp.path().join(".agents/skills/demo"),
        )
        .unwrap();
        assert_eq!(
            install_skill_body("demo", &source).unwrap(),
            SkillRouting::Routable
        );
        assert!(tmp.path().join(".agents/skills/demo/SKILL.md").is_file());
        assert!(remove_skill_body("demo").unwrap());
        assert!(fs::symlink_metadata(tmp.path().join(".agents/skills/demo")).is_err());
    }

    #[test]
    fn dangling_symlink_is_a_problem_not_silent() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        home_with_skills(&tmp);
        std::os::unix::fs::symlink(
            tmp.path().join("does-not-exist"),
            tmp.path().join(".agents/skills/broken-body"),
        )
        .unwrap();
        let (_lock, problems) = sync_bodies().unwrap();
        assert!(
            problems.iter().any(|p| p.contains("broken-body")),
            "{problems:?}"
        );
    }
}
