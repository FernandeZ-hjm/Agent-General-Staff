use super::*;
use super::{availability::*, model::*};
use crate::skill_body::console::{EntrypointKind, ManagedInventoryResult};

#[derive(Debug, Default)]
struct SkillEntrypointProjection {
    entrypoints: Vec<String>,
    intent_tags: Vec<String>,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

#[derive(Default)]
struct McpToolProjection {
    tools: Vec<String>,
    intent_tags: Vec<String>,
    positive_examples: Vec<String>,
    negative_examples: Vec<String>,
}

fn skill_entrypoint_projections(
    inventory: &ManagedInventoryResult,
) -> HashMap<String, SkillEntrypointProjection> {
    let mut projections = HashMap::<String, SkillEntrypointProjection>::new();
    for capability in inventory
        .capabilities
        .iter()
        .filter(|capability| capability.is_route_target())
    {
        let Some(routing) = capability.routing.as_ref() else {
            continue;
        };
        let (Some(parent), Some(entrypoint)) = (&routing.parent, &routing.entrypoint) else {
            continue;
        };
        if parent.kind != ManagedKind::Skill
            || entrypoint.kind != EntrypointKind::Playbook
            || routing.route_state != RouteState::Routable
        {
            continue;
        }
        let projection = projections.entry(parent.name.clone()).or_default();
        projection.entrypoints.push(entrypoint.name.clone());
        projection.intent_tags.extend(routing.intent_tags.clone());
        projection
            .positive_examples
            .extend(routing.examples.positive.clone());
        projection
            .negative_examples
            .extend(routing.examples.negative.clone());
    }
    for projection in projections.values_mut() {
        projection.entrypoints.sort();
        projection.entrypoints.dedup();
        projection.intent_tags.sort();
        projection.intent_tags.dedup();
        projection.positive_examples.sort();
        projection.positive_examples.dedup();
        projection.negative_examples.sort();
        projection.negative_examples.dedup();
    }
    projections
}

fn mcp_tool_projections(inventory: &ManagedInventoryResult) -> HashMap<String, McpToolProjection> {
    let mut projections = HashMap::<String, McpToolProjection>::new();
    for capability in inventory
        .capabilities
        .iter()
        .filter(|capability| capability.is_route_target())
    {
        let Some(routing) = capability.routing.as_ref() else {
            continue;
        };
        let (Some(parent), Some(entrypoint)) = (&routing.parent, &routing.entrypoint) else {
            continue;
        };
        if parent.kind != ManagedKind::Mcp
            || entrypoint.kind != EntrypointKind::Tool
            || routing.route_state != RouteState::Routable
        {
            continue;
        }
        let projection = projections.entry(parent.name.clone()).or_default();
        projection.tools.push(entrypoint.name.clone());
        projection.intent_tags.extend(routing.intent_tags.clone());
        projection
            .positive_examples
            .extend(routing.examples.positive.clone());
        projection
            .negative_examples
            .extend(routing.examples.negative.clone());
    }
    for projection in projections.values_mut() {
        projection.tools.sort();
        projection.tools.dedup();
        projection.intent_tags.sort();
        projection.intent_tags.dedup();
        projection.positive_examples.sort();
        projection.positive_examples.dedup();
        projection.negative_examples.sort();
        projection.negative_examples.dedup();
    }
    projections
}

fn route_state_name(state: RouteState) -> &'static str {
    match state {
        RouteState::Routable => "routable",
        RouteState::NotRoutable => "not_routable",
        RouteState::Retired => "retired",
    }
}

fn mutation_surface_name(surface: MutationSurface) -> &'static str {
    match surface {
        MutationSurface::ReadOnly => "read_only",
        MutationSurface::LocalWrite => "local_write",
        MutationSurface::ExternalWrite => "external_write",
    }
}

fn health_status_name(status: &HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unknown => "unknown",
        HealthStatus::Unhealthy => "unhealthy",
    }
}

