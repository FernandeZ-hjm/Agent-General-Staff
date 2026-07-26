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

/// Load the single persisted snapshot for one host and validate only its sealed
/// contents. Runtime reads never rebuild or compare live machine observations.
pub fn load_static_snapshot(
    runtime_home: &Path,
    active_host: &str,
) -> Result<(HostCapabilitySnapshot, ActiveSkillTable), SnapshotLoadError> {
    let content = std::fs::read_to_string(snapshot_path(runtime_home, active_host))
        .map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let snapshot: HostCapabilitySnapshot =
        serde_json::from_str(&content).map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let table = snapshot
        .validate_integrity(active_host)
        .map_err(SnapshotLoadError::Snapshot)?;
    Ok((snapshot, table))
}

#[derive(Debug)]
pub enum SnapshotLoadError {
    SkillSnapshotStale,
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

pub fn verify_skill_tags(tags: &[String], active_host: &str) -> SkillTagGate {
    verify_skill_tags_with_runtime_home(tags, active_host, &locate_runtime_home())
}

pub fn verify_skill_tags_with_runtime_home(
    tags: &[String],
    active_host: &str,
    runtime_home: &Path,
) -> SkillTagGate {
    let host = if active_host.is_empty() {
        "<host-agnostic>"
    } else {
        active_host
    };
    let loaded = load_static_snapshot(runtime_home, active_host);
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
