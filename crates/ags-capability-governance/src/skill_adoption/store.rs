use super::model::{InstalledSkillIndex, InstalledSkillRecord, INSTALLED_SKILL_INDEX_SCHEMA};
use std::fs;
use std::path::{Path, PathBuf};

pub fn installed_skill_index_path(runtime_home: &Path) -> PathBuf {
    ags_platform::RuntimeLayout::new(runtime_home).installed_skills()
}

pub fn bodies_root(runtime_home: &Path) -> PathBuf {
    ags_platform::RuntimeLayout::new(runtime_home).skill_bodies()
}

pub fn body_path(runtime_home: &Path, record: &InstalledSkillRecord) -> PathBuf {
    bodies_root(runtime_home)
        .join(&record.skill_id)
        .join(&record.body_revision)
}

pub fn load_installed_skills(runtime_home: &Path) -> Result<InstalledSkillIndex, String> {
    let path = installed_skill_index_path(runtime_home);
    if !path.exists() {
        return Ok(InstalledSkillIndex::default());
    }
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "cannot read installed Skill index {}: {error}",
            path.display()
        )
    })?;
    let registry: InstalledSkillIndex = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "cannot parse installed Skill index {}: {error}",
            path.display()
        )
    })?;
    if registry.schema_version != INSTALLED_SKILL_INDEX_SCHEMA {
        return Err(format!(
            "unsupported installed Skill index schema: {}",
            registry.schema_version
        ));
    }
    validate_installed_skill_index(&registry)?;
    Ok(registry)
}

pub fn installed_skill_index_hash(runtime_home: &Path) -> Result<String, String> {
    let registry = load_installed_skills(runtime_home)?;
    let bytes = serde_json::to_vec(&registry)
        .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?;
    Ok(ags_platform::sha256(&bytes))
}

pub(super) fn write_installed_skills(
    runtime_home: &Path,
    registry: &InstalledSkillIndex,
) -> Result<(), String> {
    validate_installed_skill_index(registry)?;
    let bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?;
    ags_platform::atomic_write(
        &installed_skill_index_path(runtime_home),
        &[bytes, b"\n".to_vec()].concat(),
    )
}

fn validate_installed_skill_index(index: &InstalledSkillIndex) -> Result<(), String> {
    for (key, record) in &index.skills {
        if key != &record.skill_id || record.skill_id.trim().is_empty() {
            return Err(format!(
                "installed Skill index identity mismatch: key={key} record={}",
                record.skill_id
            ));
        }
        match &record.source_spec {
            super::model::SourceSpec::Local { path } if path.trim().is_empty() => {
                return Err(format!(
                    "installed Skill `{key}` has an empty local source identity"
                ));
            }
            super::model::SourceSpec::GitHub { url, .. }
            | super::model::SourceSpec::Git { url, .. }
                if url.trim().is_empty() =>
            {
                return Err(format!(
                    "installed Skill `{key}` has an empty repository identity"
                ));
            }
            _ => {}
        }
        if record.body_revision.trim().is_empty()
            || record.source_hash.trim().is_empty()
            || record.body_revisions.is_empty()
        {
            return Err(format!(
                "installed Skill `{key}` has incomplete immutable revision state"
            ));
        }
        if !record
            .body_revisions
            .iter()
            .any(|revision| revision.revision == record.body_revision)
        {
            return Err(format!(
                "installed Skill `{key}` current revision is absent from history"
            ));
        }
        if record
            .body_revisions
            .iter()
            .any(|revision| revision.metadata.is_empty())
        {
            return Err(format!(
                "installed Skill `{key}` has a revision without immutable metadata"
            ));
        }
    }
    Ok(())
}
