//! Third-party capability manifest retrieval and integrity verification.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

pub const THIRD_PARTY_MANIFEST_PATH: &str = "manifests/third-party-capabilities.yaml";
pub const DEFAULT_THIRD_PARTY_MANIFEST_REVISION: &str = "821fb728b58c131c70a82dad51ccf83eb0372413";
pub const DEFAULT_THIRD_PARTY_MANIFEST_URL: &str = "https://raw.githubusercontent.com/FernandeZ-hjm/Agent-General-Staff/821fb728b58c131c70a82dad51ccf83eb0372413/manifests/third-party-capabilities.yaml";
pub const DEFAULT_THIRD_PARTY_MANIFEST_HASH: &str =
    "sha256:77af54eab76a8d031d8ec6ffdd79224c1b0f5b829e392402ac349557940a9324";
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
    pub freshness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
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
    #[serde(default)]
    pub rollback: String,
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
    if !cfg!(test) && std::env::var("AGS_THIRD_PARTY_MANIFEST_OFFLINE").as_deref() != Ok("1") {
        let url = std::env::var("AGS_THIRD_PARTY_MANIFEST_URL")
            .unwrap_or_else(|_| DEFAULT_THIRD_PARTY_MANIFEST_URL.to_string());
        if is_allowed_registry_url(&url) {
            match fetch_remote_manifest(&url) {
                Ok((manifest, content_hash)) => {
                    return Ok(ManifestResolution {
                        manifest,
                        source: url,
                        content_hash,
                        freshness: "github-pinned".into(),
                        fallback_reason: None,
                    });
                }
                Err(error) => return local_resolution(root, Some(error)),
            }
        }
        return local_resolution(
            root,
            Some("remote registry URL is not an allowed raw GitHub HTTPS URL".into()),
        );
    }
    local_resolution(root, Some("remote refresh disabled".into()))
}

fn local_resolution(
    root: &Path,
    fallback_reason: Option<String>,
) -> Result<ManifestResolution, String> {
    let path = root.join(THIRD_PARTY_MANIFEST_PATH);
    let (content, source, freshness) = match std::fs::read_to_string(&path) {
        Ok(content) => (
            content,
            path.display().to_string(),
            "workspace-snapshot".to_string(),
        ),
        Err(_) => (
            EMBEDDED_THIRD_PARTY_MANIFEST.to_string(),
            "embedded:third-party-capabilities.yaml".to_string(),
            "embedded-fallback".to_string(),
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
        content_hash: sha256(content.as_bytes()),
        freshness,
        fallback_reason,
    })
}

fn fetch_remote_manifest(url: &str) -> Result<(ThirdPartyManifest, String), String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "8",
            "--proto",
            "=https",
            "--tlsv1.2",
            url,
        ])
        .output()
        .map_err(|error| format!("GitHub registry fetch could not start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "GitHub registry fetch failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if output.stdout.len() > MAX_MANIFEST_BYTES {
        return Err("GitHub registry response exceeds 1 MiB".into());
    }
    let content = String::from_utf8(output.stdout)
        .map_err(|_| "GitHub registry response is not UTF-8".to_string())?;
    let content_hash = sha256(content.as_bytes());
    if url == DEFAULT_THIRD_PARTY_MANIFEST_URL && content_hash != DEFAULT_THIRD_PARTY_MANIFEST_HASH
    {
        return Err(format!(
            "pinned GitHub registry hash mismatch: expected {DEFAULT_THIRD_PARTY_MANIFEST_HASH}, got {content_hash}"
        ));
    }
    let manifest = parse_manifest(&content)
        .map_err(|error| format!("cannot parse GitHub registry: {error}"))?;
    validate_manifest(&manifest)?;
    Ok((manifest, content_hash))
}

fn is_allowed_registry_url(url: &str) -> bool {
    url.starts_with("https://raw.githubusercontent.com/")
        && url.ends_with("/manifests/third-party-capabilities.yaml")
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
            .is_none_or(|value| !is_git_revision(value))
    {
        return Err(format!("{} git source must pin a commit", capability.id));
    }
    Ok(())
}

fn validate_install(capability: &ThirdPartyCapability) -> Result<(), String> {
    const STRATEGIES: &[&str] = &[
        "ags-skill-adopt",
        "npm-global",
        "host-registrar",
        "external-manager",
        "none",
    ];
    if !STRATEGIES.contains(&capability.install.strategy.as_str()) {
        return Err(format!(
            "{} has unsupported install strategy {}; arbitrary shell is forbidden",
            capability.id, capability.install.strategy
        ));
    }
    Ok(())
}

fn validate_routing(capability: &ThirdPartyCapability) -> Result<(), String> {
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
        || hook.rollback.is_empty()
    {
        return Err(format!(
            "{} hook must declare host, events, config, owner, health probe, and rollback",
            capability.id
        ));
    }
    Ok(())
}

fn is_git_revision(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
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
      rollback: plugin-manager
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

    #[test]
    fn default_registry_uses_an_immutable_reviewed_commit() {
        assert_eq!(DEFAULT_THIRD_PARTY_MANIFEST_REVISION.len(), 40);
        assert!(DEFAULT_THIRD_PARTY_MANIFEST_REVISION
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert!(DEFAULT_THIRD_PARTY_MANIFEST_URL
            .contains(&format!("/{DEFAULT_THIRD_PARTY_MANIFEST_REVISION}/")));
        assert!(!DEFAULT_THIRD_PARTY_MANIFEST_URL.contains("/main/"));
        assert_eq!(DEFAULT_THIRD_PARTY_MANIFEST_HASH.len(), 71);
        assert!(DEFAULT_THIRD_PARTY_MANIFEST_HASH
            .strip_prefix("sha256:")
            .is_some_and(|hash| hash.chars().all(|character| character.is_ascii_hexdigit())));
    }
}
