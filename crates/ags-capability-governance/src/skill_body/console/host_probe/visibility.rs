use super::*;

/// The `~/<subdir>` skills directory a host loads skill entries from, if any.
/// `Some` ⇒ the host is supported and gets a real probe.
pub(in super::super) fn host_skills_subdir(host: &str) -> Option<&'static str> {
    ags_host_integration::platform_spec(host)?.native_skill_subdir
}

/// Additional shared skill sources loaded by a host. Codex, Cursor, and OMP
/// also index the multi-agent `~/.agents/skills` store; writing the same skill
/// into a native root as well creates duplicate picker entries.
pub(in super::super) fn shared_skill_dirs_for_host(
    ctx: &ConsoleContext,
    host: &str,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Some(spec) = ags_host_integration::platform_spec(host) else {
        return dirs;
    };
    if spec.loads_shared_agent_skills {
        dirs.push(ctx.home.join(".agents/skills"));
    }
    if spec.loads_codex_plugin_skills {
        dirs.extend(codex_plugin_skill_dirs(&ctx.home));
    }
    dirs
}

pub(in super::super) fn codex_plugin_skill_dirs(home: &Path) -> Vec<PathBuf> {
    let enabled = enabled_codex_plugin_names(home);
    if enabled.is_empty() {
        return Vec::new();
    }
    let cache = home.join(".codex/plugins/cache");
    let mut dirs = Vec::new();
    let Ok(marketplaces) = std::fs::read_dir(&cache) else {
        return dirs;
    };
    for marketplace in marketplaces.flatten().map(|entry| entry.path()) {
        let Ok(plugins) = std::fs::read_dir(marketplace) else {
            continue;
        };
        for plugin in plugins.flatten().map(|entry| entry.path()) {
            let Some(name) = plugin.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if enabled.contains(name) {
                collect_plugin_skill_dirs(&plugin, 0, &mut dirs);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Cache presence is not activation evidence. Codex records enabled plugins in
/// `~/.codex/config.toml`; only those plugin names may contribute runtime skill
/// visibility. A missing or malformed config therefore yields no active plugin
/// skill directories instead of making every cached version host-visible.
pub(in super::super) fn enabled_codex_plugin_names(home: &Path) -> HashSet<String> {
    let Ok(config) = std::fs::read_to_string(home.join(".codex/config.toml")) else {
        return HashSet::new();
    };
    let mut enabled = HashSet::new();
    let mut current_plugin: Option<String> = None;
    for line in config.lines().map(str::trim) {
        if let Some(id) = line
            .strip_prefix("[plugins.\"")
            .and_then(|line| line.strip_suffix("\"]"))
        {
            current_plugin = id
                .split_once('@')
                .map(|(name, _)| name)
                .or(Some(id))
                .map(str::to_string);
            continue;
        }
        if line.starts_with('[') {
            current_plugin = None;
            continue;
        }
        let Some(name) = current_plugin.as_ref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "enabled" && value.trim() == "true" {
            enabled.insert(name.clone());
        }
    }
    enabled
}

pub(in super::super) fn collect_plugin_skill_dirs(
    dir: &Path,
    depth: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > 5 {
        return;
    }
    if dir.file_name().and_then(|n| n.to_str()) == Some("skills") {
        out.push(dir.to_path_buf());
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            collect_plugin_skill_dirs(&path, depth + 1, out);
        }
    }
}

/// Compute one capability's visibility for one host. `probe` is that host's
/// MCP probe (`None` for reserved hosts).
pub(in super::super) fn host_visibility(
    ctx: &ConsoleContext,
    host: &str,
    cap_kind: &ManagedKind,
    cap_name: &str,
    canonical_source: Option<&Path>,
    external_shared: bool,
    probe: Option<&HostMcpProbe>,
) -> HostVisibility {
    if let Some(subdir) = host_skills_subdir(host) {
        return match cap_kind {
            ManagedKind::Skill => skill_path_visibility_for_host(
                ctx,
                host,
                &ctx.home.join(subdir),
                cap_name,
                canonical_source,
                external_shared,
            ),
            ManagedKind::Mcp | ManagedKind::SuiteInterface => {
                host_mcp_visibility(host, cap_name, probe)
            }
            ManagedKind::CliBacked => host_cli_visibility(host, cap_name),
        };
    }

    HostVisibility {
        host: host.to_string(),
        supported: false,
        status: HostVisibilityStatus::Unsupported,
        evidence: vec![format!(
            "Host '{host}' visibility check is not implemented in this version (model fields are stable)."
        )],
    }
}

pub(in super::super) fn skill_path_visibility_for_host(
    ctx: &ConsoleContext,
    host: &str,
    primary_skills_dir: &Path,
    name: &str,
    canonical_source: Option<&Path>,
    external_shared: bool,
) -> HostVisibility {
    if external_shared && canonical_source.is_none_or(|source| !source.join("SKILL.md").is_file()) {
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::NotVisible,
            evidence: vec![format!(
                "required shared skill body is missing: {}",
                ctx.home
                    .join(".agents/skills")
                    .join(name)
                    .join("SKILL.md")
                    .display()
            )],
        };
    }
    if external_shared
        && canonical_source
            .map(|source| !canonical_within_shared_store(&ctx.home, name, source))
            .unwrap_or(true)
    {
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Degraded,
            evidence: vec![format!(
                "external canonical body is missing or escapes the shared skill store: {}",
                ctx.home.join(".agents/skills").join(name).display()
            )],
        };
    }
    let primary = skill_path_visibility(host, primary_skills_dir, name, canonical_source);
    let shared_results = shared_skill_dirs_for_host(ctx, host)
        .into_iter()
        .map(|shared_skills_dir| {
            let direct_external_body = external_shared
                && canonical_source
                    .is_some_and(|canonical| shared_skills_dir.join(name) == canonical);
            let shared = skill_path_visibility(
                host,
                &shared_skills_dir,
                name,
                if external_shared && !direct_external_body {
                    canonical_source
                } else {
                    None
                },
            );
            (shared_skills_dir, shared)
        })
        .collect::<Vec<_>>();
    if let Some((shared_skills_dir, shared)) = shared_results
        .iter()
        .find(|(_, shared)| shared.status == HostVisibilityStatus::Degraded)
    {
        let mut evidence = vec![format!(
            "conflicting shared skill entry under {}",
            shared_skills_dir.display()
        )];
        evidence.extend(shared.evidence.clone());
        if primary.status != HostVisibilityStatus::NotVisible {
            evidence.extend(primary.evidence);
        }
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Degraded,
            evidence,
        };
    }
    let shared_visible = shared_results
        .into_iter()
        .find(|(_, shared)| shared.status == HostVisibilityStatus::Visible);
    let Some((shared_skills_dir, shared)) = shared_visible else {
        return primary;
    };

    let mut evidence = Vec::new();
    evidence.push(format!(
        "shared skill source visible under {}",
        shared_skills_dir.display()
    ));
    if primary.status == HostVisibilityStatus::Visible {
        evidence.push(format!(
            "duplicate host entry also exists under {}",
            primary_skills_dir.display()
        ));
        evidence.extend(shared.evidence);
        evidence.extend(primary.evidence);
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Degraded,
            evidence,
        };
    }
    evidence.extend(shared.evidence);
    if primary.status == HostVisibilityStatus::Degraded {
        evidence.extend(primary.evidence);
        return HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Degraded,
            evidence,
        };
    }

    HostVisibility {
        host: host.to_string(),
        supported: true,
        status: HostVisibilityStatus::Visible,
        evidence,
    }
}

