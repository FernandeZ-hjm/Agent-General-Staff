//! Managed-project refresh planning and atomic apply.

use super::model::InitFile;
use super::plan::{project_init_plan_with_protocol, ProjectInitPlan};
use super::{InitFinding, InitReport};
use std::path::{Path, PathBuf};
use std::process::Command;

const AGS_MANAGED_BEGIN: &str = "<!-- BEGIN AGS MANAGED BLOCK -->";
const AGS_MANAGED_END: &str = "<!-- END AGS MANAGED BLOCK -->";

fn managed_block_text(desired: &str) -> String {
    format!("{AGS_MANAGED_BEGIN}\n{}\n{AGS_MANAGED_END}", desired.trim())
}

/// Replace only the explicitly marked AGS-owned section of a user-owned entry
/// file. Files without a managed block receive one; generated full-entry files
/// are handled separately by `is_generated_full_entry`.
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

#[derive(Debug, Clone)]
pub struct ManagedProjectRegistration {
    pub registry_path: PathBuf,
    pub project_path: String,
    pub change: &'static str,
}

pub(crate) fn register_managed_project(
    runtime_home: &Path,
    target: &Path,
    slug: &str,
    now: u64,
    report: &mut InitReport,
) -> Option<ManagedProjectRegistration> {
    use ags_workspace_facts::managed_projects as registry;

    let registry_path = registry::registry_path(runtime_home);
    let mut managed_projects = match registry::load(&registry_path) {
        Ok(registry) => registry,
        Err(error) => {
            report.add(InitFinding::warn(
                "managed-project-registry",
                "managed-projects.yaml is malformed; reporting drift instead of overwriting",
                error,
            ));
            return None;
        }
    };
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let project_path = canonical.display().to_string();
    let is_git = Command::new("git")
        .arg("-C")
        .arg(&canonical)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let origin = is_git
        .then(|| {
            Command::new("git")
                .arg("-C")
                .arg(&canonical)
                .args(["remote", "get-url", "origin"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .flatten();
    let vcs = if is_git {
        registry::ProjectVcs::Git
    } else {
        registry::ProjectVcs::None
    };
    let entry =
        registry::describe_project(project_path.clone(), slug.to_string(), now, vcs, origin);
    let change = registry::upsert(&mut managed_projects, entry);
    if change == registry::RegistryChange::Unchanged {
        report.add(InitFinding::pass(
            "managed-project-registry",
            format!("already registered: {project_path}"),
        ));
        return None;
    }
    if let Some(parent) = registry_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&registry_path, registry::render_yaml(&managed_projects)) {
        report.add(InitFinding::warn(
            "managed-project-registry",
            "could not write managed-projects.yaml",
            error.to_string(),
        ));
        return None;
    }
    let change = if change == registry::RegistryChange::Added {
        "added"
    } else {
        "refreshed"
    };
    report.add(InitFinding::pass(
        "managed-project-registry",
        format!("registered in managed-projects.yaml: {project_path} ({change})"),
    ));
    Some(ManagedProjectRegistration {
        registry_path,
        project_path,
        change,
    })
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
        } else if super::plan::append_content_present(text, &append.content) {
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
    approved_hosts: &[String],
    apply: bool,
) -> ManagedProjectRefresh {
    let canonical = ags_platform::normalize_path(target);
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

    let mut lifecycle_pending = Vec::new();
    for host in approved_hosts {
        match crate::lifecycle_projection::LifecycleProjection::new(&canonical, host) {
            Ok(projection) if projection.observe().current => {
                unchanged.push(projection.path().display().to_string())
            }
            Ok(projection) => {
                lifecycle_pending.push((host.clone(), projection.path()));
            }
            Err(error) => blocked.push(error),
        }
    }
    let changed_files: Vec<String> = pending
        .iter()
        .map(|write| write.path.display().to_string())
        .chain(
            lifecycle_pending
                .iter()
                .map(|(_, path)| path.display().to_string()),
        )
        .collect();
    let drift = !pending.is_empty() || !lifecycle_pending.is_empty() || !blocked.is_empty();
    if !apply || !blocked.is_empty() {
        return ManagedProjectRefresh {
            target: canonical.display().to_string(),
            slug: slug.to_string(),
            status: if !blocked.is_empty() {
                "blocked"
            } else if pending.is_empty() && lifecycle_pending.is_empty() {
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
                "write failed {}: {e}; this apply was restored to its original state",
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
    for (host, _) in &lifecycle_pending {
        if let Err(error) = crate::lifecycle_projection::LifecycleProjection::new(&canonical, host)
            .and_then(|projection| projection.install().map(|_| ()))
        {
            blocked.push(format!("{host} lifecycle projection failed: {error}"));
        }
    }
    if blocked.is_empty() && !lifecycle_pending.is_empty() {
        if let Err(error) = crate::lifecycle_projection::record_lifecycle_manifest(&canonical) {
            blocked.push(format!("lifecycle manifest update failed: {error}"));
        }
    }
    if !blocked.is_empty() {
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

    ManagedProjectRefresh {
        target: canonical.display().to_string(),
        slug: slug.to_string(),
        status: if pending.is_empty() && lifecycle_pending.is_empty() {
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
