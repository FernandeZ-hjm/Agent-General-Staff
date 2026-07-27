use super::*;
use super::{availability::*, model::*};

pub fn build_capability_snapshot(
    manifest_root: &Path,
    active_host: &str,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    build_capability_snapshot_with_runtime_home(manifest_root, active_host, &locate_runtime_home())
}

pub fn build_capability_snapshot_with_runtime_home(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> Result<HostCapabilitySnapshot, SnapshotBuildError> {
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, &host_home)
}

pub fn write_capability_snapshot_with_roots(
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
    host_home: &Path,
) -> Result<HostCapabilitySnapshot, String> {
    let snapshot =
        build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, host_home)
            .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
    let serialized = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
    write_private_atomic(
        &snapshot_path(runtime_home, active_host),
        (serialized + "\n").as_bytes(),
    )?;
    Ok(snapshot)
}

/// Build a snapshot with explicit machine roots and a no-process discovery
/// runner. This is the production seam used by routing as well as the test seam:
/// capability catalog generation never launches host CLIs.
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
    let context = ConsoleContext::new(
        manifest_root.to_path_buf(),
        host_home.to_path_buf(),
        Box::new(NoProcessDiscovery),
    );
    let inventory = build_inventory(&context, &[active_host]);
    let registry_document =
        load_registry_document(manifest_root).map_err(SnapshotBuildError::Registry)?;
    let registry_bytes = std::fs::read(manifest_root.join("manifests/skills-registry.yaml"))
        .map_err(SnapshotBuildError::Read)?;
    let (auth_states, auth_hash) = load_auth_states(runtime_home, active_host);

    let metadata: HashMap<_, _> = registry_document
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill))
        .collect();

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
        let card = skill_card(manifest_root, capability, registry, auth_state);
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

    let runtime_hash =
        sha256(format!("{}\n{auth_hash}", inventory_snapshot_hash(&inventory)).as_bytes());
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
                &catalog,
                &inventory.capabilities,
            );
            ThirdPartyCapabilityCard {
                capability_id: capability.id.clone(),
                kind,
                display_name: capability.name.clone(),
                purpose: capability.purpose.clone(),
                profiles: capability.profiles.clone(),
                required: capability.required,
                route_state: capability.routing.route_state.clone(),
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
        sha256(&registry_bytes),
        runtime_hash,
        catalog,
        third_party.source.clone(),
        third_party.content_hash.clone(),
        third_party_catalog,
        active_skills,
    )
    .map_err(SnapshotBuildError::Resolve)
}
