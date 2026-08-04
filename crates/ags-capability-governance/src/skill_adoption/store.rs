use super::model::{PrivateSkillRecord, PrivateSkillRegistry, PRIVATE_SKILL_REGISTRY_SCHEMA};
use crate::{sha256, write_private_atomic};
use std::fs;
use std::path::{Path, PathBuf};

pub fn registry_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("skill-registry/private-skills.json")
}

pub fn bodies_root(runtime_home: &Path) -> PathBuf {
    runtime_home.join("skill-bodies")
}

pub fn body_path(runtime_home: &Path, record: &PrivateSkillRecord) -> PathBuf {
    bodies_root(runtime_home)
        .join(&record.skill_id)
        .join(&record.body_revision)
}

pub fn load_registry(runtime_home: &Path) -> Result<PrivateSkillRegistry, String> {
    let path = registry_path(runtime_home);
    if !path.exists() {
        return Ok(PrivateSkillRegistry::default());
    }
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "cannot read private skill registry {}: {error}",
            path.display()
        )
    })?;
    let registry: PrivateSkillRegistry = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "cannot parse private skill registry {}: {error}",
            path.display()
        )
    })?;
    if registry.schema_version != PRIVATE_SKILL_REGISTRY_SCHEMA {
        return Err(format!(
            "unsupported private skill registry schema: {}",
            registry.schema_version
        ));
    }
    Ok(registry)
}

pub fn registry_hash(runtime_home: &Path) -> Result<String, String> {
    let registry = load_registry(runtime_home)?;
    let bytes = serde_json::to_vec(&registry)
        .map_err(|error| format!("cannot serialize private skill registry: {error}"))?;
    Ok(sha256(&bytes))
}

pub(super) fn write_registry(
    runtime_home: &Path,
    registry: &PrivateSkillRegistry,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("cannot serialize private skill registry: {error}"))?;
    write_private_atomic(
        &registry_path(runtime_home),
        &[bytes, b"\n".to_vec()].concat(),
    )
}
