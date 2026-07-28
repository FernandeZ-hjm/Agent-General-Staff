use super::*;

// ── AGS project-memory capture chain checks ────────────────────────────────
//
// These diagnose the context-memory product mechanism restored as a first-class
// `ags setup` deliverable: start/capture scripts installed, the Claude
// SessionStart pipeline wired with read-only project-memory injection, the
// Claude Stop pipeline wired with the project-memory capture step, the raw
// guard preserved, task-memory refreshable, and the manual capsule
// design-purpose block intact. All are
// advisory (Warn/Skip) so a
// not-yet-bootstrapped machine reports the gap without blocking the gate.

const RAW_GUARD_MARKER: &str = "host lifecycle --event stop-guard";

/// Project slug from `config/agent-project-profile.yaml` `project.slug`,
/// falling back to the repository directory name.
pub(super) fn project_slug(repo_root: &Path) -> String {
    let profile = repo_root.join("config/agent-project-profile.yaml");
    if let Ok(raw) = std::fs::read_to_string(&profile) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&raw) {
            if let Some(slug) = doc
                .get("project")
                .and_then(|p| p.get("slug"))
                .and_then(|s| s.as_str())
            {
                let slug = slug.trim();
                if !slug.is_empty() {
                    return slug.to_string();
                }
            }
        }
    }
    repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("project")
        .to_string()
}

pub(super) fn project_memory_dir(repo_root: &Path, home: &Path) -> PathBuf {
    home.join(".agents/memory/projects")
        .join(project_slug(repo_root))
}

/// Collect every hook command string from a settings.json event (nested
/// `{hooks:[{command}]}` and flat `{command}` group forms). Returns `None` when
/// the file is missing, unreadable, invalid JSON, or has no event array.
pub(super) fn hook_commands(settings_path: &Path, event_name: &str) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(settings_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let event = parsed.get("hooks")?.get(event_name)?.as_array()?;
    let mut cmds = Vec::new();
    for group in event {
        if let Some(arr) = group.get("hooks").and_then(|h| h.as_array()) {
            for h in arr {
                if let Some(c) = h.get("command").and_then(|c| c.as_str()) {
                    cmds.push(c.to_string());
                }
            }
        }
        if let Some(c) = group.get("command").and_then(|c| c.as_str()) {
            cmds.push(c.to_string());
        }
    }
    Some(cmds)
}

pub(super) fn stop_hook_commands(settings_path: &Path) -> Option<Vec<String>> {
    hook_commands(settings_path, "Stop")
}

pub(super) fn newest_subdir_mtime(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                newest = Some(newest.map_or(m, |n| n.max(m)));
            }
        }
    }
    newest
}

/// Check the only non-Rust lifecycle asset: OMP's required thin JS extension.
pub fn omp_lifecycle_extension_present() -> Finding {
    match ags_platform::home_dir() {
        Some(h) => omp_lifecycle_extension_present_at(&h),
        None => Finding::skip(
            "omp_lifecycle_extension_present",
            "home directory not set — cannot locate the OMP extension",
        ),
    }
}

pub(super) fn omp_lifecycle_extension_present_at(home: &Path) -> Finding {
    let check = "omp_lifecycle_extension_present";
    let omp = home.join(".omp/agent/extensions/ags-memory-lifecycle.js");
    if omp.is_file() {
        Finding::pass(
            check,
            format!("OMP thin lifecycle extension present at {}", omp.display()),
        )
    } else {
        Finding::warn(
            check,
            "OMP thin lifecycle extension is missing",
            "Run `ags setup --yes`, then `ags agents govern --apply` to install AGS-owned host memory adapters.",
        )
    }
}

/// Verify the local skill-body invariant: AGS-managed skills have one canonical
/// body, and host-visible entries are thin links to that body.
pub fn host_skill_body_singleton_check(repo_root: &Path) -> Finding {
    match ags_platform::home_dir() {
        Some(h) => host_skill_body_singleton_check_at(repo_root, &h),
        None => Finding::skip(
            "host-skill-body-singleton",
            "home directory not set — cannot inspect host skill roots",
        ),
    }
}

