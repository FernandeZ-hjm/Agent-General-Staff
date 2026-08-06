use super::*;

pub fn build_inventory(ctx: &ConsoleContext, hosts: &[&str]) -> ManagedInventoryResult {
    let hosts: Vec<String> = if hosts.is_empty() {
        supported_skill_hosts()
            .into_iter()
            .map(str::to_string)
            .collect()
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
    let mut routing_meta = read_routing_metadata(&ctx.repo_root);

    // 1. Suite-managed skills from the static suite manifest.
    let scan = crate::skill_body::scan_skills(&ctx.repo_root);
    let mut known_skill_names: Vec<String> = Vec::new();
    for s in &scan.skills {
        known_skill_names.push(s.name.clone());
        let managed_status = match s.profile.as_str() {
            "required" | "optional" | "personal" => ManagedStatus::SuiteManaged,
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
            risk_notes,
            routing: None,
        });
    }

    // 1.4. User-installed Skill bodies. InstalledSkillIndex is the machine
    // truth; filesystem coincidence and catalog membership never create an
    // installed or routable capability.
    match crate::skill_adoption::load_installed_skills(&ctx.runtime_home) {
        Ok(index) => {
            for record in index.skills.values() {
                if known_skill_names
                    .iter()
                    .any(|name| name == &record.skill_id)
                {
                    continue;
                }
                known_skill_names.push(record.skill_id.clone());
                let body = crate::skill_adoption::body_path(&ctx.runtime_home, record);
                let body_present = body.join("SKILL.md").is_file();
                let body_hash_matches = body_present
                    && crate::hash_skill_source(&body)
                        .is_ok_and(|actual| actual == record.source_hash);
                let mut risk_notes = vec![
                    "Installed Skill; AGS owns its immutable body, provenance, host indexes and snapshot activation."
                        .to_string(),
                ];
                if !body_present {
                    risk_notes.push("InstalledSkillIndex body is missing SKILL.md.".to_string());
                } else if !body_hash_matches {
                    risk_notes.push(
                        "InstalledSkillIndex body hash does not match its immutable revision."
                            .to_string(),
                    );
                }

                routing_meta.map.insert(
                    record.skill_id.clone(),
                    RoutingMetadata {
                        intent_tags: record.intent_tags.clone(),
                        scope_tags: Vec::new(),
                        mutation_surface: MutationSurface::ReadOnly,
                        requires_auth: record.requires_auth,
                        auth_kind: None,
                        cost_class: CostClass::Free,
                        invoke_hint: record.invoke_hint.clone(),
                        route_priority: default_route_priority(),
                        route_state: RouteState::Routable,
                        capability_group: Vec::new(),
                        upstream_group: Some("installed-skill".to_string()),
                        examples: RouteExamples {
                            positive: record.positive_examples.clone(),
                            negative: record.negative_examples.clone(),
                        },
                        parent: None,
                        entrypoint: None,
                    },
                );

                let mut expected_hosts = record.target_hosts.clone();
                expected_hosts.sort();
                expected_hosts.dedup();
                caps.push(ManagedCapability {
                    kind: ManagedKind::Skill,
                    name: record.skill_id.clone(),
                    source: Some(body.to_string_lossy().to_string()),
                    profile: None,
                    managed_status: ManagedStatus::Governed,
                    registry_status: RegistryStatus::Registered,
                    canonical_present: body_hash_matches,
                    expected_hosts,
                    host_visibility: Vec::new(),
                    health_status: HealthStatus::Unknown,
                    risk_notes,
                    routing: None,
                });
            }
        }
        Err(error) => routing_meta
            .parse_failures
            .push(format!("installed-skill-index: {error}")),
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

    // 3. Catalog MCP identities + the bundled AGS suite interface. Static
    // declarations never manufacture third-party installation state.
    for e in read_mcp_inventory_sources(&ctx.repo_root) {
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
        // Only the suite interface can carry expected Host projections. A
        // third-party MCP becomes available solely through positive Host probe
        // evidence, never because a catalog entry exists.
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
                .filter(|candidate| host_skills_subdir(candidate).is_some())
                .cloned()
                .collect()
        };
        expected_hosts.sort();
        expected_hosts.dedup();
        caps.push(ManagedCapability {
            kind,
            name: e.name.clone(),
            source: Some(e.declaration_source.to_string()),
            profile: None,
            managed_status,
            registry_status: RegistryStatus::Registered,
            // The MCP definition in the registry IS the canonical body.
            canonical_present: true,
            expected_hosts,
            host_visibility: Vec::new(),
            health_status: HealthStatus::Unknown,
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
    //    actions, never adopted/synced. Capability Resolver dereferences their
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
        note: "Read-only inventory. Third-party capabilities are opt-in; AGS never silently bundles or installs. Add or update a reviewed source only through an explicit release/setup workflow, then refresh the host's single static snapshot and verify it.".to_string(),
        routing_parse_failures: routing_meta.parse_failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_adoption::{
        body_path, installed_skill_index_path, BodyRevision, CatalogReviewStatus,
        InstalledSkillIndex, InstalledSkillMetadata, InstalledSkillRecord, SourceSpec,
        UpdatePolicy,
    };
    use std::collections::BTreeMap;

    struct NoProcessRunner;

    impl CommandRunner for NoProcessRunner {
        fn run(&self, _spec: &ags_host_integration::McpProbeSpec) -> CommandOutcome {
            CommandOutcome::Unavailable
        }
    }

    #[test]
    fn private_adopted_skill_is_governed_in_unified_inventory() {
        let temp = tempfile::tempdir().unwrap();
        let repo_root = temp.path().join("repo");
        let home = temp.path().join("home");
        let runtime_home = temp.path().join("runtime");
        std::fs::create_dir_all(&repo_root).unwrap();
        std::fs::create_dir_all(&home).unwrap();

        let mut record = InstalledSkillRecord {
            skill_id: "private-example".to_string(),
            source: "/audited/source/private-example".to_string(),
            source_hash: String::new(),
            license_path: "/audited/source/LICENSE".to_string(),
            license_hash: "sha256:license".to_string(),
            routing_metadata_path: None,
            routing_metadata_hash: None,
            body_revision: "revision-one".to_string(),
            summary: "Private adopted example".to_string(),
            intent_tags: vec!["private-example".to_string()],
            positive_examples: vec!["use private example".to_string()],
            negative_examples: vec!["do something else".to_string()],
            entrypoints: Vec::new(),
            invoke_hint: "[skill: private-example]".to_string(),
            requires_auth: false,
            version: "1.0.0".to_string(),
            target_hosts: vec!["codex".to_string()],
            source_spec: SourceSpec::local("/audited/source/private-example"),
            resolved_source: None,
            update_policy: UpdatePolicy::Notify,
            catalog_review: CatalogReviewStatus::Unreviewed,
            risk_findings: Vec::new(),
            body_revisions: Vec::new(),
            installed_at: 0,
        };
        let body = body_path(&runtime_home, &record);
        std::fs::create_dir_all(&body).unwrap();
        std::fs::write(
            body.join("SKILL.md"),
            "---\nname: private-example\ndescription: Private adopted example\n---\n",
        )
        .unwrap();
        record.source_hash = crate::hash_skill_source(&body).unwrap();
        record.body_revisions.push(BodyRevision {
            revision: record.body_revision.clone(),
            source_hash: record.source_hash.clone(),
            resolved_source: None,
            created_at: 0,
            metadata: InstalledSkillMetadata::from_record(&record),
        });

        let registry = InstalledSkillIndex {
            schema_version: crate::skill_adoption::INSTALLED_SKILL_INDEX_SCHEMA.to_string(),
            revision: 1,
            skills: BTreeMap::from([("private-example".to_string(), record)]),
        };
        let registry_file = installed_skill_index_path(&runtime_home);
        std::fs::create_dir_all(registry_file.parent().unwrap()).unwrap();
        std::fs::write(
            &registry_file,
            serde_json::to_vec_pretty(&registry).unwrap(),
        )
        .unwrap();

        let context = ConsoleContext::new_with_runtime_home(
            &repo_root,
            &home,
            &runtime_home,
            Box::new(NoProcessRunner),
        );
        let inventory = build_inventory(&context, &["codex"]);
        let capability = inventory
            .capabilities
            .iter()
            .find(|candidate| candidate.name == "private-example")
            .expect("private adopted skill must be present");

        assert_eq!(capability.managed_status, ManagedStatus::Governed);
        assert_eq!(capability.registry_status, RegistryStatus::Registered);
        assert!(capability.canonical_present);
        assert_eq!(capability.source.as_deref(), Some(body.to_str().unwrap()));
        assert_eq!(
            capability
                .routing
                .as_ref()
                .map(|routing| routing.invoke_hint.as_str()),
            Some("[skill: private-example]")
        );
    }
}
