//! Managed-project refresh planning, apply, and rollback.

use super::model::InitFile;
use super::plan::{guard_path, project_init_plan_with_protocol, ProjectInitPlan};
use std::path::{Path, PathBuf};

const AGS_MANAGED_BEGIN: &str = "<!-- BEGIN AGS MANAGED BLOCK -->";
const AGS_MANAGED_END: &str = "<!-- END AGS MANAGED BLOCK -->";

fn managed_block_text(desired: &str) -> String {
    format!("{AGS_MANAGED_BEGIN}\n{}\n{AGS_MANAGED_END}", desired.trim())
}

/// Replace only the AGS-owned section of a user-owned entry file. Legacy
/// unmarked sections are migrated once; ambiguous project-owned sections fail
/// closed instead of being overwritten.
pub(crate) fn merge_managed_project_block(existing: &str, desired: &str) -> Result<String, String> {
    let replacement = managed_block_text(desired);
    if let Some(begin) = existing.find(AGS_MANAGED_BEGIN) {
        let Some(end_rel) = existing[begin..].find(AGS_MANAGED_END) else {
            return Err("AGS managed block begin marker has no end marker".to_string());
        };
        let end = begin + end_rel + AGS_MANAGED_END.len();
        return Ok(format!(
            "{}{}{}",
            &existing[..begin],
            replacement,
            &existing[end..]
        ));
    }

    let heading = "## Agent Governance Suite";
    if let Some(begin) = existing.find(heading) {
        let section_tail = &existing[begin + heading.len()..];
        let end = section_tail
            .find("\n## ")
            .map(|offset| begin + heading.len() + offset)
            .unwrap_or(existing.len());
        let legacy = &existing[begin..end];
        if !legacy.contains("This project is governed by AGS")
            && !legacy.contains("This project is governed by Agent Governance Suite")
        {
            return Err("existing Agent Governance Suite section is not AGS-managed".to_string());
        }
        return Ok(format!(
            "{}{}{}",
            &existing[..begin],
            replacement,
            &existing[end..]
        ));
    }

    let separator = if existing.is_empty() || existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!("{existing}{separator}{replacement}\n"))
}
#[derive(Debug, Clone)]
pub struct ManagedProjectRefresh {
    pub target: String,
    pub slug: String,
    pub status: String,
    pub drift: bool,
    pub changed_files: Vec<String>,
    pub unchanged_files: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

struct PendingProjectWrite {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    mode: Option<u32>,
}

fn is_project_memory_file(plan: &ProjectInitPlan, path: &Path) -> bool {
    path.starts_with(&plan.memory_dir)
}

fn is_entry_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("AGENTS.md" | "CLAUDE.md")
    )
}

fn is_generated_full_entry(path: &Path, content: &str) -> bool {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("CLAUDE.md") => {
            content.starts_with("# CLAUDE.md\n\n@AGENTS.md\n\n## Agent Governance Suite")
                || content.starts_with(
                    "# CLAUDE.md\n\nThis project is governed by Agent Governance Suite",
                )
        }
        Some("AGENTS.md") => {
            content.starts_with("# AGENTS.md\n\n## Agent Governance Suite")
                || content.starts_with("# AGENTS.md\n\n@CLAUDE.md")
        }
        _ => false,
    }
}

pub(crate) fn desired_project_file_content(
    plan: &ProjectInitPlan,
    file: &InitFile,
) -> Result<Option<Vec<u8>>, String> {
    let existing = match std::fs::read(&file.path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", file.path.display())),
    };
    if is_project_memory_file(plan, &file.path) && existing.is_some() {
        return Ok(None);
    }
    let Some(before) = existing.as_deref() else {
        return Ok(Some(file.content.as_bytes().to_vec()));
    };

    let desired = if let Some(append) = plan
        .append_files
        .iter()
        .find(|candidate| candidate.path == file.path)
    {
        let text = std::str::from_utf8(before)
            .map_err(|e| format!("{} is not UTF-8: {e}", file.path.display()))?;
        if is_entry_file(&file.path) {
            if is_generated_full_entry(&file.path, text) {
                file.content.clone()
            } else {
                merge_managed_project_block(text, &append.content)?
            }
        } else if text.contains(append.content.trim()) {
            text.to_string()
        } else {
            format!("{}{}", text, append.content)
        }
    } else {
        file.content.clone()
    };
    if desired.as_bytes() == before {
        Ok(None)
    } else {
        Ok(Some(desired.into_bytes()))
    }
}

