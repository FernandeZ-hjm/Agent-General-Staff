use super::*;
#[allow(unused_imports)]
use super::{
    authority::*, catalog::*, hashing::*, overlay_transaction::*, private_store::*, usage_ledger::*,
};
#[derive(Debug, Deserialize)]
pub(super) struct RegistryDocument {
    #[serde(default)]
    pub(super) skills: Vec<RegistrySkill>,
    #[serde(default)]
    demand_routes: Vec<DemandRoute>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RegistrySkill {
    pub(super) name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    routing: Option<RegistryRouting>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RegistryRouting {
    #[serde(default)]
    intent_tags: Vec<String>,
    #[serde(default)]
    requires_auth: bool,
    #[serde(default)]
    invoke_hint: String,
    #[serde(default)]
    route_state: RouteState,
    #[serde(default)]
    examples: RouteExamples,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct SkillFileMetadata {
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    intent_tags: Vec<String>,
    #[serde(default)]
    entrypoints: Vec<String>,
    #[serde(default)]
    invoke_hint: String,
    #[serde(default)]
    requires_auth: bool,
    #[serde(default)]
    version: String,
}

pub(super) fn capability_source_path(
    manifest_root: &Path,
    capability: &ManagedCapability,
) -> Option<PathBuf> {
    capability.source.as_deref().map(|source| {
        let path = Path::new(source);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            manifest_root.join(path)
        }
    })
}

pub(super) fn load_skill_file_metadata(
    manifest_root: &Path,
    capability: &ManagedCapability,
) -> SkillFileMetadata {
    let Some(path) = capability_source_path(manifest_root, capability) else {
        return SkillFileMetadata::default();
    };
    let skill_md = if path.is_dir() {
        path.join("SKILL.md")
    } else {
        path
    };
    load_skill_metadata_path(&skill_md)
}

pub(super) fn load_skill_metadata_path(skill_md: &Path) -> SkillFileMetadata {
    let Ok(content) = std::fs::read_to_string(skill_md) else {
        return SkillFileMetadata::default();
    };
    let Some(rest) = content.strip_prefix("---") else {
        return SkillFileMetadata::default();
    };
    let Some((frontmatter, _)) = rest.split_once("\n---") else {
        return SkillFileMetadata::default();
    };
    serde_yaml::from_str(frontmatter).unwrap_or_default()
}

pub(super) fn load_registry_document(root: &Path) -> Result<RegistryDocument, RegistryError> {
    let content = std::fs::read_to_string(root.join("manifests/skills-registry.yaml"))
        .map_err(RegistryError::Read)?;
    serde_yaml::from_str(&content).map_err(RegistryError::Parse)
}

pub fn load_demand_routes(root: &Path) -> Result<Vec<DemandRoute>, RegistryError> {
    load_registry_document(root).map(|document| document.demand_routes)
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthStateDocument {
    #[serde(default)]
    skills: BTreeMap<String, AuthState>,
}

pub(super) fn load_auth_states(runtime_home: &Path, host: &str) -> (AuthStateDocument, String) {
    let path = runtime_home
        .join("auth-state")
        .join(format!("{}.json", safe_host(host)));
    let Ok(bytes) = std::fs::read(path) else {
        return (AuthStateDocument::default(), sha256(b"missing-auth-state"));
    };
    let document = serde_json::from_slice(&bytes).unwrap_or_default();
    (document, sha256(&bytes))
}

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
    let third_party = ags_onboarding::manifest::resolve_third_party_manifest(manifest_root)
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
    third_party: &ags_onboarding::manifest::ManifestResolution,
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
                ags_onboarding::manifest::CapabilityKind::Skill => "exact-skill-target".to_string(),
                ags_onboarding::manifest::CapabilityKind::Mcp => "host-native-mcp".to_string(),
                ags_onboarding::manifest::CapabilityKind::Cli => {
                    "host-native-cli-or-governed-skill-wrapper".to_string()
                }
                ags_onboarding::manifest::CapabilityKind::Hook => {
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

pub(super) fn capability_profile(manifest_root: &Path) -> &'static str {
    let content =
        std::fs::read_to_string(manifest_root.join("manifests/suite.yaml")).unwrap_or_default();
    let value: serde_yaml::Value = serde_yaml::from_str(&content).unwrap_or_default();
    let suite = value.get("suite");
    let name = suite
        .and_then(|suite| suite.get("name"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default();
    let version = suite
        .and_then(|suite| suite.get("version"))
        .and_then(serde_yaml::Value::as_str)
        .unwrap_or_default();
    if name == "agent-general-staff" || version.ends_with("-public") {
        "public"
    } else {
        "private"
    }
}

pub(super) fn third_party_availability(
    capability: &ags_onboarding::manifest::ThirdPartyCapability,
    active_host: &str,
    catalog: &[SkillCard],
    inventory: &[ManagedCapability],
) -> (AvailabilityState, Vec<String>, AuthState, String) {
    use ags_onboarding::manifest::CapabilityKind;

    let mut reasons = Vec::new();
    let mut health_status = "unknown".to_string();
    let auth_state = if capability.requires_auth {
        AuthState::Unknown
    } else {
        AuthState::NotRequired
    };

    match capability.kind {
        CapabilityKind::Skill => {
            if let Some(card) = catalog.iter().find(|card| card.skill_id == capability.id) {
                reasons.extend(card.reason_codes.clone());
                health_status = if card.availability.is_ready() {
                    "healthy"
                } else {
                    "unavailable"
                }
                .to_string();
                if capability.requires_auth
                    && matches!(card.auth_state, AuthState::Missing | AuthState::Unknown)
                {
                    reasons.push("auth_required".to_string());
                }
                reasons.sort();
                reasons.dedup();
                return (
                    if card.availability.is_ready() && reasons.is_empty() {
                        AvailabilityState::Ready
                    } else {
                        AvailabilityState::Unavailable {
                            reason_codes: reasons.clone(),
                        }
                    },
                    reasons,
                    card.auth_state,
                    health_status,
                );
            }
            reasons.push("capability_not_installed".to_string());
        }
        CapabilityKind::Cli => {
            let command = capability.install.command.as_deref().unwrap_or_default();
            if command.is_empty() || !ags_platform::is_on_path(command) {
                reasons.push("command_not_on_path".to_string());
                health_status = "unavailable".to_string();
            } else if capability.requires_auth {
                reasons.push("auth_state_unknown".to_string());
                health_status = "unknown".to_string();
            } else {
                health_status = "healthy".to_string();
            }
        }
        CapabilityKind::Mcp => {
            let server_name = capability
                .mcp
                .as_ref()
                .map(|contract| contract.server_name.as_str())
                .unwrap_or(capability.id.as_str());
            if let Some(managed) = inventory.iter().find(|managed| {
                managed.kind == ManagedKind::Mcp
                    && (managed.name == capability.id || managed.name == server_name)
            }) {
                let visible = managed.host_visibility.iter().any(|visibility| {
                    visibility.host == active_host
                        && visibility.supported
                        && visibility.status == HostVisibilityStatus::Visible
                });
                if !visible {
                    reasons.push("host_not_visible".to_string());
                }
                health_status = match managed.health_status {
                    HealthStatus::Healthy => "healthy",
                    HealthStatus::Degraded => {
                        reasons.push("health_degraded".to_string());
                        "degraded"
                    }
                    HealthStatus::Unknown => {
                        reasons.push("health_unknown".to_string());
                        "unknown"
                    }
                    HealthStatus::Unhealthy => {
                        reasons.push("health_unhealthy".to_string());
                        "unhealthy"
                    }
                }
                .to_string();
            } else {
                reasons.push("host_not_visible".to_string());
                health_status = "unavailable".to_string();
            }
            if capability.requires_auth {
                reasons.push("auth_state_unknown".to_string());
            }
        }
        CapabilityKind::Hook => {
            reasons.push("event_only_not_natural_language".to_string());
            health_status = "not-probed".to_string();
        }
    }

    reasons.sort();
    reasons.dedup();
    let availability = if reasons.is_empty() {
        AvailabilityState::Ready
    } else if reasons.iter().all(|reason| {
        matches!(
            reason.as_str(),
            "auth_state_unknown"
                | "health_degraded"
                | "health_unknown"
                | "event_only_not_natural_language"
        )
    }) {
        AvailabilityState::Degraded {
            reason_codes: reasons.clone(),
        }
    } else {
        AvailabilityState::Unavailable {
            reason_codes: reasons.clone(),
        }
    };
    (availability, reasons, auth_state, health_status)
}

pub(super) struct NoProcessDiscovery;

impl CommandRunner for NoProcessDiscovery {
    fn run(&self, _program: &str, _args: &[&str]) -> CommandOutcome {
        CommandOutcome::Unavailable
    }
}

pub(super) fn routes_by_skill(routes: &[DemandRoute]) -> HashMap<&str, Vec<DemandRoute>> {
    let mut result: HashMap<&str, Vec<DemandRoute>> = HashMap::new();
    for route in routes {
        result
            .entry(route.skill_id.as_str())
            .or_default()
            .push(route.clone());
    }
    result
}

pub(super) fn auth_state_for(requires_auth: bool, observed: Option<AuthState>) -> AuthState {
    if !requires_auth {
        AuthState::NotRequired
    } else {
        observed.unwrap_or(AuthState::Unknown)
    }
}

pub(super) fn skill_card(
    manifest_root: &Path,
    capability: &ManagedCapability,
    registry: Option<&RegistrySkill>,
    overlay: Option<&UserOverlayEntry>,
    legacy_routes: &[DemandRoute],
    auth_state: AuthState,
) -> SkillCard {
    let file_metadata = load_skill_file_metadata(manifest_root, capability);
    let routing = registry.and_then(|item| item.routing.as_ref());
    let retired = routing.is_some_and(|routing| routing.route_state == RouteState::Retired);
    let ignored = overlay.is_some_and(|entry| entry.state == OverlayEntryState::Ignored)
        || capability.managed_status == ManagedStatus::Ignored;
    let routable = routing.is_some_and(|routing| routing.route_state == RouteState::Routable)
        || overlay.is_some_and(|entry| entry.state == OverlayEntryState::Active);
    let registered = capability.registry_status == RegistryStatus::Registered;
    let governance = if retired {
        GovernanceState::Retired
    } else if ignored {
        GovernanceState::Ignored
    } else if routable {
        GovernanceState::Active
    } else if registered {
        GovernanceState::ManagedInactive
    } else if capability.canonical_present {
        GovernanceState::Candidate
    } else {
        GovernanceState::Discovered
    };

    let mut reasons = Vec::new();
    if governance == GovernanceState::Candidate || governance == GovernanceState::Discovered {
        reasons.push("candidate_requires_adoption".to_string());
    }
    if governance == GovernanceState::ManagedInactive {
        reasons.push("registry_not_routable".to_string());
    }
    if retired {
        reasons.push("retired".to_string());
    }
    if !capability.canonical_present {
        reasons.push("canonical_missing".to_string());
    }
    if capability.health_status != HealthStatus::Healthy {
        reasons.push("health_degraded".to_string());
    }
    if !capability
        .host_visibility
        .iter()
        .any(|visibility| visibility.status == HostVisibilityStatus::Visible)
    {
        reasons.push("host_not_visible".to_string());
    }
    if matches!(auth_state, AuthState::Missing | AuthState::Unknown) {
        reasons.push("auth_required".to_string());
    }
    if overlay.is_some_and(|entry| entry.source_hash != source_hash(manifest_root, capability)) {
        reasons.push("source_hash_changed".to_string());
    }

    let declared_summary = registry
        .map(|item| item.description.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .or_else(|| overlay.map(|entry| entry.summary.trim().to_string()))
        .or_else(|| {
            let summary = if file_metadata.summary.trim().is_empty() {
                file_metadata.description.trim()
            } else {
                file_metadata.summary.trim()
            };
            (!summary.is_empty()).then(|| summary.to_string())
        })
        .filter(|summary| !summary.is_empty());
    let summary = declared_summary
        .clone()
        .unwrap_or_else(|| capability.name.clone());
    let mut intent_tags = routing
        .map(|routing| routing.intent_tags.clone())
        .or_else(|| overlay.map(|entry| entry.intent_tags.clone()))
        .unwrap_or_else(|| file_metadata.intent_tags.clone());
    if intent_tags.is_empty() && declared_summary.is_some() {
        intent_tags.push(capability.name.clone());
    }
    for route in legacy_routes {
        intent_tags.push(legacy_demand_tag(route.demand));
    }
    intent_tags.sort();
    intent_tags.dedup();
    let mut entrypoints = legacy_routes
        .iter()
        .filter_map(|route| route.entrypoint.clone())
        .collect::<Vec<_>>();
    if let Some(entry) = overlay {
        entrypoints.extend(entry.entrypoints.clone());
    } else {
        entrypoints.extend(file_metadata.entrypoints.clone());
    }
    entrypoints.sort();
    entrypoints.dedup();
    let invoke_hint_present = routing.is_some_and(|routing| !routing.invoke_hint.trim().is_empty())
        || overlay.is_some_and(|entry| !entry.invoke_hint.trim().is_empty())
        || !file_metadata.invoke_hint.trim().is_empty();
    let semantic_examples_complete = routing.is_none_or(|routing| {
        routing.route_state != RouteState::Routable
            || (!routing.examples.positive.is_empty() && !routing.examples.negative.is_empty())
    });
    if declared_summary.is_none()
        || intent_tags.is_empty()
        || (routable && (!invoke_hint_present || !semantic_examples_complete))
    {
        reasons.push("metadata_incomplete".to_string());
    }
    let positive_examples = routing
        .map(|routing| routing.examples.positive.clone())
        .unwrap_or_default();
    let negative_examples = routing
        .map(|routing| routing.examples.negative.clone())
        .unwrap_or_default();

    let availability = if governance == GovernanceState::Active && reasons.is_empty() {
        AvailabilityState::Ready
    } else if governance == GovernanceState::Active
        && reasons.iter().all(|reason| reason == "health_degraded")
    {
        AvailabilityState::Degraded {
            reason_codes: reasons.clone(),
        }
    } else {
        AvailabilityState::Unavailable {
            reason_codes: reasons.clone(),
        }
    };
    SkillCard {
        skill_id: capability.name.clone(),
        display_name: overlay
            .map(|entry| entry.display_name.trim().to_string())
            .filter(|display| !display.is_empty())
            .or_else(|| {
                let display = if file_metadata.display_name.trim().is_empty() {
                    file_metadata.name.trim()
                } else {
                    file_metadata.display_name.trim()
                };
                (!display.is_empty()).then(|| display.to_string())
            })
            .unwrap_or_else(|| capability.name.clone()),
        summary,
        intent_tags,
        positive_examples,
        negative_examples,
        entrypoints,
        source_kind: source_kind(capability),
        governance,
        availability,
        reason_codes: reasons,
        requires_auth: routing.is_some_and(|routing| routing.requires_auth)
            || overlay.is_some_and(|entry| entry.requires_auth)
            || file_metadata.requires_auth,
        auth_state,
        activity: ActivityState::Unobserved,
        version: overlay
            .map(|entry| entry.metadata_version.clone())
            .filter(|version| !version.is_empty())
            .or_else(|| {
                (!file_metadata.version.trim().is_empty())
                    .then(|| file_metadata.version.trim().to_string())
            })
            .unwrap_or_else(|| "registry".to_string()),
        source_hash: overlay
            .map(|entry| entry.source_hash.clone())
            .filter(|hash| !hash.is_empty())
            .unwrap_or_else(|| source_hash(manifest_root, capability)),
    }
}

pub(super) fn source_kind(capability: &ManagedCapability) -> SkillSourceKind {
    match capability.managed_status {
        ManagedStatus::SuiteManaged => SkillSourceKind::Suite,
        ManagedStatus::HostSystem => SkillSourceKind::HostSystem,
        ManagedStatus::ProjectLocal => SkillSourceKind::ProjectLocal,
        ManagedStatus::Discovered => {
            if capability
                .source
                .as_deref()
                .is_some_and(|source| source.contains("plugins/cache"))
            {
                SkillSourceKind::EnabledPlugin
            } else {
                SkillSourceKind::UserInstalled
            }
        }
        _ => SkillSourceKind::External,
    }
}

pub(super) fn source_hash(manifest_root: &Path, capability: &ManagedCapability) -> String {
    let Some(path) = capability_source_path(manifest_root, capability) else {
        return sha256(capability.name.as_bytes());
    };
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    let hashed = if path.is_dir() {
        append_source_directory(&path, &path, &mut canonical)
    } else {
        append_source_node(
            path.parent().unwrap_or_else(|| Path::new(".")),
            &path,
            &mut canonical,
        )
    };
    if hashed {
        sha256(&canonical)
    } else {
        sha256(format!("unreadable-skill-source\n{}", capability.name).as_bytes())
    }
}

pub fn hash_skill_source(path: &Path) -> Result<String, String> {
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    let hashed = if path.is_dir() {
        append_source_directory(path, path, &mut canonical)
    } else {
        append_source_node(
            path.parent().unwrap_or_else(|| Path::new(".")),
            path,
            &mut canonical,
        )
    };
    if hashed {
        Ok(sha256(&canonical))
    } else {
        Err(format!("cannot hash skill source {}", path.display()))
    }
}

pub(super) fn user_source_host_visible(
    host_home: &Path,
    active_host: &str,
    source: &UserSourceEntry,
) -> bool {
    let host_specific = match active_host {
        "codex" => Some(host_home.join(".codex/skills").join(&source.skill_id)),
        "claude-code" => Some(host_home.join(".claude/skills").join(&source.skill_id)),
        "omp" => Some(host_home.join(".omp/agent/skills").join(&source.skill_id)),
        "cursor" => Some(host_home.join(".cursor/skills").join(&source.skill_id)),
        "codebuddy-code" => Some(host_home.join(".codebuddy/skills").join(&source.skill_id)),
        _ => None,
    };
    let shared = matches!(active_host, "codex" | "omp" | "cursor")
        .then(|| host_home.join(".agents/skills").join(&source.skill_id));
    let existing = [host_specific, shared]
        .into_iter()
        .flatten()
        .filter(|entry| entry.exists() || std::fs::symlink_metadata(entry).is_ok())
        .collect::<Vec<_>>();
    existing.len() == 1
        && existing.iter().all(|entry| {
            std::fs::canonicalize(entry)
                .ok()
                .zip(std::fs::canonicalize(&source.canonical_path).ok())
                .is_some_and(|(actual, expected)| actual == expected)
                && entry.join("SKILL.md").is_file()
        })
}

/// Hash the complete skill body without timestamps or absolute paths. This
/// catches changes in referenced scripts/assets as well as `SKILL.md`. Symlinks
/// are represented by their link target and are never followed, avoiding
/// cycles or accidental traversal outside the skill body.
pub(super) fn append_source_directory(
    root: &Path,
    directory: &Path,
    canonical: &mut Vec<u8>,
) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .all(|path| append_source_node(root, path, canonical))
}

pub(super) fn append_source_node(root: &Path, path: &Path, canonical: &mut Vec<u8>) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if metadata.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(path) else {
            return false;
        };
        canonical.extend_from_slice(b"L\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(target.to_string_lossy().as_bytes());
        canonical.push(0);
        true
    } else if metadata.is_dir() {
        canonical.extend_from_slice(b"D\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        append_source_directory(root, path, canonical)
    } else if metadata.is_file() {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        canonical.extend_from_slice(b"F\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        canonical.extend_from_slice(&bytes);
        true
    } else {
        false
    }
}

pub(super) fn legacy_demand_tag(demand: SkillDemand) -> String {
    serde_json::to_value(demand)
        .ok()
        .and_then(|value| {
            Some(format!(
                "legacy:{}:{}",
                value.get("category")?.as_str()?,
                value.get("demand")?.as_str()?
            ))
        })
        .unwrap_or_else(|| "legacy:unknown".to_string())
}