pub(super) fn expected_skill_body(
    repo_root: &Path,
    skill: &ags_capability_governance::skill_body::SkillEntry,
) -> Option<PathBuf> {
    if let Some(source) = &skill.source {
        return Some(repo_root.join(source));
    }
    let fallback = match skill.profile.as_str() {
        "required" => repo_root.join("global-skills").join(&skill.name),
        "optional" => repo_root.join("skill-packs/optional").join(&skill.name),
        "personal" => repo_root.join("skill-packs/personal").join(&skill.name),
        _ => return None,
    };
    Some(fallback)
}

pub(super) fn host_skill_roots(home: &Path) -> [PathBuf; 3] {
    [
        home.join(".agents/skills"),
        home.join(".codex/skills"),
        home.join(".claude/skills"),
    ]
}

pub(super) fn display_home_path(path: &Path, home: &Path) -> String {
    path.strip_prefix(home)
        .map(|p| format!("~/{}", p.display()))
        .unwrap_or_else(|_| path.display().to_string())
}

pub(super) fn same_private_stable_suite_path(real_entry: &Path, real_canonical: &Path) -> bool {
    let Some((entry_suite, entry_rel)) = split_suite_runtime_path(real_entry) else {
        return false;
    };
    let Some((canonical_suite, canonical_rel)) = split_suite_runtime_path(real_canonical) else {
        return false;
    };

    entry_suite != canonical_suite && entry_rel == canonical_rel
}

pub(super) fn split_suite_runtime_path(path: &Path) -> Option<(&'static str, PathBuf)> {
    const SUITE_PREFIX: &str = "agent-governance-suite-";
    const SOURCE_SUFFIX: &str = "private";
    const RUNTIME_SUFFIX: &str = "stable";

    let mut suite = None;
    let mut rel = PathBuf::new();

    for component in path.components() {
        if let Some(found) = suite {
            rel.push(component.as_os_str());
            suite = Some(found);
            continue;
        }
        let Some(name) = component.as_os_str().to_str() else {
            continue;
        };
        if let Some(suffix) = name.strip_prefix(SUITE_PREFIX) {
            if suffix == SOURCE_SUFFIX {
                suite = Some("source");
            } else if suffix == RUNTIME_SUFFIX {
                suite = Some("runtime");
            }
        }
    }

    match suite {
        Some(found) if !rel.as_os_str().is_empty() => Some((found, rel)),
        _ => None,
    }
}

pub(super) fn host_skill_body_singleton_check_at(repo_root: &Path, home: &Path) -> Finding {
    let check = "host-skill-body-singleton";
    let scan = ags_capability_governance::skill_body::scan_skills(repo_root);
    let mut mismatches: Vec<String> = Vec::new();
    let mut checked_entries = 0usize;

    for skill in &scan.skills {
        let Some(expected_raw) = expected_skill_body(repo_root, skill) else {
            continue;
        };
        let Ok(expected) = std::fs::canonicalize(&expected_raw) else {
            continue;
        };
        let shared_body_root = home.join(".agents/skills");
        let mut targets = std::collections::BTreeMap::<PathBuf, Vec<String>>::new();
        let mut skill_issues: Vec<String> = Vec::new();
        for root in host_skill_roots(home) {
            let entry = root.join(&skill.name);
            let Ok(meta) = std::fs::symlink_metadata(&entry) else {
                continue;
            };
            checked_entries += 1;
            let target = std::fs::canonicalize(&entry).unwrap_or_else(|_| entry.clone());
            let target_allowed = target == expected
                || same_private_stable_suite_path(&target, &expected)
                || target.starts_with(&shared_body_root);
            targets
                .entry(target.clone())
                .or_default()
                .push(display_home_path(&entry, home));
            if !meta.file_type().is_symlink() && !entry.starts_with(&shared_body_root) {
                skill_issues.push(format!(
                    "{} is not a thin-index symlink",
                    display_home_path(&entry, home)
                ));
            } else if !target_allowed {
                skill_issues.push(format!(
                    "{} -> {}",
                    display_home_path(&entry, home),
                    target.display()
                ));
            }
        }
        if !skill_issues.is_empty() || targets.len() > 1 {
            let parts: Vec<String> = targets
                .iter()
                .map(|(target, entries)| format!("{} -> {}", entries.join(", "), target.display()))
                .collect();
            mismatches.push(format!(
                "{}: expected {}; actual {}",
                skill.name,
                expected.display(),
                parts.join("; ")
            ));
        }
    }

    if mismatches.is_empty() {
        return Finding::pass(
            check,
            format!("host skill entries resolve to one canonical body ({checked_entries} checked)"),
        );
    }

    let total = mismatches.len();
    let mut shown: Vec<String> = mismatches.into_iter().take(20).collect();
    if total > shown.len() {
        shown.push(format!("... {more} more", more = total - shown.len()));
    }
    Finding::fail(
        check,
        "host skill entries do not resolve to a single canonical body",
        shown.join("\n"),
    )
}

