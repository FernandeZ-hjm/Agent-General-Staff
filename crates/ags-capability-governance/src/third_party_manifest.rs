//! Static third-party capability manifest loading and validation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const THIRD_PARTY_MANIFEST_PATH: &str = "manifests/third-party-capabilities.yaml";
const EMBEDDED_THIRD_PARTY_MANIFEST: &str =
    include_str!("../../../manifests/third-party-capabilities.yaml");
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum CapabilityKind {
    Skill,
    Cli,
    Mcp,
    Hook,
}

impl CapabilityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
            Self::Hook => "hook",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyManifest {
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub principle: String,
    #[serde(default)]
    pub capabilities: Vec<ThirdPartyCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestResolution {
    pub manifest: ThirdPartyManifest,
    pub source: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyCapability {
    pub id: String,
    pub kind: CapabilityKind,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub risk: String,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub source: CapabilitySource,
    #[serde(default)]
    pub install: InstallContract,
    #[serde(default)]
    pub routing: RoutingContract,
    #[serde(default)]
    pub mcp: Option<McpContract>,
    #[serde(default)]
    pub hook: Option<HookContract>,
}

impl ThirdPartyCapability {
    pub fn applies_to(&self, profile: &str) -> bool {
        self.profiles.iter().any(|candidate| candidate == profile)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySource {
    #[serde(default)]
    pub manager: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub tracking_ref: Option<String>,
    #[serde(default)]
    pub integrity: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub subdir: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallContract {
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingContract {
    #[serde(default)]
    pub route_state: String,
    #[serde(default)]
    pub invoke_hint: Option<String>,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub scope_tags: Vec<String>,
    #[serde(default)]
    pub mutation_surface: String,
    #[serde(default)]
    pub auth_kind: Option<String>,
    #[serde(default)]
    pub cost_class: String,
    #[serde(default)]
    pub route_priority: Option<i32>,
    #[serde(default)]
    pub capability_group: Vec<String>,
    #[serde(default)]
    pub upstream_group: Option<String>,
    #[serde(default)]
    pub positive_examples: Vec<String>,
    #[serde(default)]
    pub negative_examples: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpContract {
    #[serde(default)]
    pub server_name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookContract {
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub config_glob: String,
    #[serde(default)]
    pub managed_by: String,
    #[serde(default)]
    pub health_probe: String,
}

pub fn read_third_party_manifest(root: &Path) -> Result<ThirdPartyManifest, String> {
    let path = root.join(THIRD_PARTY_MANIFEST_PATH);
    let content =
        std::fs::read_to_string(&path).unwrap_or_else(|_| EMBEDDED_THIRD_PARTY_MANIFEST.into());
    let manifest = parse_manifest(&content)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn resolve_third_party_manifest(root: &Path) -> Result<ManifestResolution, String> {
    local_resolution(root)
}

fn local_resolution(root: &Path) -> Result<ManifestResolution, String> {
    let path = root.join(THIRD_PARTY_MANIFEST_PATH);
    let (content, source) = match std::fs::read_to_string(&path) {
        Ok(content) => (content, path.display().to_string()),
        Err(_) => (
            EMBEDDED_THIRD_PARTY_MANIFEST.to_string(),
            "embedded:third-party-capabilities.yaml".to_string(),
        ),
    };
    if content.len() > MAX_MANIFEST_BYTES {
        return Err("third-party capability manifest exceeds 1 MiB".into());
    }
    let manifest =
        parse_manifest(&content).map_err(|error| format!("cannot parse {source}: {error}"))?;
    validate_manifest(&manifest)?;
    Ok(ManifestResolution {
        manifest,
        source,
        content_hash: ags_platform::sha256(content.as_bytes()),
    })
}

fn parse_manifest(content: &str) -> Result<ThirdPartyManifest, serde_yaml::Error> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(content)?;
    value.apply_merge()?;
    serde_yaml::from_value(value)
}

pub fn validate_manifest(manifest: &ThirdPartyManifest) -> Result<(), String> {
    if manifest.schema_version != "1.0" {
        return Err("third-party capability manifest schema_version must be 1.0".into());
    }
    let mut ids = BTreeSet::new();
    let mut mcp_server_names = BTreeSet::new();
    for capability in &manifest.capabilities {
        if capability.id.trim().is_empty() || !ids.insert(capability.id.as_str()) {
            return Err(format!(
                "third-party capability id is empty or duplicated: {}",
                capability.id
            ));
        }
        validate_source(capability)?;
        validate_install(capability)?;
        validate_routing(capability)?;
        validate_hook(capability)?;
        if capability.kind == CapabilityKind::Mcp {
            let server_name = capability
                .mcp
                .as_ref()
                .map(|mcp| mcp.server_name.trim())
                .filter(|name| !name.is_empty())
                .ok_or_else(|| format!("{} MCP contract is missing", capability.id))?;
            if !mcp_server_names.insert(server_name) {
                return Err(format!("duplicate MCP server name: {server_name}"));
            }
        }
    }
    Ok(())
}

fn validate_source(capability: &ThirdPartyCapability) -> Result<(), String> {
    let source = &capability.source;
    if matches!(source.manager.as_str(), "npm" | "git")
        && (source
            .repository
            .as_deref()
            .is_none_or(|value| !value.starts_with("https://github.com/"))
            || source.license.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{} must pin a GitHub source and license",
            capability.id
        ));
    }
    if source.manager == "npm"
        && (source.package.as_deref().is_none_or(str::is_empty)
            || source.version.as_deref().is_none_or(str::is_empty)
            || source.integrity.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{} npm source must pin package, version, and integrity",
            capability.id
        ));
    }
    if source.manager == "git"
        && source
            .revision
            .as_deref()
            .is_none_or(|value| !ags_platform::is_git_commit(value))
    {
        return Err(format!("{} git source must pin a commit", capability.id));
    }
    if capability.kind == CapabilityKind::Skill
        && source.manager == "git"
        && (source
            .integrity
            .as_deref()
            .is_none_or(|value| !ags_platform::is_sha256(value))
            || source.tracking_ref.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{} reviewed Skill source must pin sha256 integrity and a tracking ref",
            capability.id
        ));
    }
    Ok(())
}

fn validate_install(capability: &ThirdPartyCapability) -> Result<(), String> {
    const STRATEGIES: &[&str] = &["npm-global", "host-registrar", "external-manager", "none"];
    if !STRATEGIES.contains(&capability.install.strategy.as_str()) {
        return Err(format!(
            "{} has unsupported install strategy {}; arbitrary shell is forbidden",
            capability.id, capability.install.strategy
        ));
    }
    Ok(())
}

fn validate_routing(capability: &ThirdPartyCapability) -> Result<(), String> {
    if !matches!(
        capability.routing.route_state.as_str(),
        "routable" | "not-routable" | "retired"
    ) {
        return Err(format!(
            "{} has unsupported routing state {}",
            capability.id, capability.routing.route_state
        ));
    }
    if !matches!(
        capability.routing.mutation_surface.as_str(),
        "" | "read-only" | "local-write" | "external-write"
    ) {
        return Err(format!(
            "{} has unsupported mutation surface {}",
            capability.id, capability.routing.mutation_surface
        ));
    }
    if !matches!(
        capability.routing.cost_class.as_str(),
        "" | "free" | "local" | "network" | "paid"
    ) {
        return Err(format!(
            "{} has unsupported cost class {}",
            capability.id, capability.routing.cost_class
        ));
    }
    if capability.kind == CapabilityKind::Hook {
        if capability.routing.route_state == "routable" {
            return Err(format!("{} hook must not be routable", capability.id));
        }
        return Ok(());
    }
    if capability.routing.route_state == "routable"
        && (capability
            .routing
            .invoke_hint
            .as_deref()
            .is_none_or(str::is_empty)
            || capability.routing.intent_tags.is_empty()
            || capability.routing.positive_examples.is_empty()
            || capability.routing.negative_examples.is_empty())
    {
        return Err(format!(
            "{} routable capability must declare invoke_hint, intent_tags, and positive/negative examples",
            capability.id
        ));
    }
    Ok(())
}

fn validate_hook(capability: &ThirdPartyCapability) -> Result<(), String> {
    if capability.kind != CapabilityKind::Hook {
        return Ok(());
    }
    let hook = capability
        .hook
        .as_ref()
        .ok_or_else(|| format!("{} hook contract is missing", capability.id))?;
    if hook.host.is_empty()
        || hook.events.is_empty()
        || hook.config_glob.is_empty()
        || hook.managed_by.is_empty()
        || hook.health_probe.is_empty()
    {
        return Err(format!(
            "{} hook must declare host, events, config, owner, and health probe",
            capability.id
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn unified_manifest_covers_all_four_capability_kinds() {
        let manifest = read_third_party_manifest(&repo_root()).unwrap();
        for kind in [
            CapabilityKind::Skill,
            CapabilityKind::Cli,
            CapabilityKind::Mcp,
            CapabilityKind::Hook,
        ] {
            assert!(
                manifest
                    .capabilities
                    .iter()
                    .any(|capability| capability.kind == kind),
                "missing {kind:?}"
            );
        }
    }

    #[test]
    fn hook_cannot_claim_natural_language_routing() {
        let yaml = r#"
schema_version: "1.0"
capabilities:
  - id: bad-hook
    kind: hook
    profiles: [private]
    install: { strategy: external-manager }
    routing: { route_state: routable }
    hook:
      host: claude-code
      events: [Stop]
      config_glob: hooks.json
      managed_by: plugin
      health_probe: plugin-config
"#;
        let manifest: ThirdPartyManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn manifest_rejects_unknown_capability_fields() {
        let content = r#"
schema_version: "1.0"
capabilities:
  - id: example
    kind: cli
    profiles: [public]
    unexpected: true
"#;
        assert!(parse_manifest(content).is_err());
    }
}
