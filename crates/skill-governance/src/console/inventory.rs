use super::*;
#[allow(unused_imports)]
use super::{actions::*, host_probe::*, model::*};
// ── Inventory ──────────────────────────────────────────────────────────────────

/// Build the unified managed-capability inventory. Read-only. Includes
/// host-visibility evidence for each requested host (default: claude-code).
/// Walk up from `start` (inclusive) looking for a `.git` entry; the nearest
/// ancestor that has one is the project root. `None` when none is found.
pub(super) fn find_git_root(start: &Path) -> Option<PathBuf> {
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
pub(super) fn host_dir_entry_visibility(home: &Path, host: &str, name: &str) -> HostVisibility {
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
            Ok(text) => match crate::parse_front_matter(&text).0.as_deref().map(str::trim) {
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
pub(super) struct HostDirCandidate {
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
pub(super) fn classify_host_dir_entry(
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
pub(super) fn discover_host_dir_capabilities(
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

pub fn build_inventory(ctx: &ConsoleContext, hosts: &[&str]) -> ManagedInventoryResult {
    let hosts: Vec<String> = if hosts.is_empty() {
        SUPPORTED_HOSTS.iter().map(|s| s.to_string()).collect()
    } else {
        hosts.iter().map(|s| s.to_string()).collect()
    };

    // One MCP probe per requested supported host (reserved hosts get none).
    let probes: Vec<(String, HostMcpProbe)> = hosts
        .iter()
        .filter(|h| host_skills_subdir(h).is_some())
        .map(|h| (h.clone(), probe_host_mcp(ctx, h)))
        .collect();
    let mut caps: Vec<ManagedCapability> = Vec::new();

    // Skill-resolution metadata — manifest is the single authority.
    // Read up-front so expected-host gating can exclude internal-entrypoint
    // route targets (routing.parent set) before they are ever flagged.
    let routing_meta = read_routing_metadata(&ctx.repo_root);

    // 1. Suite-managed skills (from the suite manifest + ignore/adoption).
    let scan = crate::scan_skills(&ctx.repo_root);
    let mut known_skill_names: Vec<String> = Vec::new();
    for s in &scan.skills {
        known_skill_names.push(s.name.clone());
        let managed_status = match s.profile.as_str() {
            "required" | "optional" | "personal" => ManagedStatus::SuiteManaged,
            "ignored" | "rejected" => ManagedStatus::Ignored,
            _ => ManagedStatus::Discovered,
        };
        let registry_status = match managed_status {
            ManagedStatus::SuiteManaged => RegistryStatus::Registered,
            _ => RegistryStatus::NotRegistered,
        };
        let mut risk_notes: Vec<String> = s.warnings.clone();
        if let Some(fam) = cli_family_for_skill(&s.name) {
            risk_notes.push(format!(
                "Fronted by external CLI `{}` ({}). AGS distributes the skill entry but does not run `{} update`.",
                fam.cli, fam.endpoint, fam.cli
            ));
        }
        // Required skills are what the suite installs → expected visible in the
        // host. Optional/personal are opt-in, so not flagged as a verify gap.
        // An internal-entrypoint route target (routing.parent set) is NEVER a
        // standalone host body, so it never produces an expected-host gap.
        let s_is_route_target = routing_meta
            .map
            .get(&s.name)
            .is_some_and(|r| r.parent.is_some());
        let expected_hosts = if s.profile == "required" && !s_is_route_target {
            supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        };
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: s.name.clone(),
            source: s.source.clone(),
            profile: Some(s.profile.clone()),
            managed_status,
            registry_status,
            canonical_present: canonical_skill_present(&ctx.repo_root, s.source.as_deref()),
            expected_hosts,
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 1.5. Registry-governed external skill bodies. The external manager owns
    // the body under the shared multi-agent store; AGS owns only metadata and
    // per-host thin indexes.
    for body in routing_meta.external_skill_bodies.values() {
        if known_skill_names.iter().any(|name| name == &body.name) {
            continue;
        }
        known_skill_names.push(body.name.clone());
        let source = ctx.home.join(".agents/skills").join(&body.name);
        let canonical_present = canonical_within_shared_store(&ctx.home, &body.name, &source)
            && source.join("SKILL.md").is_file();
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: body.name.clone(),
            source: Some(source.to_string_lossy().to_string()),
            profile: body.profile.clone(),
            managed_status: ManagedStatus::Governed,
            registry_status: RegistryStatus::Registered,
            canonical_present,
            expected_hosts: supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "External skill body managed by `{}`; AGS owns only governance metadata and host thin indexes.",
                body.manager
            )],
            routing: None,
        });
    }

    // 1.6. Required routable parent skills declared only in the registry must
    // still materialize in the expected universe. In particular, a fresh
    // machine with no host body needs a real NotVisible row so strict verify
    // cannot report a false-green result by silently shrinking its denominator.
    for required in &routing_meta.required_skill_parents {
        if known_skill_names.iter().any(|name| name == &required.name) {
            continue;
        }
        known_skill_names.push(required.name.clone());
        let host_system = required.source_type.as_deref() == Some("host-system");
        let source = required
            .local_path
            .as_deref()
            .map(|path| ctx.repo_root.join(path))
            .or_else(|| {
                hosts.iter().find_map(|host| {
                    host_skill_body_dirs(ctx, host, &required.name)
                        .into_iter()
                        .next()
                })
            })
            .or_else(|| {
                (!host_system).then(|| ctx.home.join(".agents/skills").join(&required.name))
            });
        let canonical_present = source
            .as_ref()
            .is_some_and(|body| body.join("SKILL.md").is_file());
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: required.name.clone(),
            source: source.map(|path| path.to_string_lossy().to_string()),
            profile: Some(required.profile.clone()),
            managed_status: if host_system {
                ManagedStatus::HostSystem
            } else {
                ManagedStatus::Governed
            },
            registry_status: RegistryStatus::Registered,
            canonical_present,
            expected_hosts: supported_skill_hosts()
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "Required registry parent (source.type={}); absence remains an expected-host failure.",
                required.source_type.as_deref().unwrap_or("unspecified")
            )],
            routing: None,
        });
    }

    // 2. Local skill directories not in the manifest → Discovered (opt-in).
    let inv = crate::scan_skill_inventory(&ctx.repo_root);
    for e in &inv.entries {
        if known_skill_names.iter().any(|n| n == &e.name) {
            continue;
        }
        known_skill_names.push(e.name.clone());
        let mut risk_notes = Vec::new();
        if !e.risk_hints.is_empty() {
            risk_notes.push(format!("SKILL.md risk hints: {}", e.risk_hints.join(", ")));
        }
        if let Some(fam) = cli_family_for_skill(&e.name) {
            risk_notes.push(format!(
                "Fronted by external CLI `{}` ({}).",
                fam.cli, fam.endpoint
            ));
        }
        caps.push(ManagedCapability {
            kind: ManagedKind::Skill,
            name: e.name.clone(),
            source: Some(e.path.clone()),
            profile: None,
            managed_status: ManagedStatus::Discovered,
            registry_status: RegistryStatus::NotRegistered,
            canonical_present: e.has_skill_md,
            // Discovered skills are opt-in candidates → not a verify gap.
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 2.5. Full-machine discovery: host skills dirs (incl. `.system`) — system,
    //      external, sibling-suite, project-local, and user-installed skills not
    //      already known. READ-ONLY, fail-closed not-routable until adopted.
    for cap in discover_host_dir_capabilities(ctx, &hosts, &mut known_skill_names) {
        caps.push(cap);
    }

    // 3. Governed MCPs + AGS suite interface + CLI-backed MCPs from the registry.
    for e in read_mcp_registry(&ctx.repo_root) {
        let is_cli = e.manager.as_deref() == Some("external-cli");
        let (kind, managed_status, mut risk_notes) = if e.suite_interface {
            (
                ManagedKind::SuiteInterface,
                ManagedStatus::SuiteInterface,
                vec![
                    "AGS host initialization adapter — governance authority, not a governed third-party MCP.".to_string(),
                ],
            )
        } else if is_cli {
            (
                ManagedKind::CliBacked,
                ManagedStatus::Governed,
                vec!["Governed CLI-backed MCP.".to_string()],
            )
        } else {
            (
                ManagedKind::Mcp,
                ManagedStatus::Governed,
                vec!["Governed third-party MCP.".to_string()],
            )
        };
        if !e.suite_interface {
            risk_notes.push(
                "AGS advises host MCP registration commands; it never runs `claude mcp add/remove` itself.".to_string(),
            );
        }
        // Expected visible where the registry declares the server installed for
        // a supported host. Flags "registry says installed but host can't see it"
        // drift; an MCP the registry says is NOT installed here is not a gap.
        // An internal-entrypoint route target (routing.parent set) is never a
        // standalone host body → no expected-host gap.
        let e_is_route_target = routing_meta
            .map
            .get(&e.name)
            .is_some_and(|r| r.parent.is_some());
        let mut expected_hosts: Vec<String> = if e_is_route_target {
            Vec::new()
        } else {
            e.installed_clients
                .iter()
                .filter(|c| SUPPORTED_HOSTS.contains(&c.as_str()))
                .cloned()
                .collect()
        };
        // OMP imports Codex MCP configuration. A registry entry declared
        // installed for Codex is therefore expected in OMP as inherited
        // source configuration too; no duplicate OMP registrar is required.
        if !e_is_route_target
            && e.installed_clients.iter().any(|client| client == "codex")
            && !expected_hosts.iter().any(|client| client == "omp")
        {
            expected_hosts.push("omp".to_string());
        }
        expected_hosts.sort();
        expected_hosts.dedup();
        caps.push(ManagedCapability {
            kind,
            name: e.name.clone(),
            source: Some("manifests/mcp-registry.yaml".to_string()),
            profile: None,
            managed_status,
            registry_status: RegistryStatus::Registered,
            // The MCP definition in the registry IS the canonical body.
            canonical_present: true,
            expected_hosts,
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes,
            routing: None,
        });
    }

    // 4. Synthetic CLI-backed binaries for any present family (e.g. lark-cli).
    let mut family_clis: Vec<&'static CliFamily> = Vec::new();
    for fam in CLI_FAMILIES {
        let present = caps.iter().any(|c| {
            matches!(c.kind, ManagedKind::Skill)
                && cli_family_for_skill(&c.name).map(|f| f.cli) == Some(fam.cli)
        });
        let already = caps.iter().any(|c| c.name == fam.cli);
        if present && !already {
            family_clis.push(fam);
        }
    }
    for fam in family_clis {
        caps.push(ManagedCapability {
            kind: ManagedKind::CliBacked,
            name: fam.cli.to_string(),
            source: Some(format!("external CLI binary `{}`", fam.cli)),
            profile: None,
            managed_status: ManagedStatus::Unmanaged,
            registry_status: RegistryStatus::NotRegistered,
            // The CLI binary is external — AGS does not hold its canonical body.
            canonical_present: false,
            // A CLI binary is not a host entry → never a host-visibility gap.
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: vec![format!(
                "External official CLI talking to {}. Referenced, not adopted; AGS never runs `{} update`. Live endpoint health is a degraded observation only.",
                fam.endpoint, fam.cli
            )],
            routing: None,
        });
    }

    // 5. Fill host visibility, health, actions, and routing for every capability.
    for cap in &mut caps {
        let cli_backed_external = matches!(cap.kind, ManagedKind::CliBacked)
            && matches!(cap.managed_status, ManagedStatus::Unmanaged);
        // Host-dir-discovered capabilities pre-fill their own visibility (they
        // may live under `.system`), so only fill when not already populated.
        if cap.host_visibility.is_empty() {
            for host in &hosts {
                let probe = probes.iter().find(|(h, _)| h == host).map(|(_, p)| p);
                let canonical_source = if matches!(cap.kind, ManagedKind::Skill)
                    && !matches!(cap.managed_status, ManagedStatus::HostSystem)
                {
                    cap.source
                        .as_deref()
                        .map(|source| resolve_source(&ctx.repo_root, source))
                } else {
                    None
                };
                let external_shared = is_external_shared_skill(ctx, cap);
                cap.host_visibility.push(host_visibility(
                    ctx,
                    host,
                    &cap.kind,
                    &cap.name,
                    canonical_source.as_deref(),
                    external_shared,
                    probe,
                ));
            }
        }
        cap.health_status = derive_health(
            &cap.kind,
            &cap.name,
            &cap.host_visibility,
            &probes,
            cli_backed_external,
        );
        cap.actions = actions_for(&cap.kind, &cap.managed_status);
        // Stable routing facts (or None when the manifest declares none).
        //
        // A host-dir discovered capability can be explicitly adopted by adding a
        // skills-registry member with routing metadata (for example a
        // host-system `skill-creator` entry). In that case the manifest is the
        // registry authority even though AGS still does not hold or relink the
        // external body, so inventory must not report the row as
        // `not-registered`.
        cap.routing = routing_meta.map.get(&cap.name).cloned();
        if cap.routing.is_some() {
            cap.registry_status = RegistryStatus::Registered;
            if matches!(cap.managed_status, ManagedStatus::HostSystem) {
                cap.risk_notes
                    .retain(|note| !note.contains("Adopt via the registry to make it routable"));
                cap.risk_notes.push(
                    "Host system skill is registry-adopted for routing; AGS still recognizes it read-only and never holds/copies/relinks the body.".to_string(),
                );
            }
        }
    }

    apply_route_target_exposure_shape(&mut caps, &routing_meta.route_targets);
    apply_playbook_entrypoint_integrity(ctx, &mut caps, &routing_meta.route_targets);

    // 6. Synthesize route-target rows for internal entrypoints (playbook / MCP
    //    tool / CLI subcommand) declared under `route_targets:`. Routing-only:
    //    kind inherited from the parent, NO expected_hosts, NO host probe, NO
    //    actions, never adopted/synced. Skill Resolver dereferences their
    //    availability + `primary` to the parent capability.
    for (name, routing) in &routing_meta.route_targets {
        // Registry route-target declarations are authoritative over stale
        // host/plugin bodies with the same upstream name. Keeping the discovered
        // row would turn an internal playbook back into a standalone routable
        // skill and reintroduce duplicate host injection.
        let shadowed_body = caps
            .iter()
            .position(|cap| cap.name == *name)
            .map(|index| caps.remove(index));
        known_skill_names.retain(|known| known != name);
        known_skill_names.push(name.clone());
        let kind = routing
            .parent
            .as_ref()
            .map(|p| p.kind.clone())
            .unwrap_or(ManagedKind::Skill);
        caps.push(ManagedCapability {
            kind,
            name: name.clone(),
            source: Some("manifests (route_targets)".to_string()),
            profile: None,
            managed_status: ManagedStatus::RouteTarget,
            registry_status: RegistryStatus::Registered,
            // Metadata-only: the canonical body is the parent capability.
            canonical_present: true,
            expected_hosts: Vec::new(),
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
            actions: Vec::new(),
            risk_notes: {
                let mut notes = vec![
                    "Internal entrypoint route target of a parent capability; routing-only, never a host body, never adopted/synced.".to_string(),
                ];
                if let Some(body) = shadowed_body {
                    notes.push(format!(
                        "A stale standalone '{}' body was discovered from {:?} and shadowed by the registry route target.",
                        body.name, body.source
                    ));
                }
                notes
            },
            routing: Some(routing.clone()),
        });
    }

    caps.sort_by(|a, b| a.name.cmp(&b.name));

    let summary = summarize(&caps);
    ManagedInventoryResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        hosts,
        capabilities: caps,
        summary,
        note: "Read-only inventory. Third-party capabilities are opt-in; AGS never silently bundles or installs. Use `ags skill adopt <source>` for a catalog/local/GitHub audit plan, then `--apply` to confirm its machine-private body/source registry/overlay and planned-host thin indexes; use `ags skill ignore <skill-id>` for the overlay-only lifecycle.".to_string(),
        routing_parse_failures: routing_meta.parse_failures,
    }
}

pub(super) fn summarize(caps: &[ManagedCapability]) -> ManagedInventorySummary {
    let claude_visible = caps
        .iter()
        .filter(|c| {
            c.host_visibility
                .iter()
                .any(|v| v.host == "claude-code" && v.status == HostVisibilityStatus::Visible)
        })
        .count();
    ManagedInventorySummary {
        total: caps.len(),
        skills: caps.iter().filter(|c| c.kind == ManagedKind::Skill).count(),
        mcps: caps.iter().filter(|c| c.kind == ManagedKind::Mcp).count(),
        suite_interfaces: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::SuiteInterface)
            .count(),
        cli_backed: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::CliBacked)
            .count(),
        canonical_present: caps.iter().filter(|c| c.canonical_present).count(),
        claude_visible,
        risk_flagged: caps.iter().filter(|c| !c.risk_notes.is_empty()).count(),
        routing_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Routable)
            })
            .count(),
        routing_not_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::NotRoutable)
            })
            .count(),
        routing_retired: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Retired)
            })
            .count(),
        routing_uncovered: caps
            .iter()
            .filter(|c| {
                matches!(
                    c.managed_status,
                    ManagedStatus::SuiteManaged | ManagedStatus::Governed
                ) && c.routing.is_none()
            })
            .count(),
    }
}

