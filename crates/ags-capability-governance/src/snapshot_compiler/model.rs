use super::*;
#[allow(unused_imports)]
use crate::{
    authority::*, catalog::*, hashing::*, overlay_transaction::*, private_store::*, usage_ledger::*,
};
#[derive(Debug, Deserialize)]
pub(crate) struct RegistryDocument {
    #[serde(default)]
    pub(crate) skills: Vec<RegistrySkill>,
    #[serde(default)]
    pub(super) demand_routes: Vec<DemandRoute>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistrySkill {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(super) routing: Option<RegistryRouting>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RegistryRouting {
    #[serde(default)]
    pub(crate) intent_tags: Vec<String>,
    #[serde(default)]
    pub(super) requires_auth: bool,
    #[serde(default)]
    pub(super) invoke_hint: String,
    #[serde(default)]
    pub(super) route_state: RouteState,
    #[serde(default)]
    pub(super) routing_surface: Option<SkillRoutingSurface>,
    #[serde(default)]
    pub(super) examples: RouteExamples,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SkillFileMetadata {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(super) display_name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(super) summary: String,
    #[serde(default)]
    pub(crate) intent_tags: Vec<String>,
    #[serde(default)]
    pub(super) entrypoints: Vec<String>,
    #[serde(default)]
    pub(super) invoke_hint: String,
    #[serde(default)]
    pub(super) requires_auth: bool,
    #[serde(default)]
    pub(super) version: String,
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

pub(crate) fn load_skill_file_metadata(
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

pub(crate) fn load_skill_metadata_path(skill_md: &Path) -> SkillFileMetadata {
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

pub(crate) fn load_registry_document(root: &Path) -> Result<RegistryDocument, RegistryError> {
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
    pub(super) skills: BTreeMap<String, AuthState>,
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