/// Check that the Rust raw-tool-call Stop guard is preserved in the Stop pipeline.
pub fn raw_tool_call_stop_guard_present(repo_root: &Path) -> Finding {
    let check = "raw_tool_call_stop_guard_present";
    if is_public_edition(repo_root) {
        return Finding::skip(
            check,
            "public edition does not require the raw-tool-call Stop guard",
        );
    }
    match stop_hook_commands(&repo_root.join(".claude/settings.json")) {
        Some(cmds) if cmds.iter().any(|c| c.contains(RAW_GUARD_MARKER)) => Finding::pass(
            check,
            "Rust host lifecycle stop-guard present in Stop pipeline",
        ),
        Some(_) => Finding::warn(
            check,
            "Rust host lifecycle stop-guard missing from Stop pipeline",
            "Restore `ags host lifecycle --event stop-guard` as the first Stop hook.",
        ),
        None => Finding::warn(
            check,
            "no readable Stop pipeline in .claude/settings.json",
            "Restore .claude/settings.json with the Rust lifecycle guard as the first Stop hook.",
        ),
    }
}

/// Check that the project task-memory entrypoint exists and is not stale
/// relative to the most recent task archive. Advisory.
pub fn project_task_memory_status(repo_root: &Path) -> Finding {
    match ags_platform::home_dir() {
        Some(h) => project_task_memory_status_at(repo_root, &h),
        None => Finding::skip(
            "project_task_memory_status",
            "home directory not set — cannot locate project memory store",
        ),
    }
}

pub(super) fn project_task_memory_status_at(repo_root: &Path, home: &Path) -> Finding {
    let check = "project_task_memory_status";
    let dir = project_memory_dir(repo_root, home);
    let task_memory = dir.join("task-memory.md");
    if !task_memory.is_file() {
        if !dir.exists() {
            return Finding::skip(
                check,
                format!("no project memory store at {}", dir.display()),
            );
        }
        return Finding::warn(
            check,
            "task-memory.md missing",
            format!(
                "Expected at {}. Run `ags init` or the capture chain to create it.",
                task_memory.display()
            ),
        );
    }
    let tm_mtime = std::fs::metadata(&task_memory)
        .and_then(|m| m.modified())
        .ok();
    let newest_archive = newest_subdir_mtime(&dir.join("task-archive"));
    match (tm_mtime, newest_archive) {
        (Some(tm), Some(arch)) if arch > tm => Finding::warn(
            check,
            "task-memory.md is stale (newer task archives exist)",
            "Run `ags memory archive <receipt>` or SessionEnd after a verified `ags task close`.",
        ),
        _ => Finding::pass(
            check,
            format!("task-memory.md present at {}", task_memory.display()),
        ),
    }
}

/// Check that the manual `## 项目设计目的` block in the context capsule is
/// intact (i.e. the capsule has not been overwritten by automation). Advisory.
pub fn context_capsule_integrity(repo_root: &Path) -> Finding {
    match ags_platform::home_dir() {
        Some(h) => context_capsule_integrity_at(repo_root, &h),
        None => Finding::skip(
            "context_capsule_integrity",
            "home directory not set — cannot locate context capsule",
        ),
    }
}

