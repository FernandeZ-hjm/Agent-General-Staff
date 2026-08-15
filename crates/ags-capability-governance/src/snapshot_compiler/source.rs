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
    hash_skill_source(&path).unwrap_or_else(|_| {
        ags_platform::sha256(format!("unreadable-skill-source\n{}", capability.name).as_bytes())
    })
}

pub fn hash_skill_source(path: &Path) -> Result<String, String> {
    crate::shared_skill_source::observe_skill_source(
        path,
        crate::shared_skill_source::SourcePolicy::Generic,
    )
    .map(|source| source.source_hash)
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
    let mut canonical = crate::shared_skill_source::SKILL_SOURCE_HASH_DOMAIN.to_vec();
    append_source_file_bytes(file_name, bytes, &mut canonical);
    Ok(ags_platform::sha256(&canonical))
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

    #[test]
    fn generic_hash_accepts_a_regular_file_input() {
        let root = tempfile::tempdir().unwrap();
        let body = b"---\nname: fixture\n---\n";
        let file = root.path().join("SKILL.md");
        std::fs::write(&file, body).unwrap();
        assert_eq!(
            hash_skill_source(&file).unwrap(),
            hash_single_file_skill_source("SKILL.md", body).unwrap()
        );
    }

    #[test]
    fn legacy_single_file_digest_is_byte_compatible_with_the_v1_domain() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("SKILL.md"), b"x").unwrap();
        assert_eq!(
            hash_skill_source(root.path()).unwrap(),
            "sha256:742468b49ce73260c9c3c8e18a18bf17e313464853846ee2bd834281082da2a4"
        );
        assert_eq!(
            hash_single_file_skill_source("SKILL.md", b"x").unwrap(),
            "sha256:742468b49ce73260c9c3c8e18a18bf17e313464853846ee2bd834281082da2a4"
        );
    }

    #[test]
    fn legacy_nested_preorder_digest_is_byte_compatible() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("a")).unwrap();
        std::fs::write(root.path().join("a/z"), b"y").unwrap();
        std::fs::write(root.path().join("b"), b"x").unwrap();
        assert_eq!(
            hash_skill_source(root.path()).unwrap(),
            "sha256:5a263dd9ad335e62ce9d5ce7ffebcebe3783f88db34b31d5c78bbbe6bc7757dc"
        );
    }

    #[cfg(unix)]
    #[test]
    fn generic_hash_encodes_a_symlink_target_without_following_it() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        symlink("outside-target", root.path().join("reference")).unwrap();
        let mut canonical = crate::shared_skill_source::SKILL_SOURCE_HASH_DOMAIN.to_vec();
        canonical.extend_from_slice(b"L\0reference\0outside-target\0");
        assert_eq!(
            hash_skill_source(root.path()).unwrap(),
            ags_platform::sha256(canonical)
        );
    }

    #[test]
    fn legacy_hash_entrypoint_rejects_oversize_and_excess_entries_before_collection() {
        let oversized = tempfile::tempdir().unwrap();
        std::fs::write(
            oversized.path().join("SKILL.md"),
            vec![0_u8; 2 * 1024 * 1024 + 1],
        )
        .unwrap();
        assert!(
            hash_skill_source(oversized.path()).is_err(),
            "generic hash accepted a file beyond the shared byte budget"
        );

        let crowded = tempfile::tempdir().unwrap();
        for index in 0..=512 {
            std::fs::write(crowded.path().join(format!("member-{index:04}")), b"x").unwrap();
        }
        assert!(
            hash_skill_source(crowded.path()).is_err(),
            "generic hash collected more entries than the shared member budget"
        );
    }

    #[test]
    fn legacy_hash_source_has_no_second_pathname_walker() {
        let source = include_str!("source.rs");
        assert!(!source.contains(concat!("std::fs::", "read_dir")));
        assert!(!source.contains(concat!("std::fs::", "read(path)")));
        assert!(!source.contains(concat!("append_source_", "directory")));
    }
}
