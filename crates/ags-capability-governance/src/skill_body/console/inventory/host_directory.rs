use super::*;

/// Build the unified managed-capability inventory. Read-only. Includes
/// host-visibility evidence for each requested host (default: claude-code).
/// Walk up from `start` (inclusive) looking for a `.git` entry; the nearest
/// ancestor that has one is the project root. `None` when none is found.
pub(in crate::skill_body::console) fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(p) = cur {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

/// Host thin-index visibility for a capability discovered on disk in a host's
/// skills dir. Checks BOTH the normal entry (`<subdir>/<name>`) and the host
/// system area (`<subdir>/.system/<name>`), so a `.system` skill reads visible
/// where it actually lives. Read-only.
pub(in crate::skill_body::console) fn host_dir_entry_visibility(
    home: &Path,
    host: &str,
    name: &str,
) -> HostVisibility {
    let Some(subdir) = host_skills_subdir(host) else {
        return HostVisibility {
            host: host.to_string(),
            supported: false,
            status: HostVisibilityStatus::Unsupported,
            evidence: vec!["host has no skills directory".to_string()],
        };
    };
    let base = home.join(subdir);
    let mk = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };
    // Track the first degraded reason so a valid match at the other location can
    // still win, but a present-but-invalid SKILL.md is never silently passed as
    // Visible.
    let mut degraded: Option<HostVisibility> = None;
    let mut locations = vec![
        ("entry".to_string(), base.join(name)),
        ("system".to_string(), base.join(".system").join(name)),
    ];
    if matches!(host, "codex" | "omp") {
        locations.push(("shared".to_string(), home.join(".agents/skills").join(name)));
    }
    if host == "codex" {
        for root in codex_plugin_skill_dirs(home) {
            locations.push(("enabled-plugin".to_string(), root.join(name)));
        }
    }
    for (loc, dir) in locations {
        let Ok(meta) = std::fs::symlink_metadata(&dir) else {
            continue;
        };
        if meta.file_type().is_symlink() && !dir.exists() {
            if degraded.is_none() {
                degraded = Some(mk(
                    HostVisibilityStatus::Degraded,
                    vec![format!(
                        "dangling symlink (target missing): {}",
                        dir.display()
                    )],
                ));
            }
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        // Validate SKILL.md front-matter identity exactly like
        // `skill_path_visibility`: a present SKILL.md whose declared `name`
        // differs (or is unparseable / unreadable) is NOT the capability being
        // gated and must read Degraded, never Visible — so a mismatched or
        // replaced `.system/<name>` (or host-dir `<name>`) body cannot pass the
        // runtime skill-tag gate as the adopted capability.
        match std::fs::read_to_string(&skill_md) {
            Ok(text) => match crate::skill_body::parse_front_matter(&text)
                .0
                .as_deref()
                .map(str::trim)
            {
                Some(found) if found == name => {
                    return mk(
                        HostVisibilityStatus::Visible,
                        vec![format!(
                            "{loc} present; SKILL.md front-matter name matches: {}",
                            skill_md.display()
                        )],
                    );
                }
                Some(found) => {
                    if degraded.is_none() {
                        degraded = Some(mk(
                            HostVisibilityStatus::Degraded,
                            vec![format!(
                                "SKILL.md name mismatch: declares '{found}' but expected '{name}' at {}",
                                skill_md.display()
                            )],
                        ));
                    }
                }
                None => {
                    if degraded.is_none() {
                        degraded = Some(mk(
                            HostVisibilityStatus::Degraded,
                            vec![format!(
                                "SKILL.md present but front-matter not parseable: {}",
                                skill_md.display()
                            )],
                        ));
                    }
                }
            },
            Err(e) => {
                if degraded.is_none() {
                    degraded = Some(mk(
                        HostVisibilityStatus::Degraded,
                        vec![format!("SKILL.md unreadable: {} ({e})", skill_md.display())],
                    ));
                }
            }
        }
    }
    degraded.unwrap_or_else(|| {
        mk(
            HostVisibilityStatus::NotVisible,
            vec![format!("not found under {}", base.display())],
        )
    })
}

/// One discovered host-dir skill candidate before per-host visibility is filled.
pub(in crate::skill_body::console) struct HostDirCandidate {
    source: PathBuf,
    managed_status: ManagedStatus,
    canonical_present: bool,
    risk_notes: Vec<String>,
}

/// Classify one host skills-dir entry that is NOT already a known suite/repo
/// skill. Returns `None` when the entry should be ignored (housekeeping names).
/// READ-ONLY: never writes, never copies, never relinks. System (`.system`)
/// skills, sibling-suite bodies, other-project bodies, real user dirs, and
/// arbitrary external symlink targets are each classified distinctly, and all
/// land fail-closed `routing: None` (not-routable) until explicitly adopted.
pub(in crate::skill_body::console) fn classify_host_dir_entry(
    repo_root: &Path,
    entry: &Path,
    name: &str,
    is_system: bool,
) -> Option<HostDirCandidate> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(".bak")
        || name.starts_with(".ags-")
    {
        return None;
    }
    let link_meta = std::fs::symlink_metadata(entry).ok()?;
    let is_symlink = link_meta.file_type().is_symlink();
    // Dangling symlink → recognized but broken / unmanaged.
    if is_symlink && !entry.exists() {
        return Some(HostDirCandidate {
            source: entry.to_path_buf(),
            managed_status: ManagedStatus::Unmanaged,
            canonical_present: false,
            risk_notes: vec![format!(
                "Dangling host thin index (symlink target missing): {}. Recognized read-only; not routable.",
                entry.display()
            )],
        });
    }
    let real = std::fs::canonicalize(entry).ok()?;
    let has_skill_md = real.join("SKILL.md").is_file();

    // System skills (host built-ins under `.system`) — read-only recognition.
    if is_system {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::HostSystem,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "Host system skill — recognized read-only. AGS never holds/copies/relinks the body. Adopt via the registry to make it routable.".to_string(),
            ],
        });
    }
    // A thin index into THIS repo's AGS store is the same body already covered
    // by the suite/repo passes — skip to avoid a duplicate row.
    if canonical_within_store(repo_root, entry) {
        return None;
    }
    // A body inside a sibling AGS suite mirror (private<->stable) — recognized.
    if split_suite_runtime_path(&real).is_some() {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::Discovered,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "Discovered from a sibling AGS suite mirror. Opt-in candidate; not routable until registered.".to_string(),
            ],
        });
    }
    // A body inside another git project (not the AGS suite) — project-local.
    if let Some(proj) = find_git_root(&real) {
        if proj != *repo_root {
            return Some(HostDirCandidate {
                source: real,
                managed_status: ManagedStatus::ProjectLocal,
                canonical_present: has_skill_md,
                risk_notes: vec![format!(
                    "Project-local skill (body under {}). Read-only recognition; not routable until registered.",
                    proj.display()
                )],
            });
        }
    }
    // A real directory the user dropped directly into the host skills dir.
    if !is_symlink && real.is_dir() {
        return Some(HostDirCandidate {
            source: real,
            managed_status: ManagedStatus::Discovered,
            canonical_present: has_skill_md,
            risk_notes: vec![
                "User-installed local skill (real dir in host skills dir). Opt-in candidate; not routable until registered.".to_string(),
            ],
        });
    }
    // Anything else: a symlink to an arbitrary external location AGS does not
    // govern (e.g. an app bundle, a non-suite tool root) — unmanaged.
    Some(HostDirCandidate {
        source: real,
        managed_status: ManagedStatus::Unmanaged,
        canonical_present: has_skill_md,
        risk_notes: vec![format!(
            "External user-installed skill outside AGS governance ({}). Recognized read-only; not routable.",
            entry.display()
        )],
    })
}

