use super::model::{
    InstalledSkillIndex, InstalledSkillRecord, ReadInputSeal, INSTALLED_SKILL_INDEX_SCHEMA,
};
#[cfg(unix)]
use super::model::{ReadInputIdentity, ReadInputKind};
use std::path::{Path, PathBuf};

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) const MAX_INSTALLED_SKILL_REGISTRY_BYTES: u64 = 2 * 1024 * 1024;

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) struct ObservedInstalledSkillIndex {
    pub value: InstalledSkillIndex,
    pub raw_bytes: Option<Vec<u8>>,
    pub canonical_bytes: Vec<u8>,
    pub semantic_hash: String,
    pub seal: ReadInputSeal,
}

impl std::fmt::Debug for ObservedInstalledSkillIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObservedInstalledSkillIndex")
            .field("value", &self.value)
            .field("raw_bytes", &self.raw_bytes)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("semantic_hash", &self.semantic_hash)
            .field("seal", &self.seal)
            .finish_non_exhaustive()
    }
}

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
    observe_installed_skills(runtime_home).map(|observed| observed.value)
}

pub(super) fn observe_installed_skills(
    runtime_home: &Path,
) -> Result<ObservedInstalledSkillIndex, String> {
    #[cfg(not(unix))]
    {
        let _ = runtime_home;
        return Err("descriptor_semantics_unavailable_for_installed_skill_registry".to_string());
    }
    #[cfg(unix)]
    {
        use crate::shared_skill_source::{
            observe_optional_bounded_regular_file_at, DescriptorRoot,
            OptionalBoundedRegularFileObservation,
        };
        let runtime_parent = runtime_home
            .parent()
            .ok_or_else(|| "runtime home has no authorized parent".to_string())?;
        let runtime_name = runtime_home
            .file_name()
            .ok_or_else(|| "runtime home has no directory name".to_string())?;
        let mut authority_path = runtime_parent;
        let mut missing_prefix = PathBuf::new();
        let authority = loop {
            match DescriptorRoot::open_absolute(authority_path, "installed Skill registry") {
                Ok(authority) => break authority,
                Err(error) if error == "installed Skill registry_not_found" => {
                    let name = authority_path.file_name().ok_or_else(|| {
                        "installed Skill registry has no existing authorized ancestor".to_string()
                    })?;
                    missing_prefix = PathBuf::from(name).join(missing_prefix);
                    authority_path = authority_path.parent().ok_or_else(|| {
                        "installed Skill registry has no existing authorized ancestor".to_string()
                    })?;
                }
                Err(error) => return Err(error),
            }
        };
        let relative = missing_prefix
            .join(runtime_name)
            .join("stable-capabilities")
            .join("installed-skills.json");
        let path = installed_skill_index_path(runtime_home);
        let observation = observe_optional_bounded_regular_file_at(
            &authority,
            &relative,
            MAX_INSTALLED_SKILL_REGISTRY_BYTES,
            "installed Skill registry",
        );
        let (value, raw_bytes, seal) = match observation {
            Ok(OptionalBoundedRegularFileObservation::Present(observed)) => {
                let value = parse_installed_skills_bytes(&path, &observed.bytes)?;
                let raw = observed.bytes.clone();
                let seal = seal_registry_observation(
                    OptionalBoundedRegularFileObservation::Present(observed),
                );
                (value, Some(raw), seal)
            }
            Ok(OptionalBoundedRegularFileObservation::Absent(observed)) => {
                let value = InstalledSkillIndex::default();
                let seal = seal_registry_observation(
                    OptionalBoundedRegularFileObservation::Absent(observed),
                );
                (value, None, seal)
            }
            Err(error) if error.contains("exceeds 2097152 bytes") => {
                return Err("installed_skill_registry_exceeds_byte_limit".to_string())
            }
            Err(error) if error.contains("must be a regular file") => {
                return Err(format!(
                    "installed Skill registry must be a regular file: {error}"
                ))
            }
            Err(error) => return Err(error),
        };
        let canonical_bytes = serde_json::to_vec(&value)
            .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?;
        let semantic_hash = ags_platform::sha256(&canonical_bytes);
        Ok(ObservedInstalledSkillIndex {
            value,
            raw_bytes,
            canonical_bytes,
            semantic_hash,
            seal,
        })
    }
}

