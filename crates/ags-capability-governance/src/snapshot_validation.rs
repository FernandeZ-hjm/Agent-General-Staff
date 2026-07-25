use super::*;
#[allow(unused_imports)]
use super::{authority::*, catalog::*, hashing::*, overlay_transaction::*, snapshot_compiler::*};
#[derive(Debug)]
pub enum SnapshotBuildError {
    Read(std::io::Error),
    Registry(RegistryError),
    Resolve(ResolveError),
    Parse(serde_json::Error),
    Overlay(String),
}

pub fn load_validated_snapshot(
    manifest_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<(HostCapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    load_validated_snapshot_with_roots(manifest_root, runtime_home, active_host, &host_home)
}

/// Hermetic validation seam: compare the persisted snapshot with a freshly
/// rebuilt catalog from the same explicit host roots. A snapshot that is
/// internally self-consistent but no longer matches current skill metadata or
/// bodies is stale, not valid.
pub fn load_validated_snapshot_with_roots(
    manifest_root: &Path,
    runtime_home: &Path,
    active_host: &str,
    host_home: &Path,
) -> Result<(HostCapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let expected =
        build_capability_snapshot_with_roots(manifest_root, active_host, runtime_home, host_home)
            .map_err(SnapshotLoadError::Build)?;
    let content = std::fs::read_to_string(snapshot_path(runtime_home, active_host))
        .map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let snapshot: HostCapabilitySnapshot =
        serde_json::from_str(&content).map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let _persisted_table = snapshot
        .validate(
            active_host,
            &expected.registry_hash,
            &expected.overlay_hash,
            &expected.runtime_hash,
        )
        .map_err(SnapshotLoadError::Snapshot)?;
    if snapshot.catalog_hash != expected.catalog_hash
        || snapshot.active_table_hash != expected.active_table_hash
        || snapshot.snapshot_hash != expected.snapshot_hash
    {
        return Err(SnapshotLoadError::Snapshot(
            SnapshotError::SkillSnapshotStale,
        ));
    }
    // Activity is advisory and deliberately excluded from catalog/snapshot
    // hashes. Return the freshly observed in-memory catalog after the persisted
    // authority snapshot passes validation, so Cold/Warm can advance without a
    // snapshot rewrite or lease invalidation.
    let table = ActiveSkillTable::new(
        expected.host.clone(),
        expected.snapshot_hash.clone(),
        expected.active_skills.clone(),
    )
    .map_err(|error| SnapshotLoadError::Snapshot(SnapshotError::InvalidActiveTable(error)))?;
    Ok((expected, table))
}

#[derive(Debug)]
pub enum SnapshotLoadError {
    SkillSnapshotStale,
    Build(SnapshotBuildError),
    Snapshot(SnapshotError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillTagVerdict {
    pub tag: String,
    pub accepted: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillTagGate {
    pub active_host: String,
    pub snapshot_hash: String,
    pub verdicts: Vec<SkillTagVerdict>,
    pub all_accepted: bool,
    pub rejected: Vec<String>,
}

pub fn verify_skill_tags(tags: &[String], manifest_root: &Path, active_host: &str) -> SkillTagGate {
    verify_skill_tags_with_runtime_home(tags, manifest_root, active_host, &locate_runtime_home())
}

pub fn verify_skill_tags_with_runtime_home(
    tags: &[String],
    manifest_root: &Path,
    active_host: &str,
    runtime_home: &Path,
) -> SkillTagGate {
    let host = if active_host.is_empty() {
        "<host-agnostic>"
    } else {
        active_host
    };
    let loaded = load_validated_snapshot(manifest_root, runtime_home, active_host);
    let (snapshot_hash, active_ids, stale) = match loaded {
        Ok((snapshot, table)) => (
            snapshot.snapshot_hash,
            table
                .active_skills()
                .into_iter()
                .map(|skill| skill.skill_id)
                .collect::<HashSet<_>>(),
            false,
        ),
        Err(_) => (String::new(), HashSet::new(), true),
    };
    let verdicts = tags
        .iter()
        .map(|tag| {
            let accepted = !stale && active_ids.contains(tag);
            SkillTagVerdict {
                tag: tag.clone(),
                accepted,
                reason: if accepted {
                    String::new()
                } else if stale {
                    "skill_snapshot_stale; run `ags capability snapshot --write`".to_string()
                } else {
                    format!("`[skill: {tag}]` is not active for host '{host}'")
                },
            }
        })
        .collect::<Vec<_>>();
    let rejected = verdicts
        .iter()
        .filter(|verdict| !verdict.accepted)
        .map(|verdict| verdict.tag.clone())
        .collect::<Vec<_>>();
    SkillTagGate {
        active_host: host.to_string(),
        snapshot_hash,
        all_accepted: rejected.is_empty(),
        rejected,
        verdicts,
    }
}
