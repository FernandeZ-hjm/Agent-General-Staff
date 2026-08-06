use ags_capability_governance::skill_adoption::{
    load_installed_skills, InstalledSkillIndex, InstalledSkillMetadata,
    INSTALLED_SKILL_INDEX_SCHEMA,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const RUNTIME_MIGRATION_RECEIPT_SCHEMA: &str = "0.5.0-runtime-state-migration";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMigrationStatus {
    Current,
    Initialized,
    Migrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMigrationReceipt {
    pub schema_version: String,
    pub status: RuntimeMigrationStatus,
    pub stable_root: String,
    pub state_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived_legacy_paths: Vec<String>,
}

#[derive(Debug)]
struct LegacyLayout {
    index: PathBuf,
    bodies: PathBuf,
    snapshots: PathBuf,
    projection: PathBuf,
}

#[derive(Debug)]
struct LegacyArchive {
    root: Option<PathBuf>,
    moves: Vec<(PathBuf, PathBuf)>,
}

struct MigrationStage {
    path: PathBuf,
}

impl MigrationStage {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for MigrationStage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl LegacyArchive {
    fn archived_paths(&self) -> Vec<String> {
        self.moves
            .iter()
            .map(|(_, archived)| archived.display().to_string())
            .collect()
    }

    fn rollback(&self) -> Result<(), String> {
        for (source, archived) in self.moves.iter().rev() {
            if let Some(parent) = source.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "cannot restore retired layout parent {}: {error}",
                        parent.display()
                    )
                })?;
            }
            fs::rename(archived, source).map_err(|error| {
                format!(
                    "cannot restore retired runtime path {}: {error}",
                    source.display()
                )
            })?;
        }
        if let Some(root) = &self.root {
            let _ = fs::remove_dir_all(root);
        }
        Ok(())
    }
}

impl LegacyLayout {
    fn new(runtime_home: &Path) -> Self {
        Self {
            index: runtime_home.join("skill-registry/private-skills.json"),
            bodies: runtime_home.join("skill-bodies"),
            snapshots: runtime_home.join("capability-snapshot"),
            projection: runtime_home.join("suite-skill-projection.json"),
        }
    }

    fn present(&self) -> bool {
        self.index.exists()
            || self.bodies.exists()
            || self.snapshots.exists()
            || self.projection.exists()
    }
}

