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

// ── MCP registry reader ──────────────────────────────────────────────────────

pub(in super::super) struct RegistryEntry {
    pub(in super::super) name: String,
    pub(in super::super) manager: Option<String>,
    pub(in super::super) suite_interface: bool,
    /// Host clients the registry declares this server installed in
    /// (`install.installed_clients`). Used to decide expected host visibility.
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

/// Read `manifests/mcp-registry.yaml` and return entries from both the
/// `suite_interfaces:` (AGS self) and `mcps:` (governed) sections. Lenient:
/// returns an empty list when the file is missing or unparseable.
pub(in super::super) fn read_mcp_registry(repo_root: &Path) -> Vec<RegistryEntry> {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (section, is_iface) in [("suite_interfaces", true), ("mcps", false)] {
        if let Some(seq) = doc.get(section).and_then(|v| v.as_sequence()) {
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
                    out.push(RegistryEntry {
                        name: name.to_string(),
                        manager,
                        suite_interface: is_iface,
                        installed_clients,
                    });
                }
            }
        }
    }
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

/// Read stable routing metadata declared in `manifests/skills-registry.yaml`
/// (per skill) and `manifests/mcp-registry.yaml` (per MCP / suite interface),
/// keyed by capability name. This is the ONLY source of production routing
/// metadata — there is no built-in fallback table. Lenient: missing or
/// unparseable files yield an empty map, and entries without a `routing:` block
/// are simply absent (never synthesized). A present-but-malformed block is
/// absent from the map AND recorded in `parse_failures`.
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

    read
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
