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
        return sha256(capability.name.as_bytes());
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
        sha256(&canonical)
    } else {
        sha256(format!("unreadable-skill-source\n{}", capability.name).as_bytes())
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
        Ok(sha256(&canonical))
    } else {
        Err(format!("cannot hash skill source {}", path.display()))
    }
}

pub(super) fn user_source_host_visible(
    host_home: &Path,
    active_host: &str,
    source: &UserSourceEntry,
) -> bool {
    let host_specific = match active_host {
        "codex" => Some(host_home.join(".codex/skills").join(&source.skill_id)),
        "claude-code" => Some(host_home.join(".claude/skills").join(&source.skill_id)),
        "omp" => Some(host_home.join(".omp/agent/skills").join(&source.skill_id)),
        "cursor" => Some(host_home.join(".cursor/skills").join(&source.skill_id)),
        "codebuddy-code" => Some(host_home.join(".codebuddy/skills").join(&source.skill_id)),
        _ => None,
    };
    let shared = matches!(active_host, "codex" | "omp" | "cursor")
        .then(|| host_home.join(".agents/skills").join(&source.skill_id));
    let existing = [host_specific, shared]
        .into_iter()
        .flatten()
        .filter(|entry| entry.exists() || std::fs::symlink_metadata(entry).is_ok())
        .collect::<Vec<_>>();
    existing.len() == 1
        && existing.iter().all(|entry| {
            std::fs::canonicalize(entry)
                .ok()
                .zip(std::fs::canonicalize(&source.canonical_path).ok())
                .is_some_and(|(actual, expected)| actual == expected)
                && entry.join("SKILL.md").is_file()
        })
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
        canonical.extend_from_slice(b"F\0");
        canonical.extend_from_slice(relative.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        canonical.extend_from_slice(&bytes);
        true
    } else {
        false
    }
}

pub(super) fn legacy_demand_tag(demand: SkillDemand) -> String {
    serde_json::to_value(demand)
        .ok()
        .and_then(|value| {
            Some(format!(
                "legacy:{}:{}",
                value.get("category")?.as_str()?,
                value.get("demand")?.as_str()?
            ))
        })
        .unwrap_or_else(|| "legacy:unknown".to_string())
}