pub fn build_capability_snapshot(
    manifest_root: &Path,
    active_host: &str,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    build_capability_snapshot_with_runtime_home(
        manifest_root,
        active_host,
        &ags_platform::runtime_home(),
    )
}

pub fn build_capability_snapshot_with_runtime_home(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    build_capability_snapshot_with_live_roots(manifest_root, active_host, runtime_home, &host_home)
}

/// Build an explicit-refresh snapshot with live, read-only host MCP discovery.
///
/// Runtime request paths never call this function. Setup/update and the
/// explicit `capability snapshot --write` command use it to seal current host
/// registration evidence into the static Skill/MCP indexes.
pub fn build_capability_snapshot_with_live_roots(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    build_capability_snapshot_with_live_roots_and_runner(
        manifest_root,
        active_host,
        runtime_home,
        host_home,
        Box::new(SystemCommandRunner),
    )
}

/// Scan the machine and resolve immutable manifests once, then compile one
/// host-specific snapshot per requested Host from that shared observation.
/// This is the canonical setup/update path for multi-Host activation.
pub fn build_capability_snapshots_with_live_roots(
    manifest_root: &Path,
    active_hosts: &[String],
    runtime_home: &Path,
    host_home: &Path,
) -> Result<Vec<(String, HostCapabilitySnapshot)>, SnapshotBuildError> {
    let third_party = crate::third_party_manifest::resolve_third_party_manifest(manifest_root)
        .map_err(SnapshotBuildError::Manifest)?;
    let context = ConsoleContext::new_with_runtime_home(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        runtime_home.to_path_buf(),
        Box::new(SystemCommandRunner),
    );
    let host_refs = active_hosts.iter().map(String::as_str).collect::<Vec<_>>();
    let inventory = build_inventory(&context, &host_refs);
    let registry_document =
        load_registry_document(manifest_root).map_err(SnapshotBuildError::Registry)?;
    let registry_bytes = std::fs::read(manifest_root.join("manifests/skills-registry.yaml"))
        .map_err(SnapshotBuildError::Read)?;
    active_hosts
        .iter()
        .map(|host| {
            compile_snapshot_from_inventory(
                manifest_root,
                host,
                runtime_home,
                &context,
                &third_party,
                &inventory,
                &registry_document,
                &registry_bytes,
            )
            .map(|snapshot| (host.clone(), snapshot))
        })
        .collect()
}

/// Validate and publish one already-compiled snapshot set. Every candidate is
/// integrity-checked and serialized before the first pointer is replaced, so
/// suite activation and third-party Skill transactions cannot drift into
/// separate snapshot writers.
pub fn publish_capability_snapshots(
    runtime_home: &Path,
    snapshots: Vec<(String, HostCapabilitySnapshot)>,
) -> Result<BTreeMap<String, String>, String> {
    let prepared = snapshots
        .into_iter()
        .map(|(host, snapshot)| {
            snapshot
                .validate_integrity(&host)
                .map_err(|error| format!("invalid `{host}` candidate snapshot: {error:?}"))?;
            let hash = snapshot.snapshot_hash.clone();
            let mut bytes = serde_json::to_vec_pretty(&snapshot)
                .map_err(|error| format!("cannot serialize `{host}` snapshot: {error}"))?;
            bytes.push(b'\n');
            Ok((host, hash, bytes))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut hashes = BTreeMap::new();
    for (host, hash, bytes) in prepared {
        let path = crate::snapshot_path(runtime_home, &host);
        ags_platform::atomic_write(&path, &bytes)
            .map_err(|error| format!("cannot publish `{host}` snapshot: {error}"))?;
        hashes.insert(host, hash);
    }
    Ok(hashes)
}

/// Rebuild a live snapshot while resolving workspace-scoped host registration
/// from the workspace being inspected, not from the caller's current directory.
pub fn build_capability_snapshot_with_live_roots_at(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
    workspace: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    build_capability_snapshot_with_live_roots_and_runner(
        manifest_root,
        active_host,
        runtime_home,
        host_home,
        Box::new(WorkspaceCommandRunner {
            current_dir: workspace.to_path_buf(),
        }),
    )
}

struct WorkspaceCommandRunner {
    current_dir: PathBuf,
}

impl CommandRunner for WorkspaceCommandRunner {
    fn run(&self, spec: &ags_host_integration::McpProbeSpec) -> CommandOutcome {
        SystemCommandRunner.run_in(spec, &self.current_dir)
    }
}

fn build_capability_snapshot_with_live_roots_and_runner(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
    runner: Box<dyn CommandRunner>,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let third_party = crate::third_party_manifest::resolve_third_party_manifest(manifest_root)
        .map_err(SnapshotBuildError::Manifest)?;
    let context = ConsoleContext::new_with_runtime_home(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        runtime_home.to_path_buf(),
        runner,
    );
    build_capability_snapshot_with_context_and_manifest(
        manifest_root,
        active_host,
        runtime_home,
        &context,
        &third_party,
    )
}

pub fn write_capability_snapshot_with_roots(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
) -> Result<HostCapabilitySnapshot, String> {
    let snapshot =
        build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, host_home)
            .map_err(|error| format!("capability snapshot build failed: {error:?}"))?;
    let serialized = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("capability snapshot serialization failed: {error}"))?;
    ags_platform::atomic_write(
        &snapshot_path(runtime_home, active_host),
        (serialized + "\n").as_bytes(),
    )?;
    Ok(snapshot)
}

