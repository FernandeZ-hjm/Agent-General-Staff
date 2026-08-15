//! Static third-party capability manifest loading and validation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifiedCatalogMarker {
    schema_version: String,
    release: String,
    content_hash: String,
    catalog_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThirdPartyCapability {
    pub id: String,
    #[serde(default)]
    pub compatibility_parent: Option<String>,
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
    #[serde(default)]
    pub bundled_path: Option<String>,
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
    match verified_catalog_resolution() {
        Ok(resolution) => Ok(resolution),
        Err(_) => bundled_resolution(root),
    }
}

fn bundled_resolution(root: &Path) -> Result<ManifestResolution, String> {
    let bundled_path = root.join(THIRD_PARTY_MANIFEST_PATH);
    let (content, source) = match read_catalog_regular_file(&bundled_path, MAX_MANIFEST_BYTES) {
        Ok(bytes) => (
            String::from_utf8(bytes)
                .map_err(|_| "bundled third-party catalog is not UTF-8".to_string())?,
            format!("bundled:{}", bundled_path.display()),
        ),
        Err(_) => (
            EMBEDDED_THIRD_PARTY_MANIFEST.to_string(),
            "bundled:embedded-third-party-capabilities.yaml".to_string(),
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
        release: Some(env!("CARGO_PKG_VERSION").to_string()),
    })
}

fn verified_catalog_resolution() -> Result<ManifestResolution, String> {
    let root = catalog_cache_root();
    verified_catalog_resolution_at(&root)
}

fn verified_catalog_resolution_at(root: &Path) -> Result<ManifestResolution, String> {
    reject_catalog_symlink(root, "catalog cache")?;
    let marker_path = root.join("current.json");
    let marker_bytes = read_catalog_regular_file(&marker_path, 64 * 1024)?;
    let marker: VerifiedCatalogMarker = serde_json::from_slice(&marker_bytes)
        .map_err(|error| format!("invalid verified catalog marker: {error}"))?;
    let expected_catalog_file = marker
        .content_hash
        .strip_prefix("sha256:")
        .map(|hash| format!("third-party-capabilities-{hash}.yaml"));
    if marker.schema_version != "ags://schema/contract/v2/verified-catalog"
        || marker.release.trim().is_empty()
        || !ags_platform::is_sha256(&marker.content_hash)
        || expected_catalog_file.as_deref() != Some(marker.catalog_file.as_str())
        || catalog_release_is_older(&marker.release, env!("CARGO_PKG_VERSION"))
    {
        return Err("verified catalog marker identity is invalid".to_string());
    }
    let catalog_path = root.join(&marker.catalog_file);
    let bytes = read_catalog_regular_file(&catalog_path, MAX_MANIFEST_BYTES)?;
    let observed = ags_platform::sha256(&bytes);
    if observed != marker.content_hash {
        return Err("verified catalog cache hash mismatch".to_string());
    }
    let content =
        std::str::from_utf8(&bytes).map_err(|_| "verified catalog is not UTF-8".to_string())?;
    let manifest = parse_manifest(content)
        .map_err(|error| format!("cannot parse verified catalog: {error}"))?;
    validate_manifest(&manifest)?;
    Ok(ManifestResolution {
        manifest,
        source: marker_path.display().to_string(),
        content_hash: observed,
        release: Some(marker.release),
    })
}

fn catalog_release_is_older(candidate: &str, current: &str) -> bool {
    fn parse(value: &str) -> Option<(u64, u64, u64)> {
        let value = value.split_once('-').map_or(value, |(base, _)| base);
        let mut parts = value.split('.').map(str::parse::<u64>);
        let parsed = (
            parts.next()?.ok()?,
            parts.next()?.ok()?,
            parts.next()?.ok()?,
        );
        parts.next().is_none().then_some(parsed)
    }
    match (parse(candidate), parse(current)) {
        (Some(candidate), Some(current)) => candidate < current,
        _ => true,
    }
}

