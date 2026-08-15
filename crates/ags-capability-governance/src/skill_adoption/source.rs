#![cfg_attr(not(unix), allow(dead_code))]

use super::model::{
    AdoptionRoutingMetadata, BodyRevision, CatalogReviewStatus, InstalledSkillRecord,
    MaterializedBodyNode, ReadInputIdentity, ReadInputKind, ReadInputSeal, RiskFinding, SourceSpec,
    UpdatePolicy,
};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SIDE_INPUT_BYTES: u64 = 64 * 1024;

pub(super) const MAX_FILES: usize = 512;
pub(super) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

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

#[derive(Debug)]
pub(super) struct AuditedSource {
    pub source_dir: PathBuf,
    pub record: InstalledSkillRecord,
    pub risk_findings: Vec<RiskFinding>,
    pub read_inputs: Vec<ReadInputSeal>,
}

struct RoutingMetadataObservation {
    path: Option<String>,
    hash: Option<String>,
    seal: Option<ReadInputSeal>,
}

enum RoutingMetadataLocation {
    SourceRelative {
        relative_path: String,
        display: String,
    },
    External(PathBuf),
}

#[cfg(all(test, unix))]
thread_local! {
    static SIDE_INPUT_OBSERVER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ROUTING_SIDE_OBSERVER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Parse a GitHub repository/tree/blob URL into a canonical source identity.
///
/// The parser is deliberately offline.  It never resolves a ref or probes the
/// repository.  When a URL could be interpreted with a slash-containing ref,
/// callers must supply `requested_ref`; otherwise the ambiguous form is
/// rejected instead of being silently guessed.
pub fn parse_github_url(url: &str, requested_ref: Option<&str>) -> Result<SourceSpec, String> {
    if url.is_empty() || !url.starts_with("https://") {
        return Err("GitHub source must use https://".to_string());
    }
    if url.contains('?') || url.contains('#') {
        return Err("GitHub source must not contain query or fragment".to_string());
    }
    let authority_and_path = &url["https://".len()..];
    let (authority, raw_path) = authority_and_path
        .split_once('/')
        .ok_or_else(|| "GitHub source must include owner and repository".to_string())?;
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains(':')
        || !authority.eq_ignore_ascii_case("github.com")
    {
        return Err(
            "GitHub source host must be github.com without credentials or a port".to_string(),
        );
    }
    let mut parts = raw_path.split('/').collect::<Vec<_>>();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() < 2 {
        return Err("GitHub source must include non-empty owner and repository".to_string());
    }
    if parts.iter().any(|part| {
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part.contains('\\')
            || part.contains('%')
            || part.chars().any(|character| character.is_control())
    }) {
        return Err("GitHub source contains an invalid or traversing path segment".to_string());
    }
    let owner = parts[0];
    let mut repo = parts[1].to_string();
    if repo.ends_with(".git") {
        repo.truncate(repo.len() - ".git".len());
    }
    if owner.is_empty() || repo.is_empty() {
        return Err("GitHub source owner and repository must not be empty".to_string());
    }
    let canonical_url = format!("https://github.com/{owner}/{repo}");
    let (requested_ref, subdir) = match parts.get(2).copied() {
        None => (validate_requested_ref(requested_ref)?, None),
        Some("tree") => parse_tree_or_blob_path(&parts[3..], requested_ref, false)?,
        Some("blob") => parse_tree_or_blob_path(&parts[3..], requested_ref, true)?,
        Some(_) => return Err("GitHub source path must use /tree/ or /blob/".to_string()),
    };
    let tracking_ref = requested_ref.clone();
    Ok(SourceSpec::GitHub {
        url: canonical_url,
        requested_ref,
        tracking_ref,
        subdir,
    })
}

/// Alias kept for callers that name the operation after the source type.
pub fn parse_github_source(url: &str, requested_ref: Option<&str>) -> Result<SourceSpec, String> {
    parse_github_url(url, requested_ref)
}

pub fn parse_github_url_with_ref(
    url: &str,
    requested_ref: Option<String>,
) -> Result<SourceSpec, String> {
    parse_github_url(url, requested_ref.as_deref())
}

fn parse_tree_or_blob_path(
    remaining: &[&str],
    requested_ref: Option<&str>,
    blob: bool,
) -> Result<(Option<String>, Option<String>), String> {
    if remaining.is_empty() {
        return Err("GitHub tree/blob source is missing its ref".to_string());
    }
    let requested_ref = validate_requested_ref(requested_ref)?;
    let (resolved_ref, tail) = if let Some(requested_ref) = requested_ref.as_deref() {
        let ref_parts = requested_ref.split('/').collect::<Vec<_>>();
        if remaining.len() < ref_parts.len() || remaining[..ref_parts.len()] != ref_parts[..] {
            return Err("requested_ref does not match the GitHub URL path".to_string());
        }
        (requested_ref.to_string(), &remaining[ref_parts.len()..])
    } else {
        // A short path can be safely interpreted as a conventional one-segment
        // ref.  More nested forms are ambiguous with a slash-containing ref;
        // require the caller to make the boundary explicit.
        let maximum_tail = if blob { 2 } else { 1 };
        if remaining.len() > maximum_tail + 1 {
            return Err(
                "ambiguous GitHub ref; pass requested_ref for slash-containing refs".to_string(),
            );
        }
        (remaining[0].to_string(), &remaining[1..])
    };
    if resolved_ref.is_empty() {
        return Err("GitHub ref must not be empty".to_string());
    }
    if blob {
        if tail.last().copied() != Some("SKILL.md") {
            return Err("GitHub blob source must select SKILL.md".to_string());
        }
        let subdir_parts = &tail[..tail.len() - 1];
        let subdir = normalize_subdir(subdir_parts)?;
        Ok((Some(resolved_ref), (!subdir.is_empty()).then_some(subdir)))
    } else {
        let subdir = normalize_subdir(tail)?;
        Ok((Some(resolved_ref), (!subdir.is_empty()).then_some(subdir)))
    }
}

fn validate_requested_ref(requested_ref: Option<&str>) -> Result<Option<String>, String> {
    let Some(requested_ref) = requested_ref else {
        return Ok(None);
    };
    if requested_ref.is_empty()
        || requested_ref.starts_with('-')
        || requested_ref.contains('\\')
        || requested_ref.contains("..")
        || requested_ref.contains("@{")
        || requested_ref
            .chars()
            .any(|character| character.is_control())
        || requested_ref
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("GitHub requested_ref is empty, unsafe, or traversing".to_string());
    }
    Ok(Some(requested_ref.to_string()))
}

