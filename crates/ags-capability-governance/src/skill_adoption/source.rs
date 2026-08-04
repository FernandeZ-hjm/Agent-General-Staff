use super::model::{AdoptionRoutingMetadata, PrivateSkillRecord};
use crate::{hash_skill_source, sha256};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatter {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    intent_tags: Vec<String>,
    #[serde(default)]
    positive_examples: Vec<String>,
    #[serde(default)]
    negative_examples: Vec<String>,
    #[serde(default)]
    entrypoints: Vec<String>,
    #[serde(default)]
    invoke_hint: String,
    #[serde(default)]
    requires_auth: bool,
    #[serde(default)]
    version: String,
}

pub(super) struct AuditedSource {
    pub source_dir: PathBuf,
    pub record: PrivateSkillRecord,
    pub warnings: Vec<String>,
}

pub(super) fn audit_local_source(
    source: &Path,
    target_hosts: Vec<String>,
    routing_metadata: Option<&Path>,
) -> Result<AuditedSource, String> {
    let source_dir = if source.is_file() {
        if source.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
            return Err("local file source must be SKILL.md".to_string());
        }
        source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        source.to_path_buf()
    };
    let source_dir = fs::canonicalize(&source_dir).map_err(|error| {
        format!(
            "cannot resolve local skill source {}: {error}",
            source.display()
        )
    })?;
    if !source_dir.is_dir() {
        return Err(format!(
            "local skill source is not a directory: {}",
            source_dir.display()
        ));
    }
    let skill_md = source_dir.join("SKILL.md");
    let content = fs::read_to_string(&skill_md)
        .map_err(|error| format!("cannot read {}: {error}", skill_md.display()))?;
    let mut frontmatter = parse_frontmatter(&content)?;
    let (routing_metadata_path, routing_metadata_hash) =
        load_routing_metadata(routing_metadata, &mut frontmatter)?;
    if !stable_skill_id(&frontmatter.name) {
        return Err("skill frontmatter name is not a stable identifier".to_string());
    }
    let summary = if frontmatter.summary.trim().is_empty() {
        frontmatter.description.trim()
    } else {
        frontmatter.summary.trim()
    };
    if summary.is_empty() {
        return Err("skill frontmatter requires description or summary".to_string());
    }

    let mut warnings = Vec::new();
    let mut files = 0usize;
    let mut total = 0u64;
    audit_tree(
        &source_dir,
        &source_dir,
        &mut files,
        &mut total,
        &mut warnings,
    )?;
    let source_hash = hash_skill_source(&source_dir)?;
    let license = find_license(&source_dir).ok_or_else(|| {
        "skill source has no LICENSE, LICENSE.md, LICENSE.txt, or COPYING file".to_string()
    })?;
    let license_bytes = fs::read(&license)
        .map_err(|error| format!("cannot read license {}: {error}", license.display()))?;
    let body_revision = source_hash.trim_start_matches("sha256:").to_string();
    let mut intent_tags = frontmatter.intent_tags;
    if intent_tags.is_empty() {
        intent_tags.push(frontmatter.name.clone());
    }
    intent_tags.sort();
    intent_tags.dedup();
    let mut entrypoints = frontmatter.entrypoints;
    entrypoints.sort();
    entrypoints.dedup();
    let invoke_hint = if frontmatter.invoke_hint.trim().is_empty() {
        format!("[skill: {}]", frontmatter.name)
    } else {
        frontmatter.invoke_hint
    };
    Ok(AuditedSource {
        source_dir: source_dir.clone(),
        record: PrivateSkillRecord {
            skill_id: frontmatter.name,
            source: source_dir.to_string_lossy().into_owned(),
            source_hash,
            license_path: license.to_string_lossy().into_owned(),
            license_hash: sha256(&license_bytes),
            routing_metadata_path,
            routing_metadata_hash,
            body_revision,
            summary: summary.to_string(),
            intent_tags,
            positive_examples: frontmatter.positive_examples,
            negative_examples: frontmatter.negative_examples,
            entrypoints,
            invoke_hint,
            requires_auth: frontmatter.requires_auth,
            version: frontmatter.version,
            target_hosts,
        },
        warnings,
    })
}

