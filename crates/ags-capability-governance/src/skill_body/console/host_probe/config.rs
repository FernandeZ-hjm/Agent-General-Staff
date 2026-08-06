use super::*;

/// A family of skills fronted by an external CLI. The console recognises these
/// so it can distinguish the CLI binary, the `*-cli` family skills, and the
/// external endpoint they ultimately talk to.
pub(in super::super) struct CliFamily {
    pub(in super::super) prefix: &'static str,
    pub(in super::super) cli: &'static str,
    pub(in super::super) endpoint: &'static str,
}

pub(in super::super) const CLI_FAMILIES: &[CliFamily] = &[CliFamily {
    prefix: "lark-",
    cli: "lark-cli",
    endpoint: "Feishu / Lark Open Platform",
}];

/// Match a *skill* name to a CLI family. The synthetic CLI capability itself
/// (e.g. `lark-cli`) is excluded so it is not double-classified as a family
/// member.
pub(in super::super) fn cli_family_for_skill(skill_name: &str) -> Option<&'static CliFamily> {
    CLI_FAMILIES
        .iter()
        .find(|f| skill_name != f.cli && skill_name.starts_with(f.prefix))
}

// ── MCP inventory sources ───────────────────────────────────────────────────

pub(in super::super) struct McpInventorySource {
    pub(in super::super) name: String,
    pub(in super::super) manager: Option<String>,
    pub(in super::super) suite_interface: bool,
    pub(in super::super) declaration_source: &'static str,
    /// Only the bundled AGS suite interface may declare an expected Host set.
    /// Third-party installation and activation are observed from Host state.
    pub(in super::super) installed_clients: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct RegistrySkillBody {
    pub(in super::super) name: String,
    pub(in super::super) profile: Option<String>,
    pub(in super::super) manager: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct RequiredRegistrySkill {
    pub(in super::super) name: String,
    pub(in super::super) profile: String,
    pub(in super::super) local_path: Option<String>,
    pub(in super::super) source_type: Option<String>,
}

/// Join the bundled AGS suite interface with catalog MCP identities. Static
/// manifests never claim that a third-party MCP is installed or active; live
/// Host probes supply that fact later in the inventory build.
pub(in super::super) fn read_mcp_inventory_sources(repo_root: &Path) -> Vec<McpInventorySource> {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    let mut out = Vec::new();
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
            if let Some(seq) = doc.get("suite_interfaces").and_then(|v| v.as_sequence()) {
                for item in seq {
                    if let Some(name) = item.get("name").and_then(|v| v.as_str()) {
                        let manager = item
                            .get("package")
                            .and_then(|p| p.get("manager"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        let installed_clients = item
                            .get("install")
                            .and_then(|i| i.get("installed_clients"))
                            .and_then(|v| v.as_sequence())
                            .map(|seq| {
                                seq.iter()
                                    .filter_map(|v| v.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        out.push(McpInventorySource {
                            name: name.to_string(),
                            manager,
                            suite_interface: true,
                            declaration_source: "manifests/mcp-registry.yaml",
                            installed_clients,
                        });
                    }
                }
            }
        }
    }

    if let Ok(manifest) = crate::third_party_manifest::read_third_party_manifest(repo_root) {
        for capability in manifest.capabilities.into_iter().filter(|capability| {
            capability.kind == crate::third_party_manifest::CapabilityKind::Mcp
        }) {
            let Some(contract) = capability.mcp else {
                continue;
            };
            out.push(McpInventorySource {
                name: contract.server_name,
                manager: Some(capability.source.manager),
                suite_interface: false,
                declaration_source: crate::third_party_manifest::THIRD_PARTY_MANIFEST_PATH,
                installed_clients: Vec::new(),
            });
        }
    }
    out.sort_by(|left, right| left.name.cmp(&right.name));
    out.dedup_by(|left, right| left.name == right.name);
    out
}

// ── Skill-resolution metadata reader (manifest = single authority) ───────────

/// Result of reading routing metadata from the manifests: the parsed per-member
/// `map` plus the `parse_failures` — names of members whose `routing:` block was
/// present but failed to parse. Failures are SURFACED (not silently swallowed),
/// so doctor / inventory can flag routing schema drift while routing itself
/// stays fail-closed (a failed member is absent from `map` → never routed).
#[derive(Debug, Clone, Default)]
pub struct RoutingRead {
    pub map: HashMap<String, RoutingMetadata>,
    pub(in super::super) external_skill_bodies: HashMap<String, RegistrySkillBody>,
    pub(in super::super) required_skill_parents: Vec<RequiredRegistrySkill>,
    /// Internal-entrypoint route targets declared under a `route_targets:`
    /// section — (name, routing) pairs synthesized into route-target inventory
    /// rows. Each routing carries a `parent`; these are NEVER standalone bodies.
    pub route_targets: Vec<(String, RoutingMetadata)>,
    pub parse_failures: Vec<String>,
}

/// Read stable routing metadata from the suite Skill registry, the canonical
/// third-party catalog, and internal entrypoint declarations. There is no
/// built-in fallback table. Missing or malformed entries stay absent and are
/// recorded in `parse_failures` so routing fails closed.
pub(in super::super) fn read_routing_metadata(repo_root: &Path) -> RoutingRead {
    let mut read = RoutingRead::default();

    let manifests = [
        (
            repo_root.join("manifests/skills-registry.yaml"),
            &["skills"][..],
        ),
        (
            repo_root.join("manifests/mcp-registry.yaml"),
            &["suite_interfaces", "mcps"][..],
        ),
    ];
    for (path, member_sections) in manifests {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
            continue;
        };
        for section in member_sections {
            if let Some(seq) = doc.get(*section).and_then(|v| v.as_sequence()) {
                for item in seq {
                    collect_routing(item, &mut read, *section == "skills");
                }
            }
        }
        if let Some(seq) = doc.get("route_targets").and_then(|v| v.as_sequence()) {
            for item in seq {
                collect_route_target(item, &mut read);
            }
        }
    }

    match crate::third_party_manifest::read_third_party_manifest(repo_root) {
        Ok(manifest) => {
            for capability in manifest.capabilities.iter().filter(|capability| {
                matches!(
                    capability.kind,
                    crate::third_party_manifest::CapabilityKind::Skill
                        | crate::third_party_manifest::CapabilityKind::Mcp
                        | crate::third_party_manifest::CapabilityKind::Cli
                )
            }) {
                let name = capability
                    .mcp
                    .as_ref()
                    .map(|contract| contract.server_name.as_str())
                    .unwrap_or(capability.id.as_str());
                match catalog_routing(capability) {
                    Ok(routing) => {
                        if read.map.contains_key(name) {
                            read.parse_failures
                                .push(format!("duplicate-route-authority:{name}"));
                        } else {
                            read.map.insert(name.to_string(), routing);
                        }
                    }
                    Err(()) => read.parse_failures.push(name.to_string()),
                }
            }
        }
        Err(error) => read
            .parse_failures
            .push(format!("third-party-capabilities: {error}")),
    }

    read
}

fn catalog_routing(
    capability: &crate::third_party_manifest::ThirdPartyCapability,
) -> Result<RoutingMetadata, ()> {
    let routing = &capability.routing;
    let route_state = match routing.route_state.as_str() {
        "routable" => RouteState::Routable,
        "not-routable" => RouteState::NotRoutable,
        "retired" => RouteState::Retired,
        _ => return Err(()),
    };
    let mutation_surface = match routing.mutation_surface.as_str() {
        "" | "read-only" => MutationSurface::ReadOnly,
        "local-write" => MutationSurface::LocalWrite,
        "external-write" => MutationSurface::ExternalWrite,
        _ => return Err(()),
    };
    let cost_class = match routing.cost_class.as_str() {
        "" | "free" => CostClass::Free,
        "local" => CostClass::Local,
        "network" => CostClass::Network,
        "paid" => CostClass::Paid,
        _ => return Err(()),
    };
    Ok(RoutingMetadata {
        intent_tags: routing.intent_tags.clone(),
        scope_tags: routing.scope_tags.clone(),
        mutation_surface,
        requires_auth: capability.requires_auth,
        auth_kind: routing.auth_kind.clone(),
        cost_class,
        invoke_hint: routing.invoke_hint.clone().unwrap_or_default(),
        route_priority: routing
            .route_priority
            .unwrap_or_else(default_route_priority),
        route_state,
        capability_group: routing.capability_group.clone(),
        upstream_group: routing.upstream_group.clone(),
        examples: RouteExamples {
            positive: routing.positive_examples.clone(),
            negative: routing.negative_examples.clone(),
        },
        parent: None,
        entrypoint: None,
    })
}

/// Parse one registry entry's `name` + optional `routing:` block. An entry
/// without `routing:` is skipped (no synthesis). A malformed `routing:` block is
/// kept OUT of the map (fail-closed: never routed) but its name is recorded in
/// `parse_failures` rather than silently swallowed, so schema drift is visible.
pub(in super::super) fn collect_routing(
    item: &serde_yaml::Value,
    read: &mut RoutingRead,
    may_require_skill_body: bool,
) {
    let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    if let Some(source) = item.get("source") {
        let external = matches!(
            source.get("type").and_then(|v| v.as_str()),
            Some("external_cli_skill" | "external_shared_skill")
        );
        let manager = source
            .get("manager")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|manager| !manager.is_empty());
        if external && is_safe_path_component(name) {
            if let Some(manager) = manager {
                read.external_skill_bodies.insert(
                    name.to_string(),
                    RegistrySkillBody {
                        name: name.to_string(),
                        profile: item
                            .get("profile")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string),
                        manager: manager.to_string(),
                    },
                );
            }
        }
    }
    let Some(routing_val) = item.get("routing") else {
        return;
    };
    match serde_yaml::from_value::<RoutingMetadata>(routing_val.clone()) {
        Ok(meta) => {
            if may_require_skill_body
                && item.get("profile").and_then(|v| v.as_str()) == Some("required")
                && meta.route_state == RouteState::Routable
                && meta.parent.is_none()
                && is_safe_path_component(name)
            {
                read.required_skill_parents.push(RequiredRegistrySkill {
                    name: name.to_string(),
                    profile: "required".to_string(),
                    local_path: item
                        .get("local_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    source_type: item
                        .get("source")
                        .and_then(|source| source.get("type"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                });
            }
            read.map.insert(name.to_string(), meta);
        }
        Err(_) => read.parse_failures.push(name.to_string()),
    }
}

/// Parse one `route_targets:` entry into a (name, routing) pair. The routing
/// block MUST carry a `parent` (that is what makes it a route target); a missing
/// `routing:` block, a malformed one, or one without `parent` is recorded in
/// `parse_failures` (fail-closed: never routed, never synthesized).
pub(in super::super) fn collect_route_target(item: &serde_yaml::Value, read: &mut RoutingRead) {
    let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(routing_val) = item.get("routing") else {
        read.parse_failures.push(name.to_string());
        return;
    };
    match serde_yaml::from_value::<RoutingMetadata>(routing_val.clone()) {
        Ok(meta) if meta.parent.is_some() => {
            read.route_targets.push((name.to_string(), meta));
        }
        _ => read.parse_failures.push(name.to_string()),
    }
}