pub(super) fn normalize_subdir(parts: &[&str]) -> Result<String, String> {
    if parts.iter().any(|part| {
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part.contains('\\')
            || part.contains('%')
            || part.chars().any(|character| character.is_control())
    }) {
        return Err("GitHub source subdir contains an invalid or traversing segment".to_string());
    }
    Ok(parts.join("/"))
}

pub(super) fn validate_source_subdir(subdir: Option<&str>) -> Result<String, String> {
    let Some(subdir) = subdir else {
        return Ok(String::new());
    };
    normalize_subdir(&subdir.split('/').collect::<Vec<_>>())
}

pub(super) fn audit_local_source(
    source: &Path,
    target_hosts: Vec<String>,
    routing_metadata: Option<&Path>,
) -> Result<AuditedSource, String> {
    audit_local_source_with_boundary(source, target_hosts, routing_metadata, None)
}

pub(super) fn audit_local_source_with_boundary(
    source: &Path,
    target_hosts: Vec<String>,
    routing_metadata: Option<&Path>,
    repository_root: Option<&Path>,
) -> Result<AuditedSource, String> {
    let source_dir = if source.is_file() {
        if fs::symlink_metadata(source).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("symlink_refused: local skill source file".to_string());
        }
        if source.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
            return Err("local file source must be SKILL.md".to_string());
        }
        source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        if fs::symlink_metadata(source).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err("symlink_refused: local skill source directory".to_string());
        }
        source.to_path_buf()
    };
    let lexical_source_dir = source_dir.clone();
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
    let snapshot = super::materialize::snapshot_skill_source(&source_dir)?;
    let skill_md = source_dir.join("SKILL.md");
    let skill_md_bytes = snapshot_file_bytes(&snapshot, "SKILL.md")
        .ok_or_else(|| format!("cannot read {}: missing regular file", skill_md.display()))?;
    let content = std::str::from_utf8(skill_md_bytes)
        .map_err(|error| format!("cannot read {} as UTF-8: {error}", skill_md.display()))?;
    let mut frontmatter = parse_frontmatter(content)?;
    let routing_metadata = load_routing_metadata(
        routing_metadata,
        &source_dir,
        &lexical_source_dir,
        &snapshot,
        &mut frontmatter,
    )?;
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

    let mut risk_findings = risks_from_snapshot(&snapshot);
    let source_hash = snapshot.source_hash.clone();
    let license = find_license(&source_dir, repository_root);
    let mut side_input_seals = Vec::new();
    let (license_path, license_hash) = if let Some(license) = license {
        let relative = license
            .strip_prefix(&source_dir)
            .ok()
            .and_then(|path| path.to_str())
            .map(|path| path.replace('\\', "/"));
        let (license_bytes, license_seal) = if let Some(relative) = relative.as_deref() {
            match (
                snapshot_file_bytes(&snapshot, relative),
                snapshot_file_seal(&snapshot, relative),
            ) {
                (Some(bytes), Some(seal)) => (bytes.to_vec(), Some(seal.clone())),
                _ => observe_side_input(&license, "license")?,
            }
        } else {
            observe_side_input(&license, "license")?
        };
        if let Some(seal) = license_seal {
            if !snapshot.seals.contains(&seal) {
                side_input_seals.push(seal);
            }
        }
        if !known_license(&license_bytes) {
            risk_findings.push(RiskFinding::acknowledgement(
                "unknown_license",
                safe_relative_path(&source_dir, &license),
                "license file is present but its identifier was not recognized",
            ));
        }
        (
            license.to_string_lossy().into_owned(),
            ags_platform::sha256(&license_bytes),
        )
    } else {
        risk_findings.push(RiskFinding::acknowledgement(
            "missing_license",
            None,
            "no supported LICENSE or COPYING file was found",
        ));
        (String::new(), String::new())
    };
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
    let source_string = source_dir.to_string_lossy().into_owned();
    let mut record = InstalledSkillRecord {
        skill_id: frontmatter.name,
        source: source_string.clone(),
        source_hash: source_hash.clone(),
        license_path,
        license_hash,
        routing_metadata_path: routing_metadata.path,
        routing_metadata_hash: routing_metadata.hash,
        body_revision: body_revision.clone(),
        summary: summary.to_string(),
        intent_tags,
        positive_examples: frontmatter.positive_examples,
        negative_examples: frontmatter.negative_examples,
        entrypoints,
        invoke_hint,
        requires_auth: frontmatter.requires_auth,
        version: frontmatter.version,
        target_hosts,
        source_spec: SourceSpec::Local {
            path: source_string,
        },
        resolved_source: None,
        update_policy: UpdatePolicy::Notify,
        catalog_review: CatalogReviewStatus::Unreviewed,
        risk_findings: risk_findings.clone(),
        body_revisions: Vec::new(),
        installed_at: 0,
    };
    record.body_revisions = vec![BodyRevision::from_record(&record)];
    if let Some(seal) = routing_metadata.seal {
        if !snapshot.seals.contains(&seal) {
            side_input_seals.push(seal);
        }
    }
    let mut read_inputs = snapshot.seals.clone();
    read_inputs.extend(side_input_seals);
    read_inputs.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then(left.relative_path.cmp(&right.relative_path))
    });
    read_inputs.dedup();
    Ok(AuditedSource {
        source_dir: source_dir.clone(),
        record,
        risk_findings,
        read_inputs,
    })
}