/// Build a snapshot with explicit machine roots and a no-process discovery
/// runner. This hermetic seam is used by tests and offline onboarding
/// assessment. Explicit setup/update refreshes use
/// [`build_capability_snapshot_with_live_roots`] instead.
pub fn build_capability_snapshot_with_roots(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let third_party = crate::third_party_manifest::resolve_third_party_manifest(manifest_root)
        .map_err(SnapshotBuildError::Manifest)?;
    build_capability_snapshot_with_roots_and_manifest(
        manifest_root,
        active_host,
        runtime_home,
        host_home,
        &third_party,
    )
}

/// Build a capability snapshot against one already-resolved third-party
/// manifest. Onboarding uses this seam so its install plan and the host's
/// natural-language routing catalog share one immutable manifest hash.
pub fn build_capability_snapshot_with_roots_and_manifest(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
    third_party: &crate::third_party_manifest::ManifestResolution,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let context = ConsoleContext::new_with_runtime_home(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        runtime_home.to_path_buf(),
        Box::new(NoProcessDiscovery),
    );
    build_capability_snapshot_with_context_and_manifest(
        manifest_root,
        active_host,
        runtime_home,
        &context,
        third_party,
    )
}

fn build_capability_snapshot_with_context_and_manifest(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    context: &ConsoleContext,
    third_party: &crate::third_party_manifest::ManifestResolution,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let inventory = build_inventory(context, &[active_host]);
    let registry_document =
        load_registry_document(manifest_root).map_err(SnapshotBuildError::Registry)?;
    let registry_bytes = std::fs::read(manifest_root.join("manifests/skills-registry.yaml"))
        .map_err(SnapshotBuildError::Read)?;
    compile_snapshot_from_inventory(
        manifest_root,
        active_host,
        runtime_home,
        context,
        third_party,
        &inventory,
        &registry_document,
        &registry_bytes,
    )
}