/// One-way migration from the retired machine layout into the stable runtime
/// fact store. Normal readers never parse the retired schema or paths.
///
/// The complete candidate store is prepared beneath one staging runtime,
/// validated through the current read model, and activated by renaming the
/// single `stable-capabilities` directory. Retired paths are then moved under
/// the maintenance archive and are never consulted again.
pub fn migrate_runtime_state(runtime_home: &Path) -> Result<RuntimeMigrationReceipt, String> {
    let _lock = ags_platform::MaintenanceLock::acquire(runtime_home)?;
    let layout = ags_platform::RuntimeLayout::new(runtime_home);
    let legacy = LegacyLayout::new(runtime_home);
    fs::create_dir_all(runtime_home).map_err(|error| {
        format!(
            "cannot create runtime root {}: {error}",
            runtime_home.display()
        )
    })?;

    if layout.stable_capabilities().exists() {
        validate_current_state(runtime_home)?;
        let state_hash = hash_tree(&layout.stable_capabilities())?;
        let archive = archive_legacy(runtime_home, &legacy)?;
        let receipt = RuntimeMigrationReceipt {
            schema_version: RUNTIME_MIGRATION_RECEIPT_SCHEMA.to_string(),
            status: RuntimeMigrationStatus::Current,
            stable_root: layout.stable_capabilities().display().to_string(),
            state_hash,
            source_schema: None,
            archived_legacy_paths: archive.archived_paths(),
        };
        if let Err(error) = persist_receipt(runtime_home, &receipt) {
            archive.rollback()?;
            return Err(error);
        }
        return Ok(receipt);
    }

    let nonce = format!("{}-{}", std::process::id(), unix_nanos()?);
    let stage_runtime = runtime_home.join(format!(".stable-migration-{nonce}"));
    let _stage = MigrationStage::new(stage_runtime.clone());
    let stage_layout = ags_platform::RuntimeLayout::new(&stage_runtime);
    fs::create_dir_all(stage_layout.stable_capabilities()).map_err(|error| {
        format!(
            "cannot create stable capability migration stage {}: {error}",
            stage_layout.stable_capabilities().display()
        )
    })?;

    let had_legacy = legacy.present();
    let (index, source_schema) = if legacy.index.is_file() {
        migrate_legacy_index(&legacy.index)?
    } else {
        (InstalledSkillIndex::default(), None)
    };
    write_index(&stage_layout.installed_skills(), &index)?;

    if legacy.bodies.exists() {
        ags_platform::copy_regular_tree(&legacy.bodies, &stage_layout.skill_bodies())?;
    }
    if legacy.snapshots.exists() {
        ags_platform::copy_regular_tree(&legacy.snapshots, &stage_layout.capability_snapshots())?;
    }
    if legacy.projection.is_file() {
        copy_regular_file(&legacy.projection, &stage_layout.suite_projection_state())?;
    }

    validate_current_state(&stage_runtime)?;
    let staged_root = stage_layout.stable_capabilities();
    fs::rename(&staged_root, layout.stable_capabilities()).map_err(|error| {
        format!(
            "cannot activate stable capability store {}: {error}",
            layout.stable_capabilities().display()
        )
    })?;

    let archive = match archive_legacy(runtime_home, &legacy) {
        Ok(archive) => archive,
        Err(error) => {
            let _ = fs::remove_dir_all(layout.stable_capabilities());
            return Err(error);
        }
    };
    let status = if had_legacy {
        RuntimeMigrationStatus::Migrated
    } else {
        RuntimeMigrationStatus::Initialized
    };
    let receipt = RuntimeMigrationReceipt {
        schema_version: RUNTIME_MIGRATION_RECEIPT_SCHEMA.to_string(),
        status,
        stable_root: layout.stable_capabilities().display().to_string(),
        state_hash: hash_tree(&layout.stable_capabilities())?,
        source_schema,
        archived_legacy_paths: archive.archived_paths(),
    };
    if let Err(error) = persist_receipt(runtime_home, &receipt) {
        let remove = fs::remove_dir_all(layout.stable_capabilities()).map_err(|remove_error| {
            format!(
                "cannot rollback stable capability activation {}: {remove_error}",
                layout.stable_capabilities().display()
            )
        });
        let restore = archive.rollback();
        remove?;
        restore?;
        return Err(error);
    }
    Ok(receipt)
}

fn migrate_legacy_index(path: &Path) -> Result<(InstalledSkillIndex, Option<String>), String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "cannot read retired Skill index {}: {error}",
            path.display()
        )
    })?;
    let mut value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid retired Skill index {}: {error}", path.display()))?;
    let document = value
        .as_object_mut()
        .ok_or_else(|| "retired Skill index must be a JSON object".to_string())?;
    let source_schema = document
        .get("schema_version")
        .and_then(Value::as_str)
        .map(str::to_string);
    document.insert(
        "schema_version".to_string(),
        Value::String(INSTALLED_SKILL_INDEX_SCHEMA.to_string()),
    );
    document
        .entry("revision".to_string())
        .or_insert(Value::Number(0_u64.into()));
    let skills = document
        .entry("skills".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "retired Skill index skills must be an object".to_string())?;

    for (skill_id, raw_record) in skills.iter_mut() {
        normalize_legacy_record(skill_id, raw_record)?;
    }
    let mut index: InstalledSkillIndex = serde_json::from_value(value)
        .map_err(|error| format!("retired Skill index cannot migrate: {error}"))?;
    for record in index.skills.values_mut() {
        if record.body_revisions.is_empty() {
            record
                .body_revisions
                .push(ags_capability_governance::skill_adoption::BodyRevision::from_record(record));
        }
        let base = InstalledSkillMetadata::from_record(record);
        for revision in &mut record.body_revisions {
            if revision.metadata.is_empty() {
                let mut metadata = base.clone();
                metadata.body_revision = revision.revision.clone();
                metadata.source_hash = revision.source_hash.clone();
                metadata.resolved_source = revision.resolved_source.clone();
                revision.metadata = metadata;
            }
        }
    }
    Ok((index, source_schema))
}