#[cfg(unix)]
pub(super) fn audit_remote_source_snapshots(
    source_dir: PathBuf,
    checkout_root: &Path,
    subdir: &str,
    target_hosts: Vec<String>,
    snapshot: super::materialize::SkillSourceSnapshot,
    checkout_snapshot: &super::materialize::SkillSourceSnapshot,
) -> Result<AuditedSource, String> {
    let skill_md = source_dir.join("SKILL.md");
    let skill_md_bytes = snapshot_file_bytes(&snapshot, "SKILL.md")
        .ok_or_else(|| format!("cannot read {}: missing regular file", skill_md.display()))?;
    let content = std::str::from_utf8(skill_md_bytes)
        .map_err(|error| format!("cannot read {} as UTF-8: {error}", skill_md.display()))?;
    let frontmatter = parse_frontmatter(content)?;
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
    let mut risk_findings = risks_from_snapshot(&snapshot);
    let mut license = None;
    let mut directories = vec![subdir.trim_matches('/').to_string()];
    while let Some(last) = directories.last().cloned() {
        if last.is_empty() {
            break;
        }
        directories.push(
            Path::new(&last)
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy()
                .into_owned(),
        );
    }
    if directories.last().is_none_or(|last| !last.is_empty()) {
        directories.push(String::new());
    }
    'search: for directory in directories {
        for name in ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"] {
            let relative = if directory.is_empty() {
                name.to_string()
            } else {
                format!("{directory}/{name}")
            };
            if let Some(bytes) = snapshot_file_bytes(checkout_snapshot, &relative) {
                license = Some((relative, bytes));
                break 'search;
            }
        }
    }
    let (license_path, license_hash) = if let Some((relative, bytes)) = license {
        if !known_license(bytes) {
            risk_findings.push(RiskFinding::acknowledgement(
                "unknown_license",
                Some(relative.clone()),
                "license file is present but its identifier was not recognized",
            ));
        }
        (
            checkout_root.join(relative).to_string_lossy().into_owned(),
            ags_platform::sha256(bytes),
        )
    } else {
        risk_findings.push(RiskFinding::acknowledgement(
            "missing_license",
            None,
            "no supported LICENSE or COPYING file was found",
        ));
        (String::new(), String::new())
    };
    let source_hash = snapshot.source_hash.clone();
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
    let source_string = source_dir.to_string_lossy().into_owned();
    let mut record = InstalledSkillRecord {
        skill_id: frontmatter.name,
        source: source_string.clone(),
        source_hash: source_hash.clone(),
        license_path,
        license_hash,
        routing_metadata_path: None,
        routing_metadata_hash: None,
        body_revision: source_hash.trim_start_matches("sha256:").to_string(),
        summary: summary.to_string(),
        intent_tags,
        positive_examples: frontmatter.positive_examples,
        negative_examples: frontmatter.negative_examples,
        entrypoints,
        invoke_hint,
        requires_auth: frontmatter.requires_auth,
        version: frontmatter.version,
        target_hosts,
        source_spec: SourceSpec::Local {
            path: source_string,
        },
        resolved_source: None,
        update_policy: UpdatePolicy::Notify,
        catalog_review: CatalogReviewStatus::Unreviewed,
        risk_findings: risk_findings.clone(),
        body_revisions: Vec::new(),
        installed_at: 0,
    };
    record.body_revisions = vec![BodyRevision::from_record(&record)];
    let mut read_inputs = snapshot.seals.clone();
    read_inputs.extend(checkout_snapshot.seals.clone());
    Ok(AuditedSource {
        source_dir,
        record,
        risk_findings,
        read_inputs,
    })
}