fn load_routing_metadata(
    path: Option<&Path>,
    frontmatter: &mut SkillFrontmatter,
) -> Result<(Option<String>, Option<String>), String> {
    let Some(path) = path else {
        return Ok((None, None));
    };
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect routing metadata {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "routing metadata must be a regular file: {}",
            path.display()
        ));
    }
    if metadata.len() > 64 * 1024 {
        return Err("routing metadata exceeds 65536 bytes".to_string());
    }
    let canonical = fs::canonicalize(path).map_err(|error| {
        format!(
            "cannot resolve routing metadata {}: {error}",
            path.display()
        )
    })?;
    let bytes = fs::read(&canonical).map_err(|error| {
        format!(
            "cannot read routing metadata {}: {error}",
            canonical.display()
        )
    })?;
    let overlay: AdoptionRoutingMetadata = serde_yaml::from_slice(&bytes)
        .map_err(|error| format!("invalid routing metadata {}: {error}", canonical.display()))?;
    if let Some(summary) = overlay.summary {
        frontmatter.summary = summary;
    }
    if !overlay.intent_tags.is_empty() {
        frontmatter.intent_tags = overlay.intent_tags;
    }
    if !overlay.positive_examples.is_empty() {
        frontmatter.positive_examples = overlay.positive_examples;
    }
    if !overlay.negative_examples.is_empty() {
        frontmatter.negative_examples = overlay.negative_examples;
    }
    if !overlay.entrypoints.is_empty() {
        frontmatter.entrypoints = overlay.entrypoints;
    }
    if let Some(invoke_hint) = overlay.invoke_hint {
        frontmatter.invoke_hint = invoke_hint;
    }
    if let Some(requires_auth) = overlay.requires_auth {
        frontmatter.requires_auth = requires_auth;
    }
    if let Some(version) = overlay.version {
        frontmatter.version = version;
    }
    Ok((
        Some(canonical.to_string_lossy().into_owned()),
        Some(sha256(&bytes)),
    ))
}

fn parse_frontmatter(content: &str) -> Result<SkillFrontmatter, String> {
    let rest = content
        .strip_prefix("---")
        .ok_or_else(|| "SKILL.md must begin with YAML frontmatter".to_string())?;
    let (yaml, _) = rest
        .split_once("\n---")
        .ok_or_else(|| "SKILL.md frontmatter is not closed".to_string())?;
    serde_yaml::from_str(yaml).map_err(|error| format!("invalid SKILL.md frontmatter: {error}"))
}

fn stable_skill_id(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn audit_tree(
    root: &Path,
    directory: &Path,
    files: &mut usize,
    total: &mut u64,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let relative = path.strip_prefix(root).unwrap_or(&path).display();
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink_refused: {relative}"));
        }
        if metadata.is_dir() {
            audit_tree(root, &path, files, total, warnings)?;
        } else if metadata.is_file() {
            *files += 1;
            *total = total.saturating_add(metadata.len());
            if *files > MAX_FILES {
                return Err(format!("skill source exceeds {MAX_FILES} files"));
            }
            if metadata.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "skill source file exceeds {MAX_FILE_BYTES} bytes: {relative}"
                ));
            }
            if *total > MAX_TOTAL_BYTES {
                return Err(format!(
                    "skill source exceeds {MAX_TOTAL_BYTES} total bytes"
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 != 0 {
                    warnings.push(format!(
                        "executable file present but never executed by adoption: {relative}"
                    ));
                }
            }
        } else {
            return Err(format!("special_file_refused: {relative}"));
        }
    }
    Ok(())
}

fn find_license(source: &Path) -> Option<PathBuf> {
    let mut current = Some(source);
    while let Some(directory) = current {
        for name in ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if directory.join(".git").exists() {
            break;
        }
        current = directory.parent();
    }
    None
}