fn normalize_legacy_record(skill_id: &str, value: &mut Value) -> Result<(), String> {
    let record = value
        .as_object_mut()
        .ok_or_else(|| format!("retired Skill record `{skill_id}` must be an object"))?;
    record
        .entry("skill_id".to_string())
        .or_insert_with(|| Value::String(skill_id.to_string()));
    let source = record
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let structured_source = match record.remove("source_spec") {
        Some(Value::Object(object)) => Value::Object(object),
        Some(Value::String(path)) if !path.is_empty() => local_source(path),
        _ => local_source(source),
    };
    record.insert("source_spec".to_string(), structured_source);
    record
        .entry("update_policy".to_string())
        .or_insert(Value::String("notify".to_string()));
    record
        .entry("catalog_review".to_string())
        .or_insert(Value::String("unreviewed".to_string()));
    record
        .entry("risk_findings".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    record
        .entry("body_revisions".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(())
}

fn local_source(path: String) -> Value {
    Value::Object(Map::from_iter([
        ("kind".to_string(), Value::String("local".to_string())),
        ("path".to_string(), Value::String(path)),
    ]))
}

fn validate_current_state(runtime_home: &Path) -> Result<(), String> {
    let index = load_installed_skills(runtime_home)?;
    for record in index.skills.values() {
        let body = ags_capability_governance::skill_adoption::body_path(runtime_home, record);
        if !body.is_dir() {
            return Err(format!(
                "installed Skill body is missing after migration: {}",
                body.display()
            ));
        }
        let observed = ags_capability_governance::hash_skill_source(&body)?;
        if observed != record.source_hash {
            return Err(format!(
                "installed Skill body hash mismatch after migration: {}",
                record.skill_id
            ));
        }
    }
    Ok(())
}

fn write_index(path: &Path, index: &InstalledSkillIndex) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(index)
        .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?;
    bytes.push(b'\n');
    ags_platform::atomic_write(path, &bytes)
}

fn archive_legacy(runtime_home: &Path, legacy: &LegacyLayout) -> Result<LegacyArchive, String> {
    let mut present = [
        legacy.index.clone(),
        legacy.bodies.clone(),
        legacy.snapshots.clone(),
        legacy.projection.clone(),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect::<Vec<_>>();
    if present.is_empty() {
        return Ok(LegacyArchive {
            root: None,
            moves: Vec::new(),
        });
    }
    present.sort();
    let archive = ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("migrations")
        .join(format!("retired-layout-{}", unix_nanos()?));
    fs::create_dir_all(&archive).map_err(|error| {
        format!(
            "cannot create migration archive {}: {error}",
            archive.display()
        )
    })?;
    let mut moves = Vec::new();
    for path in present {
        let relative = path.strip_prefix(runtime_home).map_err(|_| {
            format!(
                "retired runtime path escapes runtime root: {}",
                path.display()
            )
        })?;
        let target = archive.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create migration archive parent: {error}"))?;
        }
        if let Err(error) = fs::rename(&path, &target) {
            let archive = LegacyArchive {
                root: Some(archive.clone()),
                moves,
            };
            let rollback = archive.rollback();
            return Err(format!(
                "cannot archive retired runtime path {}: {error}; rollback={}",
                path.display(),
                rollback
                    .map(|_| "ok".to_string())
                    .unwrap_or_else(|error| error)
            ));
        }
        moves.push((path, target));
    }
    let _ = fs::remove_dir(runtime_home.join("skill-registry"));
    Ok(LegacyArchive {
        root: Some(archive),
        moves,
    })
}

fn persist_receipt(runtime_home: &Path, receipt: &RuntimeMigrationReceipt) -> Result<(), String> {
    let path = ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("migrations")
        .join("current.json");
    let mut bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("cannot serialize runtime migration receipt: {error}"))?;
    bytes.push(b'\n');
    ags_platform::atomic_write(&path, &bytes)
}