fn snapshot_file_bytes<'a>(
    snapshot: &'a super::materialize::SkillSourceSnapshot,
    relative_path: &str,
) -> Option<&'a [u8]> {
    snapshot.nodes.iter().find_map(|node| match node {
        MaterializedBodyNode::RegularFile {
            relative_path: candidate,
            bytes,
            ..
        } if candidate == relative_path => Some(bytes.as_slice()),
        _ => None,
    })
}

fn snapshot_file_seal<'a>(
    snapshot: &'a super::materialize::SkillSourceSnapshot,
    relative_path: &str,
) -> Option<&'a ReadInputSeal> {
    snapshot
        .seals
        .iter()
        .find(|seal| seal.relative_path == relative_path && seal.kind == ReadInputKind::RegularFile)
}

fn observe_side_input(
    path: &Path,
    label: &str,
) -> Result<(Vec<u8>, Option<ReadInputSeal>), String> {
    #[cfg(all(test, unix))]
    SIDE_INPUT_OBSERVER_CALLS.with(|calls| calls.set(calls.get() + 1));
    let observed = crate::shared_skill_source::observe_bounded_regular_file(
        path,
        MAX_SIDE_INPUT_BYTES,
        label,
    )?;
    let seal = ReadInputSeal {
        root: observed.parent.to_string_lossy().into_owned(),
        relative_path: observed.relative_path,
        kind: ReadInputKind::RegularFile,
        mode: observed.mode,
        identity: Some(ReadInputIdentity {
            device: observed.device,
            inode: observed.inode,
        }),
        digest: ags_platform::sha256(&observed.bytes),
    };
    Ok((observed.bytes, Some(seal)))
}

