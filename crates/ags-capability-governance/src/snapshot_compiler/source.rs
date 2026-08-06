use super::model::*;
use super::*;

pub(super) fn source_kind(capability: &ManagedCapability) -> SkillSourceKind {
    match capability.managed_status {
        ManagedStatus::SuiteManaged => SkillSourceKind::Suite,
        ManagedStatus::HostSystem => SkillSourceKind::HostSystem,
        ManagedStatus::ProjectLocal => SkillSourceKind::ProjectLocal,
        ManagedStatus::Discovered => {
            if capability
                .source
                .as_deref()
                .is_some_and(|source| source.contains("plugins/cache"))
            {
                SkillSourceKind::EnabledPlugin
            } else {
                SkillSourceKind::UserInstalled
            }
        }
        _ => SkillSourceKind::External,
    }
}

pub(crate) fn source_hash(manifest_root: &Path, capability: &ManagedCapability) -> String {
    let Some(path) = capability_source_path(manifest_root, capability) else {
        return ags_platform::sha256(capability.name.as_bytes());
    };
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    let hashed = if path.is_dir() {
        append_source_directory(&path, &path, &mut canonical)
    } else {
        append_source_node(
            path.parent().unwrap_or_else(|| Path::new(".")),
            &path,
            &mut canonical,
        )
    };
    if hashed {
        ags_platform::sha256(&canonical)
    } else {
        ags_platform::sha256(format!("unreadable-skill-source\n{}", capability.name).as_bytes())
    }
}

pub fn hash_skill_source(path: &Path) -> Result<String, String> {
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    let hashed = if path.is_dir() {
        append_source_directory(path, path, &mut canonical)
    } else {
        append_source_node(
            path.parent().unwrap_or_else(|| Path::new(".")),
            path,
            &mut canonical,
        )
    };
    if hashed {
        Ok(ags_platform::sha256(&canonical))
    } else {
        Err(format!("cannot hash skill source {}", path.display()))
    }
}

/// Hash a Skill body that consists of exactly one regular file using the same
/// canonical encoding as [`hash_skill_source`]. Public projection uses this
/// before the generated body exists on disk, so its manifest can bind the
/// complete projected body instead of confusing a raw file checksum with a
/// Skill source hash.
pub fn hash_single_file_skill_source(file_name: &str, bytes: &[u8]) -> Result<String, String> {
    let path = Path::new(file_name);
    if path.components().count() != 1
        || !matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err("single-file Skill source requires one safe file name".to_string());
    }
    let mut canonical = b"ags-skill-source-v1\n".to_vec();
    append_source_file_bytes(file_name, bytes, &mut canonical);
    Ok(ags_platform::sha256(&canonical))
}

/// Hash the complete skill body without timestamps or absolute paths. This
/// catches changes in referenced scripts/assets as well as `SKILL.md`. Symlinks
/// are represented by their link target and are never followed, avoiding
/// cycles or accidental traversal outside the skill body.
pub(super) fn append_source_directory(
    root: &Path,
    directory: &Path,
    canonical: &mut Vec<u8>,
) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .all(|path| append_source_node(root, path, canonical))
}

pub(super) fn append_source_node(root: &Path, path: &Path, canonical: &mut Vec<u8>) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    if metadata.file_type().is_symlink() {
        let Ok(target) = std::fs::read_link(path) else {
            return false;
        };
        canonical.extend_from_slice(b"L\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(target.to_string_lossy().as_bytes());
        canonical.push(0);
        true
    } else if metadata.is_dir() {
        canonical.extend_from_slice(b"D\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        append_source_directory(root, path, canonical)
    } else if metadata.is_file() {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        append_source_file_bytes(&relative, &bytes, canonical);
        true
    } else {
        false
    }
}

fn append_source_file_bytes(relative: &str, bytes: &[u8], canonical: &mut Vec<u8>) {
    canonical.extend_from_slice(b"F\0");
    canonical.extend_from_slice(relative.as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    canonical.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_single_file_hash_matches_the_filesystem_body_hash() {
        let root = tempfile::tempdir().unwrap();
        let body = b"---\nname: fixture\n---\n";
        std::fs::write(root.path().join("SKILL.md"), body).unwrap();
        assert_eq!(
            hash_single_file_skill_source("SKILL.md", body).unwrap(),
            hash_skill_source(root.path()).unwrap()
        );
        assert!(hash_single_file_skill_source("nested/SKILL.md", body).is_err());
        assert!(hash_single_file_skill_source("../SKILL.md", body).is_err());
    }
}