#[allow(clippy::too_many_arguments)]
fn compile_snapshot_from_inventory(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    context: &ConsoleContext,
    third_party: &crate::third_party_manifest::ManifestResolution,
    inventory: &ManagedInventoryResult,
    registry_document: &RegistryDocument,
    registry_bytes: &[u8],
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let (auth_states, auth_hash) = load_auth_states(runtime_home, active_host);

    let metadata: HashMap<_, _> = registry_document
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill))
        .collect();
    let official_ids = metadata
        .keys()
        .map(|name| (*name).to_string())
        .collect::<HashSet<_>>();
    let entrypoint_projections = skill_entrypoint_projections(inventory);
    let mcp_projections = mcp_tool_projections(inventory);

    let mut catalog = Vec::new();
    let mut active_skills = Vec::new();
    for capability in inventory.capabilities.iter().filter(|capability| {
        capability.kind == ManagedKind::Skill
            && capability.managed_status != ManagedStatus::RouteTarget
    }) {
        let registry = metadata.get(capability.name.as_str()).copied();
        let file_requires_auth = load_skill_file_metadata(manifest_root, capability).requires_auth;
        let auth_state = auth_state_for(
            registry
                .and_then(|item| item.routing.as_ref())
                .is_some_and(|routing| routing.requires_auth)
                || file_requires_auth,
            auth_states.skills.get(&capability.name).copied(),
        );
        let mut card = skill_card(manifest_root, capability, registry, auth_state, active_host);
        if let Some(projection) = entrypoint_projections.get(&capability.name) {
            card.entrypoints.extend(projection.entrypoints.clone());
            card.entrypoints.sort();
            card.entrypoints.dedup();
            card.intent_tags.extend(projection.intent_tags.clone());
            card.intent_tags.sort();
            card.intent_tags.dedup();
            card.positive_examples
                .extend(projection.positive_examples.clone());
            card.positive_examples.sort();
            card.positive_examples.dedup();
            card.negative_examples
                .extend(projection.negative_examples.clone());
            card.negative_examples.sort();
            card.negative_examples.dedup();
        }
        if card.governance == GovernanceState::Active && card.availability.is_ready() {
            let invoke_hint = registry
                .and_then(|item| item.routing.as_ref())
                .map(|routing| routing.invoke_hint.clone())
                .filter(|hint| !hint.is_empty())
                .unwrap_or_else(|| format!("[skill: {}]", capability.name));
            let mut allowed_entrypoints = card.entrypoints.clone();
            allowed_entrypoints.sort();
            allowed_entrypoints.dedup();
            active_skills.push(ActiveSkill {
                skill_id: card.skill_id.clone(),
                invoke_hint,
                allowed_entrypoints,
                intent_tags: card.intent_tags.clone(),
                source_hash: card.source_hash.clone(),
            });
        }
        catalog.push(card);
    }

    let installed_projection = crate::skill_adoption::project_installed_skills(
        runtime_home,
        &context.home,
        active_host,
        &official_ids,
    )
    .map_err(SnapshotBuildError::Manifest)?;
    let installed_skill_catalog = installed_projection.cards.clone();
    for card in installed_projection.cards {
        catalog.retain(|candidate| candidate.skill_id != card.skill_id);
        active_skills.retain(|candidate| candidate.skill_id != card.skill_id);
        catalog.push(card);
    }
    active_skills.extend(installed_projection.active);

    let mut mcp_catalog = Vec::new();
    let mut active_mcps = Vec::new();
    for capability in inventory.capabilities.iter().filter(|capability| {
        capability.kind == ManagedKind::Mcp
            && capability.managed_status != ManagedStatus::RouteTarget
    }) {
        let Some(routing) = capability.routing.as_ref() else {
            continue;
        };
        let visible = capability.host_visibility.iter().any(|visibility| {
            visibility.host == active_host
                && visibility.supported
                && visibility.status == HostVisibilityStatus::Visible
        });
        let auth_state = if routing.requires_auth {
            if capability.health_status == HealthStatus::Healthy {
                AuthState::Satisfied
            } else {
                AuthState::Unknown
            }
        } else {
            AuthState::NotRequired
        };
        let mut reasons = Vec::new();
        if routing.route_state != RouteState::Routable {
            reasons.push("route_state_not_routable".to_string());
        }
        if !visible {
            reasons.push("host_not_visible".to_string());
        }
        match capability.health_status {
            HealthStatus::Healthy => {}
            HealthStatus::Degraded => reasons.push("health_degraded".to_string()),
            HealthStatus::Unknown => reasons.push("health_unknown".to_string()),
            HealthStatus::Unhealthy => reasons.push("health_unhealthy".to_string()),
        }
        if routing.requires_auth && auth_state != AuthState::Satisfied {
            reasons.push("auth_state_unknown".to_string());
        }
        reasons.sort();
        reasons.dedup();
        let availability = if reasons.is_empty() {
            AvailabilityState::Ready
        } else {
            AvailabilityState::Unavailable {
                reason_codes: reasons.clone(),
            }
        };
        let projection = mcp_projections.get(&capability.name);
        let mut tools = projection
            .map(|projection| projection.tools.clone())
            .unwrap_or_default();
        tools.sort();
        tools.dedup();
        let mut intent_tags = routing.intent_tags.clone();
        if let Some(projection) = projection {
            intent_tags.extend(projection.intent_tags.clone());
        }
        intent_tags.sort();
        intent_tags.dedup();
        let mut positive_examples = routing.examples.positive.clone();
        let mut negative_examples = routing.examples.negative.clone();
        if let Some(projection) = projection {
            positive_examples.extend(projection.positive_examples.clone());
            negative_examples.extend(projection.negative_examples.clone());
        }
        positive_examples.sort();
        positive_examples.dedup();
        negative_examples.sort();
        negative_examples.dedup();
        let invoke_hint = if routing.invoke_hint.trim().is_empty() {
            format!("{} MCP", capability.name)
        } else {
            routing.invoke_hint.clone()
        };
        let mutation_surface = mutation_surface_name(routing.mutation_surface).to_string();
        let card = McpCard {
            mcp_id: capability.name.clone(),
            display_name: capability.name.clone(),
            summary: invoke_hint.clone(),
            intent_tags: intent_tags.clone(),
            positive_examples,
            negative_examples,
            tools: tools.clone(),
            invoke_hint: invoke_hint.clone(),
            route_state: route_state_name(routing.route_state).to_string(),
            mutation_surface: mutation_surface.clone(),
            availability,
            reason_codes: reasons,
            requires_auth: routing.requires_auth,
            auth_state,
            health_status: health_status_name(&capability.health_status).to_string(),
        };
        if card.availability.is_ready() {
            active_mcps.push(ActiveMcp {
                mcp_id: card.mcp_id.clone(),
                invoke_hint,
                allowed_tools: tools,
                intent_tags,
                mutation_surface,
            });
        }
        mcp_catalog.push(card);
    }

    let runtime_hash = ags_platform::sha256(
        format!(
            "{}\n{auth_hash}\n{}",
            inventory_snapshot_hash(inventory),
            installed_projection.installed_skill_index_hash
        )
        .as_bytes(),
    );
    let active_profile = capability_profile(manifest_root);
    let third_party_catalog = third_party
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.applies_to(active_profile))
        .map(|capability| {
            let kind = capability.kind.as_str().to_string();
            let routing_surface = match capability.kind {
                crate::third_party_manifest::CapabilityKind::Skill => {
                    "exact-skill-target".to_string()
                }
                crate::third_party_manifest::CapabilityKind::Mcp => "host-native-mcp".to_string(),
                crate::third_party_manifest::CapabilityKind::Cli => {
                    "host-native-cli-or-governed-skill-wrapper".to_string()
                }
                crate::third_party_manifest::CapabilityKind::Hook => {
                    "host-event-only-not-natural-language".to_string()
                }
            };
            let (availability, reason_codes, auth_state, health_status) = third_party_availability(
                capability,
                active_host,
                &installed_skill_catalog,
                &inventory.capabilities,
            );
            let installation_state = match capability.kind {
                crate::third_party_manifest::CapabilityKind::Skill
                    if reason_codes
                        .iter()
                        .any(|reason| reason == "capability_not_installed") =>
                {
                    "not-installed"
                }
                crate::third_party_manifest::CapabilityKind::Skill if availability.is_ready() => {
                    "installed-snapshot-active"
                }
                crate::third_party_manifest::CapabilityKind::Skill => "installed-not-active",
                _ => "observed-runtime-state",
            };
            ThirdPartyCapabilityCard {
                capability_id: capability.id.clone(),
                kind,
                catalog_state: "recommendation-only".to_string(),
                installation_state: installation_state.to_string(),
                display_name: capability.name.clone(),
                purpose: capability.purpose.clone(),
                profiles: capability.profiles.clone(),
                required: capability.required,
                route_state: capability.routing.route_state.clone(),
                route_state_semantics: "post-activation-contract".to_string(),
                availability,
                reason_codes,
                requires_auth: capability.requires_auth,
                auth_state,
                health_status,
                invoke_hint: capability.routing.invoke_hint.clone(),
                intent_tags: capability.routing.intent_tags.clone(),
                positive_examples: capability.routing.positive_examples.clone(),
                negative_examples: capability.routing.negative_examples.clone(),
                routing_surface,
                hook_events: capability
                    .hook
                    .as_ref()
                    .map(|contract| contract.events.clone())
                    .unwrap_or_default(),
                source_version: capability.source.version.clone(),
            }
        })
        .collect();
    HostCapabilitySnapshot::new(
        active_host,
        ags_platform::sha256(registry_bytes),
        runtime_hash,
        catalog,
        mcp_catalog,
        third_party.source.clone(),
        third_party.content_hash.clone(),
        third_party_catalog,
        active_skills,
        active_mcps,
    )
    .map_err(SnapshotBuildError::Resolve)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConnectedCodexMcpRunner;

    impl CommandRunner for ConnectedCodexMcpRunner {
        fn run(&self, _spec: &ags_host_integration::McpProbeSpec) -> CommandOutcome {
            CommandOutcome::Ran {
                success: true,
                output:
                    "Name Command Status\ncontext7 context7 enabled\ncodegraph codegraph enabled\n"
                        .to_string(),
            }
        }
    }

    #[test]
    fn explicit_refresh_indexes_connected_mcp_servers_and_registered_tools() {
        let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base =
            std::env::temp_dir().join(format!("ags-mcp-snapshot-refresh-{}", std::process::id()));
        let runtime_home = base.join("runtime");
        let host_home = base.join("home");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&runtime_home).unwrap();
        std::fs::create_dir_all(&host_home).unwrap();
        let third_party =
            crate::third_party_manifest::resolve_third_party_manifest(&manifest_root).unwrap();
        let context = ConsoleContext::new_with_runtime_home(
            &manifest_root,
            &host_home,
            &runtime_home,
            Box::new(ConnectedCodexMcpRunner),
        );

        let snapshot = build_capability_snapshot_with_context_and_manifest(
            &manifest_root,
            "codex",
            &runtime_home,
            &context,
            &third_party,
        )
        .unwrap();

        let context7 = snapshot
            .mcp_catalog
            .iter()
            .find(|card| card.mcp_id == "context7")
            .unwrap();
        assert!(context7.availability.is_ready());
        assert_eq!(
            context7.tools,
            vec![
                "get-library-docs".to_string(),
                "resolve-library-id".to_string()
            ]
        );
        assert!(snapshot
            .active_mcps
            .iter()
            .any(|mcp| mcp.mcp_id == "context7"));
        assert!(!snapshot
            .active_mcps
            .iter()
            .any(|mcp| mcp.mcp_id == "evomap"));

        // A bundled suite body with the same id as a recommendation is not an
        // InstalledSkillRecord and must never make the catalog entry ready.
        let diagnosing_bugs = snapshot
            .third_party_catalog
            .iter()
            .find(|card| card.capability_id == "diagnosing-bugs")
            .unwrap();
        assert_eq!(diagnosing_bugs.catalog_state, "recommendation-only");
        assert_eq!(diagnosing_bugs.installation_state, "not-installed");
        assert_eq!(
            diagnosing_bugs.route_state_semantics,
            "post-activation-contract"
        );
        assert!(!diagnosing_bugs.availability.is_ready());
        assert!(diagnosing_bugs
            .reason_codes
            .contains(&"capability_not_installed".to_string()));

        let _ = std::fs::remove_dir_all(base);
    }
}
