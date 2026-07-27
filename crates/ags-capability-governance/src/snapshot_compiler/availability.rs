use super::*;
use super::{model::*, source::*};

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
    capability: &crate::third_party_manifest::ThirdPartyCapability,
    active_host: &str,
    catalog: &[SkillCard],
    inventory: &[ManagedCapability],
) -> (AvailabilityState, Vec<String>, AuthState, String) {
    use crate::third_party_manifest::CapabilityKind;

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

pub(crate) struct NoProcessDiscovery;

impl CommandRunner for NoProcessDiscovery {
    fn run(&self, _program: &str, _args: &[&str]) -> CommandOutcome {
        CommandOutcome::Unavailable
    }
}

pub(super) fn auth_state_for(requires_auth: bool, observed: Option<AuthState>) -> AuthState {
    if !requires_auth {
        AuthState::NotRequired
    } else {
        observed.unwrap_or(AuthState::Unknown)
    }
}

pub(crate) fn skill_card(
    manifest_root: &Path,
    capability: &ManagedCapability,
    registry: Option<&RegistrySkill>,
    auth_state: AuthState,
) -> SkillCard {
    let file_metadata = load_skill_file_metadata(manifest_root, capability);
    let routing = registry.and_then(|item| item.routing.as_ref());
    let retired = routing.is_some_and(|routing| routing.route_state == RouteState::Retired);
    let routing_surface = routing
        .and_then(|routing| routing.routing_surface)
        .unwrap_or_else(|| {
            if routing.is_some_and(|routing| routing.route_state == RouteState::Routable) {
                SkillRoutingSurface::SkillTarget
            } else {
                SkillRoutingSurface::NotRoutable
            }
        });
    let routable = routing_surface == SkillRoutingSurface::SkillTarget
        && routing.is_some_and(|routing| routing.route_state == RouteState::Routable);
    let registered = capability.registry_status == RegistryStatus::Registered;
    let governance = if retired {
        GovernanceState::Retired
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
    if governance == GovernanceState::ManagedInactive
        && routing_surface == SkillRoutingSurface::NotRoutable
    {
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
    let declared_summary = registry
        .map(|item| item.description.trim().to_string())
        .filter(|summary| !summary.is_empty())
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
        .unwrap_or_else(|| file_metadata.intent_tags.clone());
    if intent_tags.is_empty() && declared_summary.is_some() {
        intent_tags.push(capability.name.clone());
    }
    intent_tags.sort();
    intent_tags.dedup();
    let mut entrypoints = file_metadata.entrypoints.clone();
    entrypoints.sort();
    entrypoints.dedup();
    let invoke_hint_present = routing.is_some_and(|routing| !routing.invoke_hint.trim().is_empty())
        || !file_metadata.invoke_hint.trim().is_empty();
    let semantic_examples_complete = routing.is_none_or(|routing| {
        routing.route_state != RouteState::Routable
            || (!routing.examples.positive.is_empty() && !routing.examples.negative.is_empty())
    });
    let host_command_hint_missing = routing_surface == SkillRoutingSurface::HostCommand
        && routing.is_none_or(|routing| routing.invoke_hint.trim().is_empty());
    if declared_summary.is_none()
        || intent_tags.is_empty()
        || (routable && (!invoke_hint_present || !semantic_examples_complete))
        || host_command_hint_missing
    {
        reasons.push("metadata_incomplete".to_string());
    }
    let positive_examples = routing
        .map(|routing| routing.examples.positive.clone())
        .unwrap_or_default();
    let negative_examples = routing
        .map(|routing| routing.examples.negative.clone())
        .unwrap_or_default();

    let availability = if (governance == GovernanceState::Active
        || routing_surface == SkillRoutingSurface::HostCommand)
        && reasons.is_empty()
    {
        AvailabilityState::Ready
    } else if (governance == GovernanceState::Active
        || routing_surface == SkillRoutingSurface::HostCommand)
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
        display_name: {
            let display = if file_metadata.display_name.trim().is_empty() {
                file_metadata.name.trim()
            } else {
                file_metadata.display_name.trim()
            };
            (!display.is_empty()).then(|| display.to_string())
        }
        .unwrap_or_else(|| capability.name.clone()),
        summary,
        intent_tags,
        positive_examples,
        negative_examples,
        entrypoints,
        routing_surface,
        routing_hint: (routing_surface == SkillRoutingSurface::HostCommand)
            .then(|| routing.map(|routing| routing.invoke_hint.trim().to_string()))
            .flatten()
            .filter(|hint| !hint.is_empty()),
        source_kind: source_kind(capability),
        governance,
        availability,
        reason_codes: reasons,
        requires_auth: routing.is_some_and(|routing| routing.requires_auth)
            || file_metadata.requires_auth,
        auth_state,
        version: if !file_metadata.version.trim().is_empty() {
            file_metadata.version.trim().to_string()
        } else {
            "registry".to_string()
        },
        source_hash: source_hash(manifest_root, capability),
    }
}