pub(super) fn context_capsule_integrity_at(repo_root: &Path, home: &Path) -> Finding {
    let check = "context_capsule_integrity";
    let dir = project_memory_dir(repo_root, home);
    let capsule = dir.join("context-capsule.md");
    if !capsule.is_file() {
        if !dir.exists() {
            return Finding::skip(
                check,
                format!("no project memory store at {}", dir.display()),
            );
        }
        return Finding::warn(
            check,
            "context-capsule.md missing",
            format!(
                "Expected at {}. Run `ags init` to create it.",
                capsule.display()
            ),
        );
    }
    match std::fs::read_to_string(&capsule) {
        Ok(raw) if raw.contains("## 项目设计目的") => Finding::pass(
            check,
            "context-capsule.md present with manual design-purpose block",
        ),
        Ok(_) => Finding::warn(
            check,
            "context-capsule.md is missing the manual `## 项目设计目的` block",
            "The capsule may have been overwritten by automation — restore the human-only design-purpose section.",
        ),
        Err(e) => Finding::warn(check, "cannot read context-capsule.md", e.to_string()),
    }
}

// ── Project memory lifecycle closure (composite) ──────────────────────────

/// Composite check across every detected native host adapter. A Claude closure
/// cannot hide a missing Codex/OMP closure.
pub fn project_memory_lifecycle_closure(repo_root: &Path) -> Finding {
    let identity = ags_workspace_facts::detect_project(repo_root);
    let project_onboarded = identity.project_profile_path.is_some() || identity.is_ags_integrated;
    let home = ags_platform::home_dir_or_temp();
    let mut hosts = Vec::new();
    if home.join(".claude").is_dir()
        || repo_root.join(".claude").is_dir()
        || ags_platform::is_on_path("claude")
    {
        hosts.push(ags_workspace_facts::AgentType::ClaudeCode);
    }
    if home.join(".codex").is_dir()
        || repo_root.join(".codex").is_dir()
        || ags_platform::is_on_path("codex")
    {
        hosts.push(ags_workspace_facts::AgentType::Codex);
    }
    if home.join(".omp").is_dir()
        || repo_root.join(".omp").is_dir()
        || ags_platform::is_on_path("omp")
    {
        hosts.push(ags_workspace_facts::AgentType::from_str("omp").expect("valid host"));
    }
    if hosts.is_empty() {
        hosts.push(ags_workspace_facts::AgentType::ClaudeCode);
    }
    let lifecycles: Vec<_> = hosts
        .iter()
        .map(|host| ags_workspace_facts::compute_memory_lifecycle_for_host(repo_root, host))
        .collect();
    if lifecycles
        .iter()
        .all(|lifecycle| lifecycle.status == "full")
    {
        return Finding::pass(
            "project_memory_lifecycle_closure",
            format!(
                "native memory closure complete for detected hosts: {}",
                lifecycles
                    .iter()
                    .map(|lifecycle| lifecycle.host.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    let detail = lifecycles
        .iter()
        .map(|lifecycle| {
            format!(
                "{}={} ({})",
                lifecycle.host, lifecycle.status, lifecycle.summary
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let representative = lifecycles
        .iter()
        .find(|lifecycle| lifecycle.status != "full")
        .expect("at least one incomplete lifecycle");
    let mut finding = lifecycle_to_finding(representative, project_onboarded);
    finding.message = "one or more detected hosts lack a complete native memory lifecycle".into();
    finding.detail = Some(detail);
    finding
}

pub(super) fn lifecycle_to_finding(
    ml: &ags_workspace_facts::MemoryLifecycle,
    project_onboarded: bool,
) -> Finding {
    let check = "project_memory_lifecycle_closure";
    match ml.status.as_str() {
        "full" => Finding::pass(check, &ml.summary),
        "absent" if project_onboarded => Finding::warn(
            check,
            "managed project is missing its project memory store",
            "Run `ags init --target <project>` to recreate the project memory store, then `ags agents govern --agent <host> --apply` to install that host's AGS-owned native adapter.",
        ),
        "absent" => Finding::skip(check, &ml.summary),
        "read-only" => Finding::warn(check, "memory lifecycle read-only", &ml.summary),
        "write-only" => Finding::warn(check, "memory lifecycle write-only", &ml.summary),
        "files-only" => Finding::warn(check, "memory lifecycle files-only", &ml.summary),
        other => Finding::warn(
            check,
            format!("memory lifecycle unknown status: {other}"),
            &ml.summary,
        ),
    }
}