pub(in super::super) fn host_skill_body_dirs(
    ctx: &ConsoleContext,
    host: &str,
    name: &str,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(subdir) = host_skills_subdir(host) {
        roots.push(ctx.home.join(subdir));
    }
    roots.extend(shared_skill_dirs_for_host(ctx, host));
    roots
        .into_iter()
        .map(|root| root.join(name))
        .filter(|body| body.join("SKILL.md").is_file())
        .collect()
}

/// Return nested skill entrypoints underneath a parent's playbook resource
/// directory. A host that recursively discovers `SKILL.md` files will expose
/// every one of these as an independent skill, bypassing the parent router.
pub(in super::super) fn nested_playbook_skill_files(body: &Path) -> Vec<PathBuf> {
    let root = body.join("playbooks");
    let mut pending = vec![root];
    let mut found = Vec::new();

    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                pending.push(path);
            } else if entry.file_name() == "SKILL.md" && path.is_file() {
                found.push(path);
            }
        }
    }

    found.sort();
    found
}

pub(in super::super) fn playbook_body_issues(body: &Path, playbooks: &[String]) -> Vec<String> {
    let missing: Vec<&str> = playbooks
        .iter()
        .filter_map(|playbook| {
            (!body
                .join("playbooks")
                .join(playbook)
                .join("PLAYBOOK.md")
                .is_file())
            .then_some(playbook.as_str())
        })
        .collect();
    let nested = nested_playbook_skill_files(body);
    let mut issues = Vec::new();
    if !missing.is_empty() {
        issues.push(format!(
            "required PLAYBOOK.md resource(s) are missing from {}: {}",
            body.display(),
            missing.join(", ")
        ));
    }
    if !nested.is_empty() {
        issues.push(format!(
            "nested SKILL.md file(s) are host-discoverable under {}: {}",
            body.display(),
            nested
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    issues
}

pub(in super::super) fn apply_playbook_entrypoint_integrity(
    ctx: &ConsoleContext,
    caps: &mut [ManagedCapability],
    route_targets: &[(String, RoutingMetadata)],
) {
    let mut by_parent: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (_, routing) in route_targets {
        let (Some(parent), Some(entrypoint)) = (&routing.parent, &routing.entrypoint) else {
            continue;
        };
        if parent.kind == ManagedKind::Skill && entrypoint.kind == EntrypointKind::Playbook {
            by_parent
                .entry(parent.name.clone())
                .or_default()
                .push(entrypoint.name.clone());
        }
    }

    for cap in caps.iter_mut() {
        let Some(playbooks) = by_parent.get(&cap.name) else {
            continue;
        };
        let mut degraded = false;
        for visibility in &mut cap.host_visibility {
            if visibility.status != HostVisibilityStatus::Visible {
                continue;
            }
            let bodies = host_skill_body_dirs(ctx, &visibility.host, &cap.name);
            let issues: Vec<String> = if bodies.is_empty() {
                vec![format!(
                    "host-visible parent body for '{}' could not be resolved",
                    cap.name
                )]
            } else {
                bodies
                    .iter()
                    .flat_map(|body| playbook_body_issues(body, playbooks))
                    .collect()
            };
            if !issues.is_empty() {
                visibility.status = HostVisibilityStatus::Degraded;
                visibility.evidence.extend(issues);
                degraded = true;
            }
        }
        if degraded {
            cap.health_status = HealthStatus::Degraded;
            cap.risk_notes.push(
                "The host-visible parent must expose each declared playbook as PLAYBOOK.md resources and contain no nested SKILL.md entrypoints."
                    .to_string(),
            );
        }
    }
}

pub(in super::super) fn apply_route_target_exposure_shape(
    caps: &mut [ManagedCapability],
    route_targets: &[(String, RoutingMetadata)],
) {
    let conflicts: Vec<(String, String, Vec<String>)> = route_targets
        .iter()
        .filter_map(|(entrypoint_name, routing)| {
            let parent = routing.parent.as_ref()?;
            let standalone = caps.iter().find(|cap| {
                cap.name == *entrypoint_name
                    && !cap.is_route_target()
                    && cap
                        .host_visibility
                        .iter()
                        .any(|visibility| visibility.status == HostVisibilityStatus::Visible)
            })?;
            let hosts = standalone
                .host_visibility
                .iter()
                .filter(|visibility| visibility.status == HostVisibilityStatus::Visible)
                .map(|visibility| visibility.host.clone())
                .collect();
            Some((entrypoint_name.clone(), parent.name.clone(), hosts))
        })
        .collect();

    for (entrypoint_name, parent_name, hosts) in conflicts {
        if let Some(parent) = caps.iter_mut().find(|cap| cap.name == parent_name) {
            for visibility in &mut parent.host_visibility {
                if hosts.iter().any(|host| host == &visibility.host) {
                    visibility.status = HostVisibilityStatus::Degraded;
                    visibility.evidence.push(format!(
                        "unexpected standalone entrypoint '{entrypoint_name}' is also visible; invoke it only through parent '{parent_name}'"
                    ));
                }
            }
            parent.health_status = HealthStatus::Degraded;
            parent.risk_notes.push(format!(
                "Internal entrypoint '{entrypoint_name}' is exposed as a standalone host skill."
            ));
        }
        if let Some(standalone) = caps.iter_mut().find(|cap| cap.name == entrypoint_name) {
            standalone.risk_notes.push(format!(
                "Unexpected standalone exposure: this entrypoint belongs to parent '{parent_name}'."
            ));
        }
    }
}

/// Skill-path visibility for a host: `<skills_dir>/<name>/SKILL.md`,
/// symlink-aware. Distinguishes loadable, present-but-not-loadable, dangling
/// symlink, and absent. Works for any host's skills dir (Claude / Codex).
pub(in super::super) fn skill_path_visibility(
    host: &str,
    skills_dir: &Path,
    name: &str,
    canonical_source: Option<&Path>,
) -> HostVisibility {
    let mut evidence = Vec::new();
    let v = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };

    // Refuse to resolve a host path from an unsafe name — a name with `/`, `..`,
    // or an absolute prefix could otherwise stat outside the skills directory.
    if !is_safe_path_component(name) {
        evidence.push(format!(
            "unsafe capability name '{name}' — refusing to resolve a host skill path"
        ));
        return v(HostVisibilityStatus::Degraded, evidence);
    }

    let skill_dir = skills_dir.join(name);
    let skill_md = skill_dir.join("SKILL.md");
    let link_meta = std::fs::symlink_metadata(&skill_dir);

    // Detect a dangling symlink before following it.
    if let Ok(meta) = &link_meta {
        if meta.file_type().is_symlink() {
            if std::fs::metadata(&skill_dir).is_err() {
                evidence.push(format!(
                    "dangling symlink (target missing): {}",
                    skill_dir.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
            evidence.push(format!(
                "skill dir is a symlink with a resolving target: {}",
                skill_dir.display()
            ));
        }
    }

    if !skill_dir.exists() {
        evidence.push(format!("not found under {}", skill_dir.display()));
        return v(HostVisibilityStatus::NotVisible, evidence);
    }
    if let Some(canonical) = canonical_source {
        let Some(meta) = link_meta.ok() else {
            evidence.push(format!(
                "host entry metadata unreadable: {}",
                skill_dir.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        };
        if !meta.file_type().is_symlink() {
            evidence.push(format!(
                "host entry is not a thin-index symlink to the AGS canonical body: {}",
                skill_dir.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        }
        let real_entry = match std::fs::canonicalize(&skill_dir) {
            Ok(p) => p,
            Err(e) => {
                evidence.push(format!(
                    "host thin index target is not canonicalizable: {} ({e})",
                    skill_dir.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
        };
        let real_canonical = match std::fs::canonicalize(canonical) {
            Ok(p) => p,
            Err(e) => {
                evidence.push(format!(
                    "AGS canonical source is not canonicalizable: {} ({e})",
                    canonical.display()
                ));
                return v(HostVisibilityStatus::Degraded, evidence);
            }
        };
        let Some(match_kind) = thin_index_target_match(&real_entry, &real_canonical) else {
            evidence.push(format!(
                "host thin index points to {}, expected AGS canonical {}",
                real_entry.display(),
                real_canonical.display()
            ));
            return v(HostVisibilityStatus::Degraded, evidence);
        };
        evidence.push(format!(
            "thin index resolves to {match_kind}: {}",
            real_entry.display()
        ));
    }
    if !skill_md.is_file() {
        evidence.push(format!(
            "dir present but SKILL.md missing: {}",
            skill_md.display()
        ));
        return v(HostVisibilityStatus::NotVisible, evidence);
    }
    match std::fs::read_to_string(&skill_md) {
        Ok(text) => {
            let (parsed_name, _desc) = crate::skill_body::parse_front_matter(&text);
            match parsed_name.as_deref().map(str::trim) {
                None => {
                    evidence.push(format!(
                        "SKILL.md present but front-matter not parseable: {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Degraded, evidence)
                }
                // The host loads skills by their front-matter `name`. A file at
                // the expected path whose declared name differs is NOT the
                // capability the operator thinks is installed — do not pass it.
                Some(found) if found != name => {
                    evidence.push(format!(
                        "SKILL.md name mismatch: declares '{found}' but expected '{name}' at {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Degraded, evidence)
                }
                Some(_) => {
                    evidence.push(format!(
                        "SKILL.md present and front-matter name matches: {}",
                        skill_md.display()
                    ));
                    v(HostVisibilityStatus::Visible, evidence)
                }
            }
        }
        Err(e) => {
            evidence.push(format!("SKILL.md unreadable: {} ({e})", skill_md.display()));
            v(HostVisibilityStatus::Degraded, evidence)
        }
    }
}

pub(in super::super) fn thin_index_target_match(
    real_entry: &Path,
    real_canonical: &Path,
) -> Option<&'static str> {
    if real_entry == real_canonical {
        return Some("AGS canonical body");
    }
    if same_private_stable_suite_path(real_entry, real_canonical) {
        return Some("AGS stable/private runtime twin");
    }
    None
}

pub(in super::super) fn same_private_stable_suite_path(
    real_entry: &Path,
    real_canonical: &Path,
) -> bool {
    let Some((entry_suite, entry_rel)) = split_suite_runtime_path(real_entry) else {
        return false;
    };
    let Some((canonical_suite, canonical_rel)) = split_suite_runtime_path(real_canonical) else {
        return false;
    };

    entry_suite != canonical_suite && entry_rel == canonical_rel
}

pub(in super::super) fn split_suite_runtime_path(path: &Path) -> Option<(&'static str, PathBuf)> {
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

    suite.and_then(|found| {
        if rel.components().next().is_some() {
            Some((found, rel))
        } else {
            None
        }
    })
}

/// MCP-registration visibility for a host via its cached `<host> mcp list`.
pub(in super::super) fn host_mcp_visibility(
    host: &str,
    name: &str,
    probe: Option<&HostMcpProbe>,
) -> HostVisibility {
    let v = |status, evidence| HostVisibility {
        host: host.to_string(),
        supported: true,
        status,
        evidence,
    };
    let Some(probe) = probe else {
        return v(
            HostVisibilityStatus::Degraded,
            vec![format!(
                "no MCP probe available for host '{host}' (degraded)."
            )],
        );
    };
    if probe.status != ags_host_integration::HostProbeStatus::Ready {
        return v(
            HostVisibilityStatus::Degraded,
            vec![format!(
                "{} reported {:?}: {}",
                probe.evidence_source, probe.status, probe.evidence
            )],
        );
    }
    match probe.find(name) {
        Some(entry) => v(
            HostVisibilityStatus::Visible,
            vec![format!(
                "reported by {} (enabled/connected: {})",
                probe.evidence_source, entry.active
            )],
        ),
        None => v(
            HostVisibilityStatus::NotVisible,
            vec![format!("'{name}' not found in {}", probe.evidence_source)],
        ),
    }
}

pub(in super::super) fn host_cli_visibility(host: &str, name: &str) -> HostVisibility {
    match ags_platform::find_in_path(name) {
        Some(path) => HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::Visible,
            evidence: vec![format!("external CLI is executable at {}", path.display())],
        },
        None => HostVisibility {
            host: host.to_string(),
            supported: true,
            status: HostVisibilityStatus::NotVisible,
            evidence: vec![format!("external CLI '{name}' was not found in PATH")],
        },
    }
}

/// Derive runtime health across the probed hosts; kept distinct from host
/// visibility and conservative — never Healthy without positive evidence, and
/// live external endpoints (e.g. Feishu) are only ever a degraded observation.
pub(in super::super) fn derive_health(
    kind: &ManagedKind,
    name: &str,
    host_vis: &[HostVisibility],
    probes: &[(String, HostMcpProbe)],
    cli_backed_external: bool,
) -> HealthStatus {
    if cli_backed_external {
        return HealthStatus::Degraded;
    }
    match kind {
        ManagedKind::Skill => {
            if host_vis
                .iter()
                .any(|v| v.status == HostVisibilityStatus::Visible)
            {
                HealthStatus::Healthy
            } else if host_vis
                .iter()
                .any(|v| v.status == HostVisibilityStatus::Degraded)
            {
                HealthStatus::Degraded
            } else {
                HealthStatus::Unknown
            }
        }
        ManagedKind::Mcp | ManagedKind::SuiteInterface => {
            let mut any_connected = false;
            let mut any_present = false;
            for (_, p) in probes {
                if p.status != ags_host_integration::HostProbeStatus::Ready {
                    continue;
                }
                if let Some(entry) = p.find(name) {
                    any_present = true;
                    any_connected |= entry.active;
                }
            }
            if any_connected {
                HealthStatus::Healthy
            } else if any_present {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Unknown
            }
        }
        ManagedKind::CliBacked => HealthStatus::Unknown,
    }
}