fn risks_from_snapshot(snapshot: &super::materialize::SkillSourceSnapshot) -> Vec<RiskFinding> {
    let mut risks = Vec::new();
    for node in &snapshot.nodes {
        let MaterializedBodyNode::RegularFile {
            relative_path,
            bytes,
            mode,
        } = node
        else {
            continue;
        };
        if is_scriptish_snapshot(relative_path, *mode) {
            risks.push(RiskFinding::acknowledgement(
                "script_or_executable_content",
                Some(relative_path.clone()),
                "script or executable content is retained but never executed by adoption",
            ));
        }
        if bytes_contain_suspected_secret(bytes) {
            risks.push(RiskFinding::acknowledgement(
                "suspected_sensitive_content",
                Some(relative_path.clone()),
                "content matches a sensitive-material heuristic; body bytes are withheld",
            ));
        }
    }
    risks
}

fn is_scriptish_snapshot(path: &str, mode: u32) -> bool {
    if mode & 0o111 != 0 {
        return true;
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == "install.sh"
        || name == "setup.sh"
        || name == "uninstall.sh"
        || name == "run.sh"
        || name.ends_with(".sh")
        || name.ends_with(".bash")
        || name.ends_with(".py")
        || name.ends_with(".js")
        || name.ends_with(".mjs")
        || name.ends_with(".command")
        || name.ends_with(".bat")
        || name.ends_with(".cmd")
        || name.ends_with(".ps1")
}

fn bytes_contain_suspected_secret(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    text.contains("-----BEGIN ")
        || text.contains("ghp_")
        || text.contains("github_pat_")
        || text.contains("AKIA")
        || lower.contains("private_key")
        || lower.contains("client_secret")
        || lower.contains("access_token")
}

fn load_routing_metadata(
    path: Option<&Path>,
    source_root: &Path,
    lexical_source_root: &Path,
    snapshot: &super::materialize::SkillSourceSnapshot,
    frontmatter: &mut SkillFrontmatter,
) -> Result<RoutingMetadataObservation, String> {
    let Some(path) = path else {
        return Ok(RoutingMetadataObservation {
            path: None,
            hash: None,
            seal: None,
        });
    };
    let location = classify_routing_metadata(path, source_root, lexical_source_root)?;
    let (bytes, observed_path, seal) = match location {
        RoutingMetadataLocation::SourceRelative {
            relative_path,
            display,
        } => {
            let bytes = snapshot_file_bytes(snapshot, &relative_path).ok_or_else(|| {
                format!("metadata_file_not_found: --metadata file does not exist: {display}")
            })?;
            let seal = snapshot_file_seal(snapshot, &relative_path)
                .cloned()
                .ok_or_else(|| format!("routing metadata is not a regular file: {display}"))?;
            (bytes.to_vec(), display, seal)
        }
        RoutingMetadataLocation::External(path) => {
            #[cfg(all(test, unix))]
            ROUTING_SIDE_OBSERVER_CALLS.with(|calls| calls.set(calls.get() + 1));
            let observed = match crate::shared_skill_source::observe_bounded_regular_file(
                &path,
                MAX_SIDE_INPUT_BYTES,
                "routing metadata",
            ) {
                Ok(observed) => observed,
                Err(error) if error == "routing metadata_not_found" => {
                    let value = path.to_string_lossy();
                    if value.contains('\n')
                        || value.trim_start().starts_with("---")
                        || value.contains(": ")
                        || value.trim_end().ends_with(':')
                    {
                        return Err(
                    "metadata_argument_requires_file: --metadata accepts an existing YAML file path (<FILE>), not inline YAML"
                        .to_string(),
                );
                    }
                    return Err(format!(
                        "metadata_file_not_found: --metadata file does not exist: {}",
                        path.display()
                    ));
                }
                Err(error) => {
                    return Err(error);
                }
            };
            let seal = ReadInputSeal {
                root: observed.parent.to_string_lossy().into_owned(),
                relative_path: observed.relative_path,
                kind: ReadInputKind::RegularFile,
                mode: observed.mode,
                identity: Some(ReadInputIdentity {
                    device: observed.device,
                    inode: observed.inode,
                }),
                digest: ags_platform::sha256(&observed.bytes),
            };
            (
                observed.bytes,
                observed.path.to_string_lossy().into_owned(),
                seal,
            )
        }
    };
    let overlay: AdoptionRoutingMetadata = serde_yaml::from_slice(&bytes)
        .map_err(|error| format!("invalid routing metadata {observed_path}: {error}"))?;
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
    Ok(RoutingMetadataObservation {
        path: Some(observed_path),
        hash: Some(ags_platform::sha256(&bytes)),
        seal: Some(seal),
    })
}

fn classify_routing_metadata(
    path: &Path,
    source_root: &Path,
    lexical_source_root: &Path,
) -> Result<RoutingMetadataLocation, String> {
    use std::path::Component;

    let (candidate, display) = if path.is_absolute() {
        match path
            .strip_prefix(lexical_source_root)
            .or_else(|_| path.strip_prefix(source_root))
        {
            Ok(relative) => (relative, path.to_string_lossy().into_owned()),
            Err(_) => return Ok(RoutingMetadataLocation::External(path.to_path_buf())),
        }
    } else {
        (path, path.to_string_lossy().into_owned())
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => normalized.push(name),
            _ => return Err("routing_metadata_traversal_refused".to_string()),
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("routing_metadata_path_has_no_file".to_string());
    }
    let relative_path = normalized.to_string_lossy().replace('\\', "/");
    Ok(RoutingMetadataLocation::SourceRelative {
        relative_path,
        display,
    })
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

fn known_license(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    [
        "mit license",
        "mit ",
        "apache license",
        "bsd license",
        "isc license",
        "gnu general public license",
        "mozilla public license",
        "unlicense",
        "spdx-license-identifier-",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn safe_relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn find_license(source: &Path, repository_root: Option<&Path>) -> Option<PathBuf> {
    let mut current = Some(source);
    while let Some(directory) = current {
        for name in ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"] {
            let candidate = directory.join(name);
            if fs::symlink_metadata(&candidate)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Some(candidate);
            }
        }
        if repository_root.is_some_and(|root| directory == root) {
            break;
        }
        if directory.join(".git").exists() {
            break;
        }
        current = directory.parent();
    }
    None
}

#[cfg(all(test, unix))]
mod bounded_audit_tests {
    use super::*;

    #[test]
    fn audit_never_reads_bytes_beyond_scanner_budget_after_stat_window() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("skill");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            b"---\nname: bounded-audit-team\ndescription: Fixture.\n---\n",
        )
        .unwrap();
        let growing = source.join("A.txt");
        fs::write(&growing, b"small").unwrap();
        crate::shared_skill_source::set_after_named_stat_hook(Box::new(move || {
            fs::write(growing, vec![0_u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        }));

        let error = match audit_local_source(&source, vec!["codex".to_string()], None) {
            Ok(_) => panic!("production Skill walker did not observe the growth hook"),
            Err(error) => error,
        };
        assert!(
            error.contains(&MAX_FILE_BYTES.to_string())
                || error.contains("candidate_read_input_drift"),
            "unexpected bounded production-walker error: {error}"
        );
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("skill");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("SKILL.md"),
            b"---\nname: side-input-team\ndescription: Fixture.\n---\n",
        )
        .unwrap();
        let license = temp.path().join("LICENSE");
        fs::write(&license, b"MIT License\n").unwrap();
        (temp, source, license)
    }

    #[test]
    fn license_rejects_named_file_replaced_by_outside_symlink() {
        let (temp, source, license) = fixture();
        let outside = temp.path().join("outside-license");
        fs::write(&outside, b"MIT License outside sentinel\n").unwrap();
        let license_for_hook = license.clone();
        let outside_for_hook = outside.clone();
        crate::shared_skill_source::set_after_bounded_file_named_stat_hook(Box::new(move || {
            fs::remove_file(&license_for_hook).unwrap();
            std::os::unix::fs::symlink(&outside_for_hook, &license_for_hook).unwrap();
        }));

        assert!(
            audit_local_source(&source, vec![], None).is_err(),
            "license reader followed a symlink installed after discovery"
        );
        assert_eq!(
            fs::read(&outside).unwrap(),
            b"MIT License outside sentinel\n"
        );
    }

    #[test]
    fn license_rejects_oversized_file() {
        let (_temp, source, license) = fixture();
        fs::write(&license, vec![b' '; MAX_SIDE_INPUT_BYTES as usize + 1]).unwrap();

        let error = match audit_local_source(&source, vec![], None) {
            Ok(_) => panic!("oversized license was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("license exceeds 65536 bytes"), "{error}");
    }

    #[test]
    fn license_rejects_growth_after_opened_stat() {
        let (_temp, source, license) = fixture();
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            let mut bytes = b"MIT License\n".to_vec();
            bytes.resize(MAX_SIDE_INPUT_BYTES as usize + 1, b' ');
            fs::write(license, bytes).unwrap();
        }));

        let error = match audit_local_source(&source, vec![], None) {
            Ok(_) => panic!("license growth was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("license exceeds 65536 bytes"), "{error}");
    }

    #[test]
    fn routing_rejects_named_file_replaced_by_outside_symlink() {
        let (temp, source, _license) = fixture();
        let routing = temp.path().join("routing.yaml");
        let outside = temp.path().join("outside-routing.yaml");
        fs::write(&routing, b"summary: inside\n").unwrap();
        fs::write(&outside, b"summary: outside sentinel\n").unwrap();
        let routing_for_hook = routing.clone();
        let outside_for_hook = outside.clone();
        crate::shared_skill_source::set_after_bounded_file_named_stat_hook(Box::new(move || {
            fs::remove_file(&routing_for_hook).unwrap();
            std::os::unix::fs::symlink(&outside_for_hook, &routing_for_hook).unwrap();
        }));

        assert!(
            audit_local_source(&source, vec![], Some(&routing)).is_err(),
            "routing reader followed a symlink installed after the named stat"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"summary: outside sentinel\n");
    }

    #[test]
    fn relative_routing_path_is_bound_to_the_snapshotted_source_root() {
        let (_temp, source, _license) = fixture();
        fs::write(source.join("routing.yaml"), b"summary: source relative\n").unwrap();

        ROUTING_SIDE_OBSERVER_CALLS.with(|calls| calls.set(0));
        let audited = audit_local_source(&source, vec![], Some(Path::new("routing.yaml")))
            .expect("source-relative routing metadata should be resolved from the held source");

        assert_eq!(audited.record.summary, "source relative");
        assert_eq!(
            audited.record.routing_metadata_path.as_deref(),
            Some("routing.yaml")
        );
        assert!(audited
            .read_inputs
            .iter()
            .any(|seal| seal.relative_path == "routing.yaml"));
        ROUTING_SIDE_OBSERVER_CALLS.with(|calls| assert_eq!(calls.get(), 0));
    }

    #[test]
    fn absolute_in_tree_routing_reuses_the_source_snapshot() {
        let (_temp, source, _license) = fixture();
        let routing = source.join("routing.yaml");
        fs::write(&routing, b"summary: absolute in tree\n").unwrap();
        ROUTING_SIDE_OBSERVER_CALLS.with(|calls| calls.set(0));

        let audited = audit_local_source(&source, vec![], Some(&routing)).unwrap();

        assert_eq!(audited.record.summary, "absolute in tree");
        ROUTING_SIDE_OBSERVER_CALLS.with(|calls| assert_eq!(calls.get(), 0));
    }

    #[test]
    fn routing_traversal_is_rejected_lexically() {
        let (_temp, source, _license) = fixture();
        let error =
            audit_local_source(&source, vec![], Some(Path::new("../routing.yaml"))).unwrap_err();
        assert_eq!(error, "routing_metadata_traversal_refused");
    }

    #[test]
    fn missing_in_tree_routing_is_rejected_from_the_snapshot() {
        let (_temp, source, _license) = fixture();
        let error =
            audit_local_source(&source, vec![], Some(Path::new("missing.yaml"))).unwrap_err();
        assert!(error.contains("metadata_file_not_found"), "{error}");
    }

    #[test]
    fn in_tree_routing_symlink_is_rejected_by_the_source_snapshot() {
        let (temp, source, _license) = fixture();
        let outside = temp.path().join("outside.yaml");
        fs::write(&outside, b"summary: outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, source.join("routing.yaml")).unwrap();

        let error =
            audit_local_source(&source, vec![], Some(Path::new("routing.yaml"))).unwrap_err();
        assert!(error.contains("symlink_refused"), "{error}");
        assert_eq!(fs::read(outside).unwrap(), b"summary: outside\n");
    }

    #[test]
    fn external_routing_parent_swap_is_rejected() {
        let (temp, source, _license) = fixture();
        let temp_root = temp.path().to_path_buf();
        let parent = temp_root.join("external");
        fs::create_dir(&parent).unwrap();
        let routing = parent.join("routing.yaml");
        fs::write(&routing, b"summary: held\n").unwrap();
        let moved = temp_root.join("external-held");
        let outside = temp_root.join("outside-external");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("routing.yaml"), b"summary: outside sentinel\n").unwrap();
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            fs::rename(&parent, &moved).unwrap();
            std::os::unix::fs::symlink(&outside, &parent).unwrap();
        }));

        let error = audit_local_source(&source, vec![], Some(&routing)).unwrap_err();
        assert!(
            error.contains("read_input_drift") || error.contains("root_identity_drift"),
            "{error}"
        );
    }

    #[test]
    fn routing_rejects_oversized_file() {
        let (temp, source, _license) = fixture();
        let routing = temp.path().join("routing.yaml");
        fs::write(&routing, vec![b' '; MAX_SIDE_INPUT_BYTES as usize + 1]).unwrap();

        let error = match audit_local_source(&source, vec![], Some(&routing)) {
            Ok(_) => panic!("oversized routing metadata was accepted"),
            Err(error) => error,
        };
        assert!(
            error.contains("routing metadata exceeds 65536 bytes"),
            "{error}"
        );
    }

    #[test]
    fn routing_rejects_growth_after_opened_stat() {
        let (temp, source, _license) = fixture();
        let routing = temp.path().join("routing.yaml");
        fs::write(&routing, b"summary: inside\n").unwrap();
        let routing_for_hook = routing.clone();
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            let mut bytes = b"summary: grown\n#".to_vec();
            bytes.resize(MAX_SIDE_INPUT_BYTES as usize + 1, b' ');
            fs::write(routing_for_hook, bytes).unwrap();
        }));

        let error = match audit_local_source(&source, vec![], Some(&routing)) {
            Ok(_) => panic!("routing metadata growth was accepted"),
            Err(error) => error,
        };
        assert!(
            error.contains("routing metadata exceeds 65536 bytes"),
            "{error}"
        );
    }

    #[test]
    fn side_input_io_has_no_pathname_read_fallback() {
        let source = include_str!("source.rs");
        assert!(!source.contains(concat!("fs::", "read(&license)")));
        assert!(!source.contains(concat!("fs::", "read(&canonical)")));
    }
}