/// Deterministic content hash of the machine-local capability snapshot. Hashes a
/// CANONICAL projection (sorted `name|kind|managed_status|registry|route_state|
/// canonical|host=visibility…|health` lines) with FNV-1a — dependency-free and
/// stable across runs for identical machine state. Used as the task-card snapshot
/// attestation token. Contains capability NAMES + statuses only — NO absolute
/// paths — so it is safe to record in a (machine-local) snapshot or a task card.
pub fn inventory_snapshot_hash(inv: &ManagedInventoryResult) -> String {
    fn vis_str(s: &HostVisibilityStatus) -> &'static str {
        match s {
            HostVisibilityStatus::Visible => "visible",
            HostVisibilityStatus::NotVisible => "not-visible",
            HostVisibilityStatus::Degraded => "degraded",
            HostVisibilityStatus::Unsupported => "unsupported",
            HostVisibilityStatus::Deferred => "deferred",
        }
    }
    fn health_str(h: &HealthStatus) -> &'static str {
        match h {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unknown => "unknown",
            HealthStatus::Unhealthy => "unhealthy",
        }
    }
    fn kind_str(k: &ManagedKind) -> &'static str {
        match k {
            ManagedKind::Skill => "skill",
            ManagedKind::Mcp => "mcp",
            ManagedKind::SuiteInterface => "suite-interface",
            ManagedKind::CliBacked => "cli-backed",
        }
    }
    let route_str = |c: &ManagedCapability| -> &'static str {
        match c.routing.as_ref().map(|r| r.route_state) {
            Some(RouteState::Routable) => "routable",
            Some(RouteState::NotRoutable) => "not-routable",
            Some(RouteState::Retired) => "retired",
            None => "none",
        }
    };
    let mut lines: Vec<String> = inv
        .capabilities
        .iter()
        .map(|c| {
            let mut vis: Vec<String> = c
                .host_visibility
                .iter()
                .map(|v| format!("{}={}", v.host, vis_str(&v.status)))
                .collect();
            vis.sort();
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                c.name,
                kind_str(&c.kind),
                managed_status_str(&c.managed_status),
                if matches!(c.registry_status, RegistryStatus::Registered) {
                    "registered"
                } else {
                    "not-registered"
                },
                route_str(c),
                c.canonical_present,
                vis.join(","),
                health_str(&c.health_status),
            )
        })
        .collect();
    lines.sort();
    let joined = lines.join("\n");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

/// Whether AGS holds the canonical skill body: the resolved source dir contains
/// a `SKILL.md`. Read-only.
pub(super) fn canonical_skill_present(repo_root: &Path, source: Option<&str>) -> bool {
    source
        .map(|s| resolve_source(repo_root, s).join("SKILL.md").is_file())
        .unwrap_or(false)
}