/// Inspect or refresh one registered project through a single deep interface.
/// User-owned entry files are changed only inside the AGS managed section;
/// project memory is create-only; AGS-owned protocol/template files are exact.
pub fn refresh_managed_project(
    target: &Path,
    slug: &str,
    source_root: &Path,
    apply: bool,
) -> ManagedProjectRefresh {
    let canonical = guard_path(target);
    if ags_workspace_facts::detect_project(&canonical).is_ags_suite {
        return ManagedProjectRefresh {
            target: canonical.display().to_string(),
            slug: slug.to_string(),
            status: "suite-authority".to_string(),
            drift: false,
            changed_files: Vec::new(),
            unchanged_files: Vec::new(),
            blocked_reasons: Vec::new(),
        };
    }

    let plan = project_init_plan_with_protocol(
        &canonical,
        Some(slug.to_string()),
        Some(source_root.join("protocol")),
    );
    let mut pending = Vec::new();
    let mut unchanged = Vec::new();
    let mut blocked = plan.warnings.clone();
    for file in &plan.files {
        match desired_project_file_content(&plan, file) {
            Ok(Some(after)) => {
                let before = std::fs::read(&file.path).ok();
                pending.push(PendingProjectWrite {
                    path: file.path.clone(),
                    before,
                    after,
                    mode: file.mode,
                });
            }
            Ok(None) => unchanged.push(file.path.display().to_string()),
            Err(e) => blocked.push(e),
        }
    }

    let changed_files: Vec<String> = pending
        .iter()
        .map(|write| write.path.display().to_string())
        .collect();
    let drift = !pending.is_empty() || !blocked.is_empty();
    if !apply || !blocked.is_empty() {
        return ManagedProjectRefresh {
            target: canonical.display().to_string(),
            slug: slug.to_string(),
            status: if !blocked.is_empty() {
                "blocked"
            } else if pending.is_empty() {
                "clean"
            } else {
                "planned"
            }
            .to_string(),
            drift,
            changed_files,
            unchanged_files: unchanged,
            blocked_reasons: blocked,
        };
    }

    let mut applied: Vec<&PendingProjectWrite> = Vec::new();
    for write in &pending {
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = write.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&write.path, &write.after)?;
            let _requested_mode = write.mode;
            #[cfg(unix)]
            if let Some(mode) = _requested_mode {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&write.path)?.permissions();
                permissions.set_mode(mode);
                std::fs::set_permissions(&write.path, permissions)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            for previous in applied.iter().rev() {
                if let Some(before) = &previous.before {
                    let _ = std::fs::write(&previous.path, before);
                } else {
                    let _ = std::fs::remove_file(&previous.path);
                }
            }
            blocked.push(format!(
                "write failed {}: {e}; prior writes rolled back",
                write.path.display()
            ));
            return ManagedProjectRefresh {
                target: canonical.display().to_string(),
                slug: slug.to_string(),
                status: "failed".to_string(),
                drift: true,
                changed_files,
                unchanged_files: unchanged,
                blocked_reasons: blocked,
            };
        }
        applied.push(write);
    }

    ManagedProjectRefresh {
        target: canonical.display().to_string(),
        slug: slug.to_string(),
        status: if pending.is_empty() {
            "clean"
        } else {
            "applied"
        }
        .to_string(),
        drift: false,
        changed_files,
        unchanged_files: unchanged,
        blocked_reasons: Vec::new(),
    }
}