/// Full-machine discovery: scan every supported host's skills dir (and its
/// `.system` area) for skills not already known from the suite/repo passes, and
/// model each as a `ManagedCapability` with per-host thin-index visibility.
/// READ-ONLY. Bodies are never copied; classification is fail-closed
/// (`routing: None` ⇒ not-routable) — adoption into the registry is the only
/// way one of these becomes routable.
pub(in crate::skill_body::console) fn discover_host_dir_capabilities(
    ctx: &ConsoleContext,
    hosts: &[String],
    known: &mut Vec<String>,
) -> Vec<ManagedCapability> {
    let mut by_name: std::collections::BTreeMap<String, HostDirCandidate> =
        std::collections::BTreeMap::new();
    for host in hosts {
        let Some(subdir) = host_skills_subdir(host) else {
            continue;
        };
        let base = ctx.home.join(subdir);
        // Normal entries.
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if known.iter().any(|n| n == &name) || by_name.contains_key(&name) {
                    continue;
                }
                if let Some(c) = classify_host_dir_entry(&ctx.repo_root, &e.path(), &name, false) {
                    by_name.insert(name, c);
                }
            }
        }
        // Host system area.
        let sys = base.join(".system");
        if let Ok(rd) = std::fs::read_dir(&sys) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if known.iter().any(|n| n == &name) || by_name.contains_key(&name) {
                    continue;
                }
                if let Some(c) = classify_host_dir_entry(&ctx.repo_root, &e.path(), &name, true) {
                    by_name.insert(name, c);
                }
            }
        }

        // Shared multi-agent bodies and enabled Codex plugin skill roots are
        // runtime-visible sources too. `shared_skill_dirs_for_host` only
        // returns plugin roots proven enabled by host configuration, so a
        // disabled cache entry never enters the candidate catalog.
        for root in shared_skill_dirs_for_host(ctx, host) {
            let enabled_plugin = root.starts_with(ctx.home.join(".codex/plugins/cache"));
            let Ok(rd) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if known.iter().any(|known| known == &name) || by_name.contains_key(&name) {
                    continue;
                }
                if let Some(mut candidate) =
                    classify_host_dir_entry(&ctx.repo_root, &entry.path(), &name, false)
                {
                    if enabled_plugin {
                        candidate.risk_notes.push(
                            "Discovered from a host-configured enabled plugin root; cache presence alone is never activation evidence."
                                .to_string(),
                        );
                    }
                    by_name.insert(name, candidate);
                }
            }
        }
    }

    // Project-scoped skill roots are candidates even before a host thin index
    // points at them. They remain read-only and fail closed until overlay
    // adoption; the suite never copies or rewrites their bodies.
    for root in [
        ctx.repo_root.join(".agents/skills"),
        ctx.repo_root.join(".codex/skills"),
        ctx.repo_root.join(".claude/skills"),
    ] {
        let Ok(rd) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if known.iter().any(|known| known == &name) || by_name.contains_key(&name) {
                continue;
            }
            let path = entry.path();
            if !path.join("SKILL.md").is_file() {
                continue;
            }
            by_name.insert(
                name,
                HostDirCandidate {
                    source: path,
                    managed_status: ManagedStatus::ProjectLocal,
                    canonical_present: true,
                    risk_notes: vec![
                        "Project-local skill — recognized read-only and unavailable for routing until explicitly adopted in the machine overlay."
                            .to_string(),
                    ],
                },
            );
        }
    }
    let mut out = Vec::new();
    for (name, cand) in by_name {
        known.push(name.clone());
        let host_visibility: Vec<HostVisibility> = hosts
            .iter()
            .map(|h| host_dir_entry_visibility(&ctx.home, h, &name))
            .collect();
        out.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name,
            source: Some(cand.source.to_string_lossy().to_string()),
            profile: None,
            managed_status: cand.managed_status,
            registry_status: RegistryStatus::NotRegistered,
            canonical_present: cand.canonical_present,
            expected_hosts: Vec::new(),
            host_visibility,
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: cand.risk_notes,
            routing: None,
        });
    }
    out
}