fn copy_regular_file(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        format!(
            "cannot inspect migration file {}: {error}",
            source.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "migration source must be a regular file: {}",
            source.display()
        ));
    }
    let bytes = fs::read(source)
        .map_err(|error| format!("cannot read migration file {}: {error}", source.display()))?;
    ags_platform::atomic_write(target, &bytes)
}

fn hash_tree(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut material = Vec::new();
    for (relative, path) in files {
        material.extend_from_slice(relative.as_bytes());
        material.push(0);
        material.extend_from_slice(
            &fs::read(&path).map_err(|error| {
                format!("cannot hash migrated state {}: {error}", path.display())
            })?,
        );
        material.push(0xff);
    }
    Ok(ags_platform::sha256(&material))
}

fn collect_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("cannot hash state tree {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate state tree: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot inspect state tree {}: {error}", path.display()))?;
        if kind.is_symlink() {
            return Err(format!(
                "stable state tree contains a symlink: {}",
                path.display()
            ));
        }
        if kind.is_dir() {
            collect_files(root, &path, out)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "stable state path escaped root".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, path));
        } else {
            return Err(format!(
                "stable state tree contains a special file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn unix_nanos() -> Result<u128, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_one_stable_runtime_fact_root() {
        let temp = tempfile::tempdir().unwrap();
        let receipt = migrate_runtime_state(temp.path()).unwrap();
        assert_eq!(receipt.status, RuntimeMigrationStatus::Initialized);
        let layout = ags_platform::RuntimeLayout::new(temp.path());
        assert!(layout.installed_skills().is_file());
        assert!(load_installed_skills(temp.path())
            .unwrap()
            .skills
            .is_empty());
    }

    #[test]
    fn moves_retired_layout_out_of_active_paths() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = LegacyLayout::new(temp.path());
        fs::create_dir_all(legacy.index.parent().unwrap()).unwrap();
        fs::write(
            &legacy.index,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "0.4.0-private-skill-registry",
                "revision": 0,
                "skills": {}
            }))
            .unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&legacy.snapshots).unwrap();
        fs::write(legacy.snapshots.join("note.txt"), b"fixture").unwrap();

        let receipt = migrate_runtime_state(temp.path()).unwrap();
        assert_eq!(receipt.status, RuntimeMigrationStatus::Migrated);
        assert!(!legacy.index.exists());
        assert!(!legacy.snapshots.exists());
        assert!(!receipt.archived_legacy_paths.is_empty());
        assert_eq!(
            load_installed_skills(temp.path()).unwrap().schema_version,
            INSTALLED_SKILL_INDEX_SCHEMA
        );
    }

    #[test]
    fn normal_reader_rejects_retired_schema_without_migration() {
        let temp = tempfile::tempdir().unwrap();
        let layout = ags_platform::RuntimeLayout::new(temp.path());
        fs::create_dir_all(layout.stable_capabilities()).unwrap();
        fs::write(
            layout.installed_skills(),
            br#"{"schema_version":"0.4.0-private-skill-registry","revision":0,"skills":{}}"#,
        )
        .unwrap();
        assert!(load_installed_skills(temp.path())
            .unwrap_err()
            .contains("unsupported installed Skill index schema"));
    }
}