#[cfg(unix)]
fn seal_registry_observation(
    observed: crate::shared_skill_source::OptionalBoundedRegularFileObservation,
) -> ReadInputSeal {
    use crate::shared_skill_source::OptionalBoundedRegularFileObservation;
    match observed {
        OptionalBoundedRegularFileObservation::Present(observed) => ReadInputSeal {
            root: observed.parent.to_string_lossy().into_owned(),
            relative_path: observed.relative_path,
            kind: ReadInputKind::RegularFile,
            mode: observed.mode,
            identity: Some(ReadInputIdentity {
                device: observed.device,
                inode: observed.inode,
            }),
            digest: ags_platform::sha256(&observed.bytes),
        },
        OptionalBoundedRegularFileObservation::Absent(observed) => ReadInputSeal {
            root: observed.parent.to_string_lossy().into_owned(),
            relative_path: observed.relative_path.clone(),
            kind: ReadInputKind::Absent,
            mode: observed.mode,
            identity: Some(ReadInputIdentity {
                device: observed.device,
                inode: observed.inode,
            }),
            digest: ags_platform::sha256(
                [
                    b"ags-absent-installed-skill-registry-v1\n".as_slice(),
                    observed.relative_path.as_bytes(),
                ]
                .concat(),
            ),
        },
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn parse_installed_skills_bytes(
    path: &Path,
    bytes: &[u8],
) -> Result<InstalledSkillIndex, String> {
    let registry: InstalledSkillIndex = serde_json::from_slice(bytes).map_err(|error| {
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
    Ok(observe_installed_skills(runtime_home)?.semantic_hash)
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

#[cfg(all(test, unix))]
mod descriptor_loader_red_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn runtime(temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join("runtime")
    }

    fn write_default_registry(runtime: &Path) -> PathBuf {
        let path = installed_skill_index_path(runtime);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&InstalledSkillIndex::default()).unwrap(),
        )
        .unwrap();
        path
    }

    #[test]
    fn absent_registry_has_a_typed_descriptor_seal() {
        let temp = tempfile::TempDir::new().unwrap();
        let observed = observe_installed_skills(&runtime(&temp)).unwrap();
        assert_eq!(observed.value, InstalledSkillIndex::default());
        assert_eq!(observed.raw_bytes, None);
        assert_eq!(observed.seal.kind, ReadInputKind::Absent);
        assert!(observed.seal.identity.is_some());
        assert_eq!(
            Path::new(&observed.seal.root),
            temp.path().canonicalize().unwrap()
        );
        assert_eq!(
            observed.seal.relative_path,
            "runtime/stable-capabilities/installed-skills.json"
        );
    }

    #[test]
    fn installed_registry_loader_rejects_fifo_without_blocking() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = runtime(&temp);
        let path = installed_skill_index_path(&runtime);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success());

        let error = observe_installed_skills(&runtime).unwrap_err();
        assert!(error.contains("must be a regular file"), "{error}");
    }

    #[test]
    fn installed_registry_loader_rejects_growth_after_open() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = runtime(&temp);
        let path = write_default_registry(&runtime);
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            fs::write(
                path,
                vec![b' '; MAX_INSTALLED_SKILL_REGISTRY_BYTES as usize + 1],
            )
            .unwrap();
        }));

        assert_eq!(
            observe_installed_skills(&runtime).unwrap_err(),
            "installed_skill_registry_exceeds_byte_limit"
        );
    }

    #[test]
    fn installed_registry_loader_rejects_runtime_root_swap() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = runtime(&temp);
        write_default_registry(&runtime);
        let moved = runtime.with_file_name("runtime-held");
        let outside = runtime.with_file_name("outside-runtime");
        write_default_registry(&outside);
        let runtime_for_hook = runtime.clone();
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            fs::rename(&runtime_for_hook, &moved).unwrap();
            symlink(&outside, &runtime_for_hook).unwrap();
        }));

        let error = observe_installed_skills(&runtime).unwrap_err();
        assert!(
            error.contains("read_input_drift") || error.contains("root_identity_drift"),
            "{error}"
        );
    }

    #[test]
    fn absent_registry_rejects_deepest_held_tree_replacement() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = runtime(&temp);
        let stable = runtime.join("stable-capabilities");
        fs::create_dir_all(&stable).unwrap();
        let moved = runtime.join("stable-held");
        let stable_for_hook = stable.clone();
        crate::shared_skill_source::set_after_bounded_absent_first_stat_hook(Box::new(move || {
            fs::rename(&stable_for_hook, &moved).unwrap();
            fs::create_dir(&stable_for_hook).unwrap();
        }));

        let error = observe_installed_skills(&runtime)
            .expect_err("held absent registry parent replacement must fail");
        assert!(error.contains("drift"), "{error}");
    }

    #[test]
    fn absent_registry_rejects_first_missing_component_becoming_symlink() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = runtime(&temp);
        fs::create_dir_all(&runtime).unwrap();
        let stable = runtime.join("stable-capabilities");
        let outside = temp.path().join("outside-stable");
        fs::create_dir(&outside).unwrap();
        let stable_for_hook = stable.clone();
        let outside_for_hook = outside.clone();
        crate::shared_skill_source::set_after_bounded_absent_first_stat_hook(Box::new(move || {
            symlink(&outside_for_hook, &stable_for_hook).unwrap();
        }));

        let error =
            observe_installed_skills(&runtime).expect_err("absent component appearance must fail");
        assert!(
            error.contains("appeared") || error.contains("drift"),
            "{error}"
        );
        assert!(!outside.join("installed-skills.json").exists());
    }

    #[test]
    fn installed_registry_loader_rejects_oversize_before_parsing() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = temp.path().to_path_buf();
        let path = installed_skill_index_path(&runtime);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();

        let error = load_installed_skills(&runtime).unwrap_err();
        assert_eq!(error, "installed_skill_registry_exceeds_byte_limit");
    }

    #[test]
    fn installed_registry_loader_rejects_symlink_without_reading_target() {
        let temp = tempfile::TempDir::new().unwrap();
        let runtime = temp.path().to_path_buf();
        let path = installed_skill_index_path(&runtime);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let outside = temp.path().join("outside.json");
        fs::write(&outside, b"outside sentinel").unwrap();
        symlink(&outside, &path).unwrap();

        let error = load_installed_skills(&runtime).unwrap_err();
        assert!(error.contains("must be a regular file"), "{error}");
        assert_eq!(fs::read(outside).unwrap(), b"outside sentinel");
    }
}
