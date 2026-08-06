use super::model::{
    AdoptionRoutingMetadata, BodyRevision, CatalogReviewStatus, InstalledSkillRecord, RiskFinding,
    SourceSpec, UpdatePolicy,
};
use crate::hash_skill_source;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

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

pub(super) struct AuditedSource {
    pub source_dir: PathBuf,
    pub record: InstalledSkillRecord,
    pub risk_findings: Vec<RiskFinding>,
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

    let mut risk_findings = Vec::new();
    let mut files = 0usize;
    let mut total = 0u64;
    audit_tree(
        &source_dir,
        &source_dir,
        &mut files,
        &mut total,
        &mut risk_findings,
    )?;
    let source_hash = hash_skill_source(&source_dir)?;
    let license = find_license(&source_dir, repository_root);
    let (license_path, license_hash) = if let Some(license) = license {
        let license_bytes = fs::read(&license)
            .map_err(|error| format!("cannot read license {}: {error}", license.display()))?;
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
        routing_metadata_path,
        routing_metadata_hash,
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
    Ok(AuditedSource {
        source_dir: source_dir.clone(),
        record,
        risk_findings,
    })
}

fn load_routing_metadata(
    path: Option<&Path>,
    frontmatter: &mut SkillFrontmatter,
) -> Result<(Option<String>, Option<String>), String> {
    let Some(path) = path else {
        return Ok((None, None));
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
            return Err(format!(
                "cannot inspect routing metadata {}: {error}",
                path.display()
            ))
        }
    };
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
        Some(ags_platform::sha256(&bytes)),
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

pub(super) fn validate_materialized_tree(root: &Path) -> Result<(), String> {
    audit_materialized_tree_with_risks(root).map(|_| ())
}

pub(super) fn audit_materialized_tree_with_risks(root: &Path) -> Result<Vec<RiskFinding>, String> {
    let mut files = 0usize;
    let mut total = 0u64;
    let mut risks = Vec::new();
    audit_tree(root, root, &mut files, &mut total, &mut risks)?;
    Ok(risks)
}

fn audit_tree(
    root: &Path,
    directory: &Path,
    files: &mut usize,
    total: &mut u64,
    risk_findings: &mut Vec<RiskFinding>,
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
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink_refused: {relative}"));
        }
        if metadata.is_dir() {
            audit_tree(root, &path, files, total, risk_findings)?;
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
            let scriptish = is_scriptish(&path, &metadata);
            if scriptish {
                let finding = RiskFinding::acknowledgement(
                    "script_or_executable_content",
                    Some(relative.clone()),
                    "script or executable content is retained but never executed by adoption",
                );
                risk_findings.push(finding);
            }
            if contains_suspected_secret(&path)? {
                risk_findings.push(RiskFinding::acknowledgement(
                    "suspected_sensitive_content",
                    Some(relative),
                    "content matches a sensitive-material heuristic; body bytes are withheld",
                ));
            }
        } else {
            return Err(format!("special_file_refused: {relative}"));
        }
    }
    Ok(())
}

fn is_scriptish(path: &Path, _metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if _metadata.permissions().mode() & 0o111 != 0 {
            return true;
        }
    }
    let name = path
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

fn contains_suspected_secret(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("cannot inspect content {}: {error}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let lower = text.to_ascii_lowercase();
    Ok(text.contains("-----BEGIN ")
        || text.contains("ghp_")
        || text.contains("github_pat_")
        || text.contains("AKIA")
        || lower.contains("private_key")
        || lower.contains("client_secret")
        || lower.contains("access_token"))
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
