use super::*;
#[allow(unused_imports)]
use super::{authority::*, catalog::*, hashing::*, snapshot_compiler::*};

const MAX_HOST_REGISTRATION_BYTES: u64 = 64 * 1024;
const MAX_CAPABILITY_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalArtifactReadError {
    Unavailable(String),
    Refused(String),
    Stale(String),
}

#[cfg_attr(not(unix), allow(dead_code))]
fn classify_canonical_read_error(detail: String) -> CanonicalArtifactReadError {
    if detail.ends_with("_not_found") {
        CanonicalArtifactReadError::Unavailable(detail)
    } else if detail.contains("drift") || detail.contains("identity") {
        CanonicalArtifactReadError::Stale(detail)
    } else {
        CanonicalArtifactReadError::Refused(detail)
    }
}

fn canonical_host_id(
    host: &str,
) -> Result<ags_host_integration::HostId, CanonicalArtifactReadError> {
    let host_id =
        ags_host_integration::HostId::new(host).map_err(CanonicalArtifactReadError::Refused)?;
    if host_id.as_str() != host {
        return Err(CanonicalArtifactReadError::Refused(
            "canonical artifact host id is not normalized".to_string(),
        ));
    }
    Ok(host_id)
}

fn read_canonical_runtime_artifact(
    runtime_home: &Path,
    relative_path: &Path,
    maximum_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, CanonicalArtifactReadError> {
    #[cfg(unix)]
    {
        let root = crate::shared_skill_source::DescriptorRoot::open_absolute(runtime_home, label)
            .map_err(classify_canonical_read_error)?;
        let observed = crate::shared_skill_source::observe_bounded_regular_file_at(
            &root,
            relative_path,
            maximum_bytes,
            label,
        )
        .map_err(classify_canonical_read_error)?;
        Ok(observed.bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (runtime_home, relative_path, maximum_bytes, label);
        Err(CanonicalArtifactReadError::Refused(
            "descriptor_semantics_unavailable_for_canonical_runtime_artifact".to_string(),
        ))
    }
}

pub(crate) fn load_canonical_host_registration(
    runtime_home: &Path,
    host: &str,
) -> Result<ags_host_integration::HostRegistration, CanonicalArtifactReadError> {
    let host_id = canonical_host_id(host)?;
    let relative = PathBuf::from("hosts")
        .join(host_id.as_str())
        .join("registration.json");
    let bytes = read_canonical_runtime_artifact(
        runtime_home,
        &relative,
        MAX_HOST_REGISTRATION_BYTES,
        "canonical_host_registration",
    )?;
    let registration: ags_host_integration::HostRegistration = serde_json::from_slice(&bytes)
        .map_err(|error| CanonicalArtifactReadError::Stale(error.to_string()))?;
    if registration.host_id != host_id {
        return Err(CanonicalArtifactReadError::Stale(
            "host registration subject mismatch".to_string(),
        ));
    }
    Ok(registration)
}
#[derive(Debug)]
pub enum SnapshotBuildError {
    Read(std::io::Error),
    Registry(RegistryError),
    Resolve(ResolveError),
    Parse(serde_json::Error),
    Manifest(String),
}

/// Load the single persisted snapshot for one host and validate only its sealed
/// contents. Runtime reads never rebuild or compare live machine observations.
pub fn load_static_snapshot(
    runtime_home: &Path,
    active_host: &str,
) -> Result<(HostCapabilitySnapshot, ActiveCapabilityTables), SnapshotLoadError> {
    let host_id = canonical_host_id(active_host).map_err(SnapshotLoadError::CanonicalArtifact)?;
    let relative = PathBuf::from("stable-capabilities")
        .join("snapshots")
        .join(format!("{}.json", host_id.as_str()));
    let bytes = read_canonical_runtime_artifact(
        runtime_home,
        &relative,
        MAX_CAPABILITY_SNAPSHOT_BYTES,
        "canonical_capability_snapshot",
    )
    .map_err(SnapshotLoadError::CanonicalArtifact)?;
    let snapshot: HostCapabilitySnapshot =
        serde_json::from_slice(&bytes).map_err(|_| SnapshotLoadError::SkillSnapshotStale)?;
    let table = snapshot
        .validate_integrity(active_host)
        .map_err(SnapshotLoadError::Snapshot)?;
    Ok((snapshot, table))
}

#[derive(Debug)]
pub enum SnapshotLoadError {
    SkillSnapshotStale,
    CanonicalArtifact(CanonicalArtifactReadError),
    Snapshot(SnapshotError),
}

/// Validate persisted authorities that are intentionally outside the snapshot
/// body. This is used only at snapshot publish/activation boundaries; core
/// control-plane operations do not depend on host registration availability.
pub fn validate_snapshot_authorities(
    runtime_home: &Path,
    host: &str,
    snapshot: &HostCapabilitySnapshot,
) -> Result<(), String> {
    snapshot
        .validate_integrity(host)
        .map_err(|error| format!("capability_snapshot_invalid: {error:?}"))?;
    let registration =
        load_canonical_host_registration(runtime_home, host).map_err(|error| match error {
            CanonicalArtifactReadError::Unavailable(detail) => {
                format!("snapshot_required: canonical registration unavailable: {detail}")
            }
            CanonicalArtifactReadError::Refused(detail) => {
                format!("skill_snapshot_refused: canonical registration: {detail}")
            }
            CanonicalArtifactReadError::Stale(detail) => {
                format!("skill_snapshot_stale: canonical registration: {detail}")
            }
        })?;
    if registration.host_id.as_str() != host
        || registration.surface != snapshot.surface
        || registration.registration_hash != snapshot.host_registration_hash
    {
        return Err("skill_snapshot_stale: host registration digest drift".to_string());
    }
    let installed_index_hash = crate::skill_adoption::installed_skill_index_hash(runtime_home)?;
    if installed_index_hash != snapshot.installed_skill_index_hash {
        return Err("skill_snapshot_stale: installed Skill index digest drift".to_string());
    }
    if crate::snapshot_input_set_hash(snapshot) != snapshot.input_set_hash {
        return Err("skill_snapshot_stale: snapshot input_set_hash mismatch".to_string());
    }
    Ok(())
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
    verify_skill_tags_with_runtime_home(tags, active_host, &ags_platform::runtime_home())
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
                    "skill_snapshot_stale; run `ags govern capability snapshot --host <id>` and apply its action_ref".to_string()
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

#[cfg(all(test, unix))]
mod canonical_artifact_reader_tests {
    use super::*;
    use std::io::Write as _;
    use std::os::unix::fs::symlink;
    use std::process::Command;
    use std::time::{Duration, Instant};

    const REGISTRATION_FIFO_CHILD: &str = "AGS_REGISTRATION_FIFO_CHILD";
    const SNAPSHOT_FIFO_CHILD: &str = "AGS_SNAPSHOT_FIFO_CHILD";

    fn registration(host: &str) -> ags_host_integration::HostRegistration {
        ags_host_integration::HostRegistration::new(
            ags_host_integration::HostId::new(host).unwrap(),
            ags_host_integration::AgentSurface::Hybrid,
            None,
        )
    }

    fn registration_path(runtime: &Path, host: &str) -> PathBuf {
        runtime.join("hosts").join(host).join("registration.json")
    }

    fn write_registration(runtime: &Path, host: &str) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(&registration(host)).unwrap();
        let path = registration_path(runtime, host);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &bytes).unwrap();
        bytes
    }

    fn snapshot(host: &str) -> HostCapabilitySnapshot {
        HostCapabilitySnapshot::new(
            &registration(host),
            "sha256:registry",
            "sha256:runtime",
            "sha256:installed",
            Vec::new(),
            Vec::new(),
            "fixture://third-party",
            "sha256:third-party",
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn wait_bounded(mut child: std::process::Child) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let status = child.wait().unwrap();
                panic!("canonical artifact reader blocked on a FIFO: {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn registration_fifo_is_rejected_without_blocking() {
        if std::env::var_os(REGISTRATION_FIFO_CHILD).is_none() {
            let status = wait_bounded(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "snapshot_validation::canonical_artifact_reader_tests::registration_fifo_is_rejected_without_blocking",
                    ])
                    .env(REGISTRATION_FIFO_CHILD, "1")
                    .spawn()
                    .unwrap(),
            );
            assert!(status.success(), "FIFO child failed: {status}");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = registration_path(temp.path(), "hermes");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        assert!(matches!(
            load_canonical_host_registration(temp.path(), "hermes"),
            Err(CanonicalArtifactReadError::Refused(_))
        ));
    }

    #[test]
    fn registration_leaf_symlink_is_refused_even_with_exact_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = serde_json::to_vec_pretty(&registration("hermes")).unwrap();
        let outside = temp.path().join("outside-registration.json");
        std::fs::write(&outside, bytes).unwrap();
        let path = registration_path(temp.path(), "hermes");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(outside, path).unwrap();
        assert!(matches!(
            load_canonical_host_registration(temp.path(), "hermes"),
            Err(CanonicalArtifactReadError::Refused(_))
        ));
    }

    #[test]
    fn registration_same_inode_same_size_rewrite_during_read_is_stale() {
        let temp = tempfile::tempdir().unwrap();
        let original = write_registration(temp.path(), "hermes");
        let path = registration_path(temp.path(), "hermes");
        let mut replacement = original.clone();
        let index = replacement.iter().position(|byte| *byte == b'2').unwrap();
        replacement[index] = b'3';
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            file.write_all(&replacement).unwrap();
            file.sync_all().unwrap();
        }));
        assert!(matches!(
            load_canonical_host_registration(temp.path(), "hermes"),
            Err(CanonicalArtifactReadError::Stale(_))
        ));
    }

    #[test]
    fn registration_parent_replacement_during_read_is_stale() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = write_registration(temp.path(), "hermes");
        let host_dir = temp.path().join("hosts/hermes");
        let retired = temp.path().join("hosts/hermes-retired");
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            std::fs::rename(&host_dir, retired).unwrap();
            std::fs::create_dir(&host_dir).unwrap();
            std::fs::write(host_dir.join("registration.json"), bytes).unwrap();
        }));
        assert!(matches!(
            load_canonical_host_registration(temp.path(), "hermes"),
            Err(CanonicalArtifactReadError::Stale(_))
        ));
    }

    #[test]
    fn snapshot_fifo_is_rejected_without_blocking() {
        if std::env::var_os(SNAPSHOT_FIFO_CHILD).is_none() {
            let status = wait_bounded(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "snapshot_validation::canonical_artifact_reader_tests::snapshot_fifo_is_rejected_without_blocking",
                    ])
                    .env(SNAPSHOT_FIFO_CHILD, "1")
                    .spawn()
                    .unwrap(),
            );
            assert!(status.success(), "FIFO child failed: {status}");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = snapshot_path(temp.path(), "hermes");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        assert!(matches!(
            load_static_snapshot(temp.path(), "hermes"),
            Err(SnapshotLoadError::CanonicalArtifact(
                CanonicalArtifactReadError::Refused(_)
            ))
        ));
    }

    #[test]
    fn snapshot_leaf_symlink_is_refused_even_with_exact_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside-snapshot.json");
        std::fs::write(&outside, serde_json::to_vec(&snapshot("hermes")).unwrap()).unwrap();
        let path = snapshot_path(temp.path(), "hermes");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(outside, path).unwrap();
        assert!(matches!(
            load_static_snapshot(temp.path(), "hermes"),
            Err(SnapshotLoadError::CanonicalArtifact(
                CanonicalArtifactReadError::Refused(_)
            ))
        ));
    }
}
