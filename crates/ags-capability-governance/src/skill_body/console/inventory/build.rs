use super::*;

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
    let scan = crate::skill_body::scan_skills(&ctx.repo_root);
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
    let inv = crate::skill_body::scan_skill_inventory(&ctx.repo_root);
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
