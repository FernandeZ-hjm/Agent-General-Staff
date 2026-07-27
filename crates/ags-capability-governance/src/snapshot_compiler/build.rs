use super::*;
use super::{availability::*, model::*, source::*};
use crate::hashing::unix_timestamp;

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
        .map_err(SnapshotBuildError::Overlay)?;
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
    let registry_active_since =
        std::fs::metadata(manifest_root.join("manifests/skills-registry.yaml"))
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
    let overlay = load_user_overlay(runtime_home).map_err(SnapshotBuildError::Overlay)?;
    let source_registry =
        load_user_source_registry(runtime_home).map_err(SnapshotBuildError::Overlay)?;
    let imported_targets: HashMap<_, _> = source_registry
        .entries
        .iter()
        .map(|source| (source.skill_id.as_str(), source.target_hosts.as_slice()))
        .collect();
    let relevant_overlay_entries = overlay
        .entries
        .iter()
        .filter(|entry| {
            imported_targets
                .get(entry.skill_id.as_str())
                .is_none_or(|hosts| hosts.iter().any(|host| host == active_host))
        })
        .collect::<Vec<_>>();
    let overlay_bytes = serde_json::to_vec(&relevant_overlay_entries)
        .map_err(|error| SnapshotBuildError::Overlay(error.to_string()))?;
    let overlay_modified_since = std::fs::metadata(overlay_path(runtime_home))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    let overlay_hash = if overlay_bytes.is_empty() {
        sha256(b"empty-user-overlay")
    } else {
        sha256(&overlay_bytes)
    };
    let mut relevant_sources = source_registry
        .entries
        .iter()
        .filter(|source| source.target_hosts.iter().any(|host| host == active_host))
        .cloned()
        .collect::<Vec<_>>();
    for source in &mut relevant_sources {
        source.target_hosts = vec![active_host.to_string()];
    }
    relevant_sources.sort_by(|left, right| left.skill_id.cmp(&right.skill_id));
    let source_registry_bytes = serde_json::to_vec(&relevant_sources)
        .map_err(|error| SnapshotBuildError::Overlay(error.to_string()))?;
    let (auth_states, auth_hash) = load_auth_states(runtime_home, active_host);
    let usage_events = load_usage_events(runtime_home, active_host);
    let overlay_receipts = load_overlay_mutation_receipts(runtime_home).unwrap_or_default();
    let now_unix = unix_timestamp();

    let metadata: HashMap<_, _> = registry_document
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill))
        .collect();
    let overlay_entries: HashMap<_, _> = overlay
        .entries
        .iter()
        .map(|entry| (entry.skill_id.as_str(), entry))
        .collect();
    let routes_by_skill = routes_by_skill(&registry_document.demand_routes);
    let official_ids: HashSet<_> = registry_document
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    let imported_ids: HashSet<_> = source_registry
        .entries
        .iter()
        .filter(|source| source.target_hosts.iter().any(|host| host == active_host))
        .map(|source| source.skill_id.as_str())
        .collect();
    if let Some(collision) = source_registry
        .entries
        .iter()
        .find(|source| official_ids.contains(source.skill_id.as_str()))
    {
        return Err(SnapshotBuildError::Overlay(format!(
            "user source {} collides with official registry precedence",
            collision.skill_id
        )));
    }

    let mut catalog = Vec::new();
    let mut active_skills = Vec::new();
    for capability in inventory.capabilities.iter().filter(|capability| {
        capability.kind == ManagedKind::Skill
            && capability.managed_status != ManagedStatus::RouteTarget
            && !imported_ids.contains(capability.name.as_str())
    }) {
        let official = official_ids.contains(capability.name.as_str());
        let registry = metadata.get(capability.name.as_str()).copied();
        let overlay_entry = if official {
            None
        } else {
            overlay_entries.get(capability.name.as_str()).copied()
        };
        let legacy_routes = routes_by_skill
            .get(capability.name.as_str())
            .cloned()
            .unwrap_or_default();
        let file_requires_auth = load_skill_file_metadata(manifest_root, capability).requires_auth;
        let auth_state = auth_state_for(
            registry
                .and_then(|item| item.routing.as_ref())
                .is_some_and(|routing| routing.requires_auth)
                || overlay_entry.is_some_and(|entry| entry.requires_auth)
                || file_requires_auth,
            auth_states.skills.get(&capability.name).copied(),
        );
        let mut card = skill_card(
            manifest_root,
            capability,
            registry,
            overlay_entry,
            &legacy_routes,
            auth_state,
        );
        let active_since = overlay_entry
            .filter(|entry| entry.state == OverlayEntryState::Active)
            .and_then(|_| {
                overlay_active_since(&overlay_receipts, &card.skill_id).or(overlay_modified_since)
            })
            .or_else(|| {
                (card.governance == GovernanceState::Active)
                    .then_some(registry_active_since)
                    .flatten()
            });
        card.activity = activity_for_skill(&card.skill_id, &usage_events, now_unix, active_since);
        if card.governance == GovernanceState::Active && card.availability.is_ready() {
            let invoke_hint = registry
                .and_then(|item| item.routing.as_ref())
                .map(|routing| routing.invoke_hint.clone())
                .filter(|hint| !hint.is_empty())
                .or_else(|| {
                    overlay_entry
                        .map(|entry| entry.invoke_hint.clone())
                        .filter(|hint| !hint.is_empty())
                })
                .unwrap_or_else(|| format!("[skill: {}]", capability.name));
            let mut allowed_entrypoints = card.entrypoints.clone();
            allowed_entrypoints.sort();
            allowed_entrypoints.dedup();
            active_skills.push(ActiveSkill {
                skill_id: card.skill_id.clone(),
                invoke_hint,
                allowed_entrypoints,
                intent_tags: card.intent_tags.clone(),
                legacy_demands: legacy_routes.iter().map(|route| route.demand).collect(),
                source_hash: card.source_hash.clone(),
            });
        }
        catalog.push(card);
    }

    let mut imported_body_hashes = Vec::new();
    for source in &source_registry.entries {
        let canonical = Path::new(&source.canonical_path);
        let actual_source_hash = hash_skill_source(canonical).ok();
        if !source.target_hosts.iter().any(|host| host == active_host) {
            continue;
        }
        imported_body_hashes.push(format!(
            "{}:{}",
            source.skill_id,
            actual_source_hash.as_deref().unwrap_or("unreadable")
        ));
        if official_ids.contains(source.skill_id.as_str()) {
            continue;
        }
        let overlay_entry = overlay_entries.get(source.skill_id.as_str()).copied();
        let ignored = overlay_entry.is_some_and(|entry| entry.state == OverlayEntryState::Ignored);
        let active = overlay_entry.is_some_and(|entry| entry.state == OverlayEntryState::Active);
        let canonical_present = canonical.join("SKILL.md").is_file();
        let visible = user_source_host_visible(host_home, active_host, source);
        let auth_state = auth_state_for(
            source.requires_auth,
            auth_states.skills.get(&source.skill_id).copied(),
        );
        let governance = if ignored {
            GovernanceState::Ignored
        } else if active {
            GovernanceState::Active
        } else {
            GovernanceState::Candidate
        };
        let mut reasons = Vec::new();
        if governance == GovernanceState::Candidate {
            reasons.push("candidate_requires_adoption".to_string());
        }
        if !canonical_present {
            reasons.push("canonical_missing".to_string());
        }
        if !visible {
            reasons.push("host_not_visible".to_string());
        }
        if matches!(auth_state, AuthState::Missing | AuthState::Unknown) {
            reasons.push("auth_required".to_string());
        }
        if actual_source_hash.as_deref() != Some(source.source_hash.as_str())
            || overlay_entry.is_some_and(|entry| entry.source_hash != source.source_hash)
        {
            reasons.push("source_hash_changed".to_string());
        }
        if source.summary.trim().is_empty()
            || source.intent_tags.is_empty()
            || overlay_entry.is_some_and(|entry| entry.invoke_hint.trim().is_empty())
        {
            reasons.push("metadata_incomplete".to_string());
        }
        reasons.sort();
        reasons.dedup();
        let availability = if governance == GovernanceState::Active && reasons.is_empty() {
            AvailabilityState::Ready
        } else {
            AvailabilityState::Unavailable {
                reason_codes: reasons.clone(),
            }
        };
        let mut intent_tags = source.intent_tags.clone();
        intent_tags.sort();
        intent_tags.dedup();
        let mut entrypoints = source.entrypoints.clone();
        entrypoints.sort();
        entrypoints.dedup();
        let mut card = SkillCard {
            skill_id: source.skill_id.clone(),
            display_name: source.display_name.clone(),
            summary: source.summary.clone(),
            intent_tags,
            positive_examples: Vec::new(),
            negative_examples: Vec::new(),
            entrypoints,
            routing_surface: SkillRoutingSurface::SkillTarget,
            routing_hint: None,
            source_kind: SkillSourceKind::External,
            governance,
            availability,
            reason_codes: reasons,
            requires_auth: source.requires_auth,
            auth_state,
            activity: ActivityState::Unobserved,
            version: source.audit_version.clone(),
            source_hash: actual_source_hash.unwrap_or_else(|| source.source_hash.clone()),
        };
        let active_since = overlay_entry
            .filter(|entry| entry.state == OverlayEntryState::Active)
            .and_then(|_| {
                overlay_active_since(&overlay_receipts, &card.skill_id).or(overlay_modified_since)
            });
        card.activity = activity_for_skill(&card.skill_id, &usage_events, now_unix, active_since);
        if card.governance == GovernanceState::Active && card.availability.is_ready() {
            let invoke_hint = overlay_entry
                .map(|entry| entry.invoke_hint.clone())
                .filter(|hint| !hint.is_empty())
                .unwrap_or_else(|| format!("[skill: {}]", source.skill_id));
            active_skills.push(ActiveSkill {
                skill_id: card.skill_id.clone(),
                invoke_hint,
                allowed_entrypoints: card.entrypoints.clone(),
                intent_tags: card.intent_tags.clone(),
                legacy_demands: Vec::new(),
                source_hash: card.source_hash.clone(),
            });
        }
        catalog.push(card);
    }

    let runtime_hash = sha256(
        format!(
            "{}\n{auth_hash}\n{}",
            inventory_snapshot_hash(&inventory),
            if source_registry_bytes.is_empty() {
                sha256(b"empty-user-source-registry")
            } else {
                sha256(
                    format!(
                        "{}\n{}",
                        sha256(&source_registry_bytes),
                        imported_body_hashes.join("\n")
                    )
                    .as_bytes(),
                )
            }
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
        overlay_hash,
        runtime_hash,
        catalog,
        third_party.source.clone(),
        third_party.content_hash.clone(),
        third_party_catalog,
        active_skills,
    )
    .map_err(SnapshotBuildError::Resolve)
}