fn catalog_cache_root() -> PathBuf {
    if let Some(cache) = std::env::var_os("AGS_CACHE_DIR") {
        return PathBuf::from(cache).join("launcher-state/catalog");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ags/launcher-state/catalog")
}

fn reject_catalog_symlink(path: &Path, label: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{label} is not a real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn read_catalog_regular_file(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect catalog file {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("catalog file is not regular: {}", path.display()));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(format!(
            "catalog file exceeds {max_bytes} bytes: {}",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn parse_manifest(content: &str) -> Result<ThirdPartyManifest, String> {
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("manifest_yaml_syntax: {error}"))?;
    validate_known_fields(&value, content)?;
    value
        .apply_merge()
        .map_err(|error| format!("manifest_yaml_merge: {error}"))?;
    serde_yaml::from_value(value).map_err(|error| format!("manifest_schema_invalid: {error}"))
}

fn validate_known_fields(value: &serde_yaml::Value, content: &str) -> Result<(), String> {
    const ROOT: &[&str] = &["schema_version", "principle", "capabilities"];
    const CAPABILITY: &[&str] = &[
        "id",
        "compatibility_parent",
        "kind",
        "name",
        "profiles",
        "required",
        "tier",
        "purpose",
        "risk",
        "requires_auth",
        "source",
        "install",
        "routing",
        "mcp",
        "hook",
    ];
    const SOURCE: &[&str] = &[
        "manager",
        "package",
        "version",
        "revision",
        "tracking_ref",
        "integrity",
        "repository",
        "license",
        "subdir",
        "bundled_path",
    ];
    const INSTALL: &[&str] = &["strategy", "command", "install_location", "depends_on"];
    const ROUTING: &[&str] = &[
        "route_state",
        "invoke_hint",
        "intent_tags",
        "scope_tags",
        "mutation_surface",
        "auth_kind",
        "cost_class",
        "route_priority",
        "capability_group",
        "upstream_group",
        "positive_examples",
        "negative_examples",
    ];
    const MCP: &[&str] = &["server_name", "command", "args"];
    const HOOK: &[&str] = &[
        "host",
        "events",
        "config_glob",
        "managed_by",
        "health_probe",
    ];

    check_mapping_fields(value, "$", ROOT, content)?;
    let Some(capabilities) = value
        .get("capabilities")
        .and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(());
    };
    for (index, capability) in capabilities.iter().enumerate() {
        let base = format!("capabilities[{index}]");
        check_mapping_fields(capability, &base, CAPABILITY, content)?;
        for (field, allowed) in [
            ("source", SOURCE),
            ("install", INSTALL),
            ("routing", ROUTING),
            ("mcp", MCP),
            ("hook", HOOK),
        ] {
            if let Some(nested) = capability.get(field) {
                if !nested.is_null() {
                    check_mapping_fields(nested, &format!("{base}.{field}"), allowed, content)?;
                }
            }
        }
    }
    Ok(())
}

fn check_mapping_fields(
    value: &serde_yaml::Value,
    yaml_path: &str,
    allowed: &[&str],
    content: &str,
) -> Result<(), String> {
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    for key in mapping.keys() {
        let Some(field) = key.as_str() else {
            continue;
        };
        if field == "<<" || allowed.contains(&field) {
            continue;
        }
        let (line, column) = yaml_key_location(content, field).unwrap_or((0, 0));
        return Err(format!(
            "manifest_unknown_field: yaml_path={yaml_path}.{field} field={field} allowed_fields=[{}] line={line} column={column}",
            allowed.join(",")
        ));
    }
    Ok(())
}

fn yaml_key_location(content: &str, field: &str) -> Option<(usize, usize)> {
    for (line_index, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed
            .strip_prefix(field)
            .is_some_and(|suffix| suffix.starts_with(':'))
        {
            return Some((line_index + 1, line.len() - trimmed.len() + 1));
        }
    }
    None
}

pub fn validate_manifest(manifest: &ThirdPartyManifest) -> Result<(), String> {
    if manifest.schema_version != "1.0" {
        return Err("third-party capability manifest schema_version must be 1.0".into());
    }
    let mut ids = BTreeSet::new();
    let mut compatibility_parents = BTreeSet::new();
    let mut mcp_server_names = BTreeSet::new();
    for capability in &manifest.capabilities {
        if capability.id.trim().is_empty() || !ids.insert(capability.id.as_str()) {
            return Err(format!(
                "third-party capability id is empty or duplicated: {}",
                capability.id
            ));
        }
        validate_source(capability)?;
        if let Some(parent) = capability.compatibility_parent.as_deref() {
            if !compatibility_parents.insert(parent) {
                return Err(format!("duplicate compatibility parent: {parent}"));
            }
        }
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
    if source.manager == "bundled"
        && (source
            .bundled_path
            .as_deref()
            .is_none_or(|value| validate_bundled_path(value).is_err())
            || source
                .integrity
                .as_deref()
                .is_none_or(|value| !ags_platform::is_sha256(value))
            || source.license.as_deref().is_none_or(str::is_empty))
    {
        return Err(format!(
            "{} bundled Skill source must pin a confined path, sha256 integrity, and license",
            capability.id
        ));
    }
    if let Some(parent) = capability.compatibility_parent.as_deref() {
        if capability.kind != CapabilityKind::Skill
            || source.manager != "bundled"
            || !stable_capability_id(parent)
        {
            return Err(format!(
                "{} compatibility_parent is valid only for a bundled Skill and must be a stable id",
                capability.id
            ));
        }
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

fn validate_bundled_path(value: &str) -> Result<(), ()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::CurDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(());
    }
    Ok(())
}

fn stable_capability_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
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
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct AdapterProvenance {
        distribution_id: String,
        compatibility_parent: String,
        upstream: AdapterUpstream,
        non_endorsement: String,
        excluded_upstream_paths: Vec<String>,
        ags_only_files: Vec<LocalProvenanceFile>,
        upstream_files: Vec<UpstreamProvenanceFile>,
    }

    #[derive(Deserialize)]
    struct AdapterUpstream {
        commit: String,
        license: String,
        copyright: String,
    }

    #[derive(Deserialize)]
    struct LocalProvenanceFile {
        local: String,
        sha256: String,
    }

    #[derive(Deserialize)]
    struct UpstreamProvenanceFile {
        local: String,
        upstream_sha256: String,
        local_sha256: String,
        status: String,
        #[serde(default)]
        modification_note: Option<String>,
    }

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
        let error = parse_manifest(content).unwrap_err();
        assert!(error.contains("manifest_unknown_field"));
        assert!(error.contains("yaml_path=capabilities[0].unexpected"));
        assert!(error.contains("field=unexpected"));
        assert!(error.contains("allowed_fields=[id,compatibility_parent,kind,name"));
        assert!(error.contains("line=7 column=5"));
    }

    #[test]
    fn verified_catalog_cache_accepts_exact_hash_and_rejects_drift() {
        let temp = tempfile::tempdir().unwrap();
        let catalog = b"schema_version: \"1.0\"\nprinciple: fixture\ncapabilities: []\n";
        let hash = ags_platform::sha256(catalog);
        let catalog_file = format!(
            "third-party-capabilities-{}.yaml",
            hash.trim_start_matches("sha256:")
        );
        std::fs::write(temp.path().join(&catalog_file), catalog).unwrap();
        std::fs::write(
            temp.path().join("current.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "ags://schema/contract/v2/verified-catalog",
                "release": "0.4.20",
                "content_hash": hash,
                "catalog_file": catalog_file.clone()
            }))
            .unwrap(),
        )
        .unwrap();
        let resolved = verified_catalog_resolution_at(temp.path()).unwrap();
        assert_eq!(resolved.release.as_deref(), Some("0.4.20"));
        assert_eq!(resolved.content_hash, ags_platform::sha256(catalog));

        std::fs::write(
            temp.path().join(&catalog_file),
            b"schema_version: \"1.0\"\ncapabilities: []\n",
        )
        .unwrap();
        assert!(verified_catalog_resolution_at(temp.path())
            .unwrap_err()
            .contains("hash mismatch"));
    }

    #[test]
    fn superpowers_adapter_provenance_license_and_catalog_identity_are_machine_checked() {
        let root = repo_root();
        let adapter = root.join("skill-packs/optional/ags-superpowers-adapter");
        let provenance: AdapterProvenance = serde_yaml::from_str(
            &std::fs::read_to_string(adapter.join("PROVENANCE.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(provenance.distribution_id, "ags-superpowers-adapter");
        assert_eq!(provenance.compatibility_parent, "superpowers");
        assert_eq!(
            provenance.upstream.commit,
            "44c9b2d6e889982ac18c27d05a19fefe335194e1"
        );
        assert_eq!(provenance.upstream.license, "MIT");
        assert_eq!(
            provenance.upstream.copyright,
            "Copyright (c) 2025 Jesse Vincent"
        );
        assert!(provenance.non_endorsement.contains("not official"));
        assert!(provenance
            .excluded_upstream_paths
            .iter()
            .any(|path| path.contains("visual-companion")));

        for file in &provenance.ags_only_files {
            let bytes = std::fs::read(adapter.join(&file.local)).unwrap();
            assert_eq!(
                ags_platform::sha256(&bytes),
                format!("sha256:{}", file.sha256)
            );
        }
        for file in &provenance.upstream_files {
            let bytes = std::fs::read(adapter.join(&file.local)).unwrap();
            assert_eq!(
                ags_platform::sha256(&bytes),
                format!("sha256:{}", file.local_sha256),
                "{}",
                file.local
            );
            assert!(ags_platform::is_sha256(&format!(
                "sha256:{}",
                file.upstream_sha256
            )));
            match file.status.as_str() {
                "unmodified" => {
                    assert_eq!(file.local_sha256, file.upstream_sha256);
                    assert!(file.modification_note.is_none());
                }
                "modified" => {
                    assert_ne!(file.local_sha256, file.upstream_sha256);
                    assert!(file
                        .modification_note
                        .as_deref()
                        .is_some_and(|note| !note.trim().is_empty()));
                }
                status => panic!("unsupported provenance status {status}"),
            }
        }

        let declared_files = provenance
            .ags_only_files
            .iter()
            .map(|file| file.local.clone())
            .chain(
                provenance
                    .upstream_files
                    .iter()
                    .map(|file| file.local.clone()),
            )
            .collect::<std::collections::BTreeSet<_>>();
        let actual_files = walk_adapter_paths(&adapter)
            .iter()
            .map(|path| {
                path.strip_prefix(&adapter)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .filter(|path| path != "PROVENANCE.yaml")
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            declared_files, actual_files,
            "every distributed adapter file except the self-describing provenance manifest must have one license/provenance declaration"
        );

        let license = std::fs::read_to_string(adapter.join("LICENSE")).unwrap();
        assert!(license.contains("Copyright (c) 2025 Jesse Vincent"));
        assert!(license.contains("Permission is hereby granted"));
        assert!(!walk_adapter_paths(&adapter)
            .iter()
            .any(|path| path.to_string_lossy().to_ascii_lowercase().contains("logo")));
        let all_text = walk_adapter_paths(&adapter)
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!all_text.contains("visual-companion.py"));
        assert!(!all_text.contains("telemetry endpoint"));

        let manifest = read_third_party_manifest(&root).unwrap();
        let catalog = manifest
            .capabilities
            .iter()
            .find(|capability| capability.id == "ags-superpowers-adapter")
            .unwrap();
        assert_eq!(catalog.compatibility_parent.as_deref(), Some("superpowers"));
        assert_eq!(catalog.source.manager, "bundled");
        let adapter_hash = crate::hash_skill_source(&adapter).unwrap();
        assert_eq!(
            catalog.source.integrity.as_deref(),
            Some(adapter_hash.as_str())
        );
    }

    fn walk_adapter_paths(root: &Path) -> Vec<PathBuf> {
        let mut pending = vec![root.to_path_buf()];
        let mut files = Vec::new();
        while let Some(path) = pending.pop() {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }
}
