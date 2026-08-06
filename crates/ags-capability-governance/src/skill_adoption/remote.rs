use super::model::{
    AdoptionContext, BodyRevision, InstalledSkillRecord, ResolvedSource, RiskFinding, SourceSpec,
};
use super::source::{
    audit_local_source_with_boundary, parse_github_url, validate_materialized_tree,
    validate_source_subdir,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The only process seam used by remote Skill acquisition.  Implementations
/// receive discrete values; no shell command string is ever accepted.
pub trait GitBackend {
    fn resolve_commit(
        &self,
        repository_url: &str,
        requested_ref: Option<&str>,
    ) -> Result<String, String>;

    /// Fetch the immutable object into an isolated repository and return its
    /// tree metadata without populating a worktree.
    fn prepare_checkout(
        &self,
        repository_url: &str,
        resolved_commit: &str,
        destination: &Path,
    ) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
        let _ = (repository_url, resolved_commit, destination);
        Ok(None)
    }

    /// Populate the already-prepared repository with only the selected Skill
    /// subtree and root license files. No second fetch is permitted here.
    fn materialize_selected(
        &self,
        _repository_url: &str,
        resolved_commit: &str,
        destination: &Path,
        _subdir: &str,
        _license_paths: &[String],
    ) -> Result<(), String>;

    /// A backend may inspect its checkout's Git tree for gitlinks and unsafe
    /// paths.  The materialized-tree audit below remains mandatory for every
    /// backend.
    fn validate_checkout(&self, _destination: &Path) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTreeEntryKind {
    Directory,
    Regular,
    Symlink,
    Gitlink,
    Special,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTreeEntry {
    pub path: String,
    pub kind: RemoteTreeEntryKind,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemGitBackend;

#[derive(Debug, Clone)]
pub struct RemoteCandidate {
    pub checkout_root: PathBuf,
    pub skill_dir: PathBuf,
    pub record: InstalledSkillRecord,
    pub resolved_source: ResolvedSource,
}

pub fn acquire_remote_candidate(
    context: &AdoptionContext,
    source: &SourceSpec,
) -> Result<RemoteCandidate, String> {
    acquire_remote_candidate_with_backend(context, source, &SystemGitBackend)
}

pub fn acquire_remote_candidate_with_backend(
    context: &AdoptionContext,
    source: &SourceSpec,
    backend: &dyn GitBackend,
) -> Result<RemoteCandidate, String> {
    let (repository_url, requested_ref, subdir) = validate_remote_source(source)?;
    let resolved_commit = backend.resolve_commit(repository_url, requested_ref.as_deref())?;
    validate_commit(&resolved_commit)?;
    let candidate_identity = ags_platform::sha256(
        &serde_json::to_vec(&(source, &resolved_commit))
            .map_err(|error| format!("cannot serialize candidate identity: {error}"))?,
    );
    let candidate_name = candidate_identity.trim_start_matches("sha256:");
    let candidates_root = context.runtime_home.join("candidates");
    let candidate_root = candidates_root.join(candidate_name);
    let checkout_root = candidate_root.join("checkout");
    ensure_directory_not_symlink(&candidates_root, "candidate store")?;
    ensure_directory_not_symlink(&candidate_root, "candidate")?;
    ensure_directory_not_symlink(&checkout_root, "candidate checkout")?;
    if !checkout_root.exists() {
        fs::create_dir_all(&candidates_root).map_err(|error| {
            format!(
                "cannot create candidate root {}: {error}",
                candidates_root.display()
            )
        })?;
        let stage =
            candidates_root.join(format!(".stage-{}-{}", std::process::id(), candidate_name));
        if fs::symlink_metadata(&stage).is_ok() {
            fs::remove_dir_all(&stage).map_err(|error| {
                format!("cannot clear candidate stage {}: {error}", stage.display())
            })?;
        }
        fs::create_dir_all(&stage).map_err(|error| {
            format!("cannot create candidate stage {}: {error}", stage.display())
        })?;
        let result = (|| -> Result<(), String> {
            let tree_metadata =
                backend.prepare_checkout(repository_url, &resolved_commit, &stage)?;
            let license_paths = tree_metadata
                .as_deref()
                .map(|entries| preflight_selected_tree(entries, &subdir))
                .transpose()?
                .unwrap_or_default();
            backend.materialize_selected(
                repository_url,
                &resolved_commit,
                &stage,
                &subdir,
                &license_paths,
            )?;
            ensure_directory_not_symlink(&stage, "candidate checkout")?;
            backend.validate_checkout(&stage)?;
            remove_git_metadata(&stage)?;
            if let Some(entries) = tree_metadata.as_deref() {
                validate_materialized_tree_metadata(&stage, entries, &subdir)?;
            }
            let selected_stage = stage.join(&subdir);
            validate_materialized_tree(&selected_stage)?;
            validate_materialized_license_files(&stage, &license_paths)?;
            fs::create_dir_all(&candidate_root).map_err(|error| {
                format!(
                    "cannot create candidate directory {}: {error}",
                    candidate_root.display()
                )
            })?;
            fs::rename(&stage, &checkout_root).map_err(|error| {
                format!(
                    "cannot publish candidate checkout {}: {error}",
                    checkout_root.display()
                )
            })?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&stage);
            let _ = fs::remove_dir(&candidates_root);
            return Err(error);
        }
    }
    let skill_dir = checkout_root.join(&subdir);
    ensure_contained_directory(&checkout_root, &skill_dir)?;
    validate_materialized_tree(&skill_dir)?;
    // Fresh candidates already validated the exact tree-derived license set;
    // cached candidates are rechecked against the supported root names.
    validate_materialized_license_files(&checkout_root, &[])?;
    if !skill_dir.join("SKILL.md").is_file() {
        return Err(format!(
            "remote candidate does not contain SKILL.md under {}",
            subdir.if_empty("repository root")
        ));
    }
    let audited =
        audit_local_source_with_boundary(&skill_dir, Vec::new(), None, Some(&checkout_root))?;
    let mut repository_risks = audited.risk_findings.clone();
    repository_risks
        .sort_by(|left, right| left.code.cmp(&right.code).then(left.path.cmp(&right.path)));
    repository_risks.dedup();
    if !repository_risks
        .iter()
        .any(|finding| finding.code == "catalog_unreviewed")
    {
        repository_risks.push(RiskFinding::acknowledgement(
            "catalog_unreviewed",
            None,
            "third-party source has not completed catalog review",
        ));
        repository_risks
            .sort_by(|left, right| left.code.cmp(&right.code).then(left.path.cmp(&right.path)));
    }
    let source_label = source_label(source, &subdir);
    let resolved_source = ResolvedSource {
        source_spec: source.clone(),
        resolved_commit,
        body_hash: audited.record.source_hash.clone(),
        candidate_identity,
        subdir,
    };
    let mut record = audited.record;
    record.source = source_label;
    record.source_spec = source.clone();
    record.resolved_source = Some(resolved_source.clone());
    record.risk_findings = repository_risks;
    record.body_revisions = vec![BodyRevision::from_record(&record)];
    Ok(RemoteCandidate {
        checkout_root,
        skill_dir,
        record,
        resolved_source,
    })
}

fn validate_remote_source(source: &SourceSpec) -> Result<(&str, Option<String>, String), String> {
    let (url, requested_ref, raw_subdir) = match source {
        SourceSpec::GitHub {
            url,
            requested_ref,
            tracking_ref: _,
            subdir,
        } => {
            let parsed = parse_github_url(url, requested_ref.as_deref())?;
            let SourceSpec::GitHub {
                url: canonical,
                requested_ref: parsed_ref,
                tracking_ref: _,
                subdir: parsed_subdir,
            } = parsed
            else {
                return Err(
                    "internal GitHub source parser returned a non-GitHub source".to_string()
                );
            };
            // SourceSpec stores the repository identity, ref and subdirectory as
            // separate fields. Re-parsing the canonical repository URL must
            // therefore produce no embedded path; the separately bound subdir
            // is validated below by validate_source_subdir.
            if canonical != *url || parsed_ref != *requested_ref || parsed_subdir.is_some() {
                return Err(
                    "GitHub source is not canonical or its path binding is invalid".to_string(),
                );
            }
            (url.as_str(), requested_ref.clone(), subdir.clone())
        }
        SourceSpec::Git {
            url,
            requested_ref,
            tracking_ref: _,
            subdir,
        } => {
            if !url.starts_with("file://") {
                return Err("generic Git source is restricted to file:// test seams".to_string());
            }
            (url.as_str(), requested_ref.clone(), subdir.clone())
        }
        SourceSpec::Local { .. } => {
            return Err("local source is not a remote candidate".to_string())
        }
    };
    let subdir = validate_source_subdir(raw_subdir.as_deref())?;
    if let Some(requested_ref) = requested_ref.as_deref() {
        if requested_ref.starts_with('-')
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
            return Err("remote requested_ref is unsafe".to_string());
        }
    }
    Ok((url, requested_ref, subdir))
}

fn preflight_selected_tree(
    entries: &[RemoteTreeEntry],
    subdir: &str,
) -> Result<Vec<String>, String> {
    let selected_prefix = if subdir.is_empty() {
        String::new()
    } else {
        format!("{subdir}/")
    };
    let skill_md = if subdir.is_empty() {
        "SKILL.md".to_string()
    } else {
        format!("{subdir}/SKILL.md")
    };
    let mut selected_files = 0usize;
    let mut selected_bytes = 0u64;
    let mut found_skill = false;
    let mut license_paths = Vec::new();
    for entry in entries {
        validate_tree_metadata_path(&entry.path)?;
        let selected =
            subdir.is_empty() || entry.path == subdir || entry.path.starts_with(&selected_prefix);
        let root_license = is_root_license_path(&entry.path);
        // The candidate contains only the selected Skill subtree plus root
        // license files. Unselected monorepo entries are outside both the
        // materialized body and its trust boundary, so they must not affect
        // this candidate's type or size checks.
        if !selected && !root_license {
            continue;
        }
        if !matches!(
            entry.kind,
            RemoteTreeEntryKind::Directory | RemoteTreeEntryKind::Regular
        ) {
            return Err(format!(
                "{} refused in remote tree: {}",
                match entry.kind {
                    RemoteTreeEntryKind::Symlink => "symlink_refused",
                    RemoteTreeEntryKind::Gitlink => "submodule_refused",
                    RemoteTreeEntryKind::Special => "special_file_refused",
                    RemoteTreeEntryKind::Directory | RemoteTreeEntryKind::Regular => {
                        "remote_tree_type_refused"
                    }
                },
                entry.path
            ));
        }
        if entry.kind == RemoteTreeEntryKind::Directory {
            continue;
        }
        if entry.path == skill_md {
            found_skill = true;
        }
        if root_license {
            license_paths.push(entry.path.clone());
        }
        if entry.size > super::source::MAX_FILE_BYTES {
            return Err(format!(
                "skill source file exceeds {} bytes: {}",
                super::source::MAX_FILE_BYTES,
                entry.path
            ));
        }
        if selected {
            selected_files = selected_files.saturating_add(1);
            selected_bytes = selected_bytes.saturating_add(entry.size);
            if selected_files > super::source::MAX_FILES {
                return Err(format!(
                    "selected Skill subtree exceeds {} files",
                    super::source::MAX_FILES
                ));
            }
            if selected_bytes > super::source::MAX_TOTAL_BYTES {
                return Err(format!(
                    "selected Skill subtree exceeds {} total bytes",
                    super::source::MAX_TOTAL_BYTES
                ));
            }
        }
    }
    if !found_skill {
        return Err(format!(
            "remote tree metadata does not contain a regular SKILL.md under {}",
            if subdir.is_empty() {
                "repository root"
            } else {
                subdir
            }
        ));
    }
    license_paths.sort();
    license_paths.dedup();
    Ok(license_paths)
}

fn validate_tree_metadata_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || Path::new(path).is_absolute()
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("path_traversal_refused: {path}"));
    }
    Ok(())
}

fn is_root_license_path(path: &str) -> bool {
    matches!(path, "LICENSE" | "LICENSE.md" | "LICENSE.txt" | "COPYING")
}

fn source_label(source: &SourceSpec, subdir: &str) -> String {
    match source {
        SourceSpec::GitHub { url, .. } | SourceSpec::Git { url, .. } => {
            if subdir.is_empty() {
                url.clone()
            } else {
                format!("{url}#/{subdir}")
            }
        }
        SourceSpec::Local { path } => path.clone(),
    }
}

fn ensure_contained_directory(root: &Path, child: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(child).map_err(|error| {
        format!(
            "cannot inspect remote candidate path {}: {error}",
            child.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!("symlink_refused: {}", child.display()));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "remote candidate path is not a directory: {}",
            child.display()
        ));
    }
    let root = root.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize candidate root {}: {error}",
            root.display()
        )
    })?;
    let child = child.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize candidate path {}: {error}",
            child.display()
        )
    })?;
    if !child.starts_with(&root) {
        return Err(format!(
            "remote candidate path escapes checkout root: {}",
            child.display()
        ));
    }
    Ok(())
}

fn validate_materialized_license_files(
    checkout_root: &Path,
    license_paths: &[String],
) -> Result<(), String> {
    let required = !license_paths.is_empty();
    let paths = if license_paths.is_empty() {
        ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"]
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>()
    } else {
        license_paths.to_vec()
    };
    for relative in paths {
        let path = checkout_root.join(&relative);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            if required {
                return Err(format!("remote selected license is missing: {relative}"));
            }
            continue;
        };
        if metadata.file_type().is_symlink() {
            return Err(format!("symlink_refused: {relative}"));
        }
        if !metadata.is_file() {
            return Err(format!("special_file_refused: {relative}"));
        }
        if metadata.len() > super::source::MAX_FILE_BYTES {
            return Err(format!(
                "skill source file exceeds {} bytes: {relative}",
                super::source::MAX_FILE_BYTES
            ));
        }
    }
    Ok(())
}

fn validate_materialized_tree_metadata(
    checkout_root: &Path,
    entries: &[RemoteTreeEntry],
    subdir: &str,
) -> Result<(), String> {
    let selected_prefix = if subdir.is_empty() {
        String::new()
    } else {
        format!("{subdir}/")
    };
    for entry in entries {
        let selected =
            subdir.is_empty() || entry.path == subdir || entry.path.starts_with(&selected_prefix);
        if !selected && !is_root_license_path(&entry.path) {
            continue;
        }
        let path = checkout_root.join(&entry.path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "selected remote tree entry is missing {}: {error}",
                entry.path
            )
        })?;
        match entry.kind {
            RemoteTreeEntryKind::Directory => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(format!("symlink_refused: {}", entry.path));
                }
            }
            RemoteTreeEntryKind::Regular => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(format!("special_file_refused: {}", entry.path));
                }
                if metadata.len() > super::source::MAX_FILE_BYTES {
                    return Err(format!(
                        "skill source file exceeds {} bytes: {}",
                        super::source::MAX_FILE_BYTES,
                        entry.path
                    ));
                }
            }
            RemoteTreeEntryKind::Symlink => {
                return Err(format!("symlink_refused: {}", entry.path));
            }
            RemoteTreeEntryKind::Gitlink => {
                return Err(format!("submodule_refused: {}", entry.path));
            }
            RemoteTreeEntryKind::Special => {
                return Err(format!("special_file_refused: {}", entry.path));
            }
        }
    }
    Ok(())
}

fn ensure_directory_not_symlink(path: &Path, label: &str) -> Result<(), String> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() {
        return Err(format!("symlink_refused: {label} {}", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("special_file_refused: {label} {}", path.display()));
    }
    Ok(())
}

fn remove_git_metadata(root: &Path) -> Result<(), String> {
    let git = root.join(".git");
    let Ok(metadata) = fs::symlink_metadata(&git) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() {
        return Err("symlink_refused: .git".to_string());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(&git)
            .map_err(|error| format!("cannot remove isolated Git metadata: {error}"))
    } else if metadata.is_file() {
        fs::remove_file(&git)
            .map_err(|error| format!("cannot remove isolated Git metadata: {error}"))
    } else {
        Err("special_file_refused: .git".to_string())
    }
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if !ags_platform::is_git_commit(commit) {
        return Err("remote Git did not resolve a full immutable commit".to_string());
    }
    Ok(())
}

impl GitBackend for SystemGitBackend {
    fn resolve_commit(
        &self,
        repository_url: &str,
        requested_ref: Option<&str>,
    ) -> Result<String, String> {
        let output = if let Some(requested_ref) = requested_ref {
            // `ls-remote --refs` only resolves names advertised by the
            // remote.  An arbitrary full object id is a valid immutable pin
            // even when it is not itself a ref, so preserve it and let the
            // subsequent isolated fetch/checkout prove reachability.
            if ags_platform::is_git_commit(requested_ref) {
                return Ok(requested_ref.to_ascii_lowercase());
            }
            let mut output = run_git(
                &[
                    "ls-remote".to_string(),
                    "--refs".to_string(),
                    repository_url.to_string(),
                    requested_ref.to_string(),
                ],
                None,
            )?;
            if output.trim().is_empty() && !requested_ref.starts_with("refs/") {
                output = run_git(
                    &[
                        "ls-remote".to_string(),
                        "--refs".to_string(),
                        repository_url.to_string(),
                        format!("refs/heads/{requested_ref}"),
                    ],
                    None,
                )?;
            }
            output
        } else {
            run_git(
                &[
                    "ls-remote".to_string(),
                    repository_url.to_string(),
                    "HEAD".to_string(),
                ],
                None,
            )?
        };
        let commits = output
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .filter(|value| value.len() >= 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let Some(commit) = commits.first() else {
            return Err("remote Git ref did not resolve to a commit".to_string());
        };
        if commits.iter().any(|candidate| candidate != commit) {
            return Err("remote Git ref resolved to multiple commits".to_string());
        }
        Ok(commit.clone())
    }

    fn prepare_checkout(
        &self,
        repository_url: &str,
        resolved_commit: &str,
        destination: &Path,
    ) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
        init_git_repository(repository_url, destination)?;
        fetch_git_commit(destination, resolved_commit)?;
        let output = run_git(
            &[
                "-C".to_string(),
                destination.to_string_lossy().into_owned(),
                "ls-tree".to_string(),
                "-r".to_string(),
                "-l".to_string(),
                "-z".to_string(),
                resolved_commit.to_string(),
            ],
            None,
        )?;
        parse_tree_metadata(&output)
    }

    fn materialize_selected(
        &self,
        _repository_url: &str,
        resolved_commit: &str,
        destination: &Path,
        subdir: &str,
        license_paths: &[String],
    ) -> Result<(), String> {
        run_git(
            &[
                "-C".to_string(),
                destination.to_string_lossy().into_owned(),
                "sparse-checkout".to_string(),
                "init".to_string(),
                "--no-cone".to_string(),
            ],
            None,
        )?;
        let mut patterns = Vec::new();
        if subdir.is_empty() {
            patterns.push("/SKILL.md".to_string());
        } else {
            patterns.push(format!("/{subdir}/**"));
        }
        patterns.extend(license_paths.iter().map(|path| format!("/{path}")));
        let mut sparse_args = vec![
            "-C".to_string(),
            destination.to_string_lossy().into_owned(),
            "sparse-checkout".to_string(),
            "set".to_string(),
            "--no-cone".to_string(),
            "--".to_string(),
        ];
        sparse_args.extend(patterns);
        run_git(&sparse_args, None)?;
        run_git(
            &[
                "-C".to_string(),
                destination.to_string_lossy().into_owned(),
                "checkout".to_string(),
                "--detach".to_string(),
                "--force".to_string(),
                resolved_commit.to_string(),
            ],
            None,
        )?;
        Ok(())
    }

    fn validate_checkout(&self, destination: &Path) -> Result<(), String> {
        let output = run_git(
            &[
                "-C".to_string(),
                destination.to_string_lossy().into_owned(),
                "ls-tree".to_string(),
                "-r".to_string(),
                "-z".to_string(),
                "HEAD".to_string(),
            ],
            None,
        )?;
        for entry in output.as_bytes().split(|byte| *byte == 0) {
            if entry.is_empty() {
                continue;
            }
            let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
                return Err("remote Git tree entry is malformed".to_string());
            };
            let header = &entry[..tab];
            let path = &entry[tab + 1..];
            let mode = header
                .split(|byte| *byte == b' ')
                .next()
                .unwrap_or_default();
            if mode == b"160000" {
                return Err("submodule_refused: Git tree contains a gitlink".to_string());
            }
            let path = std::str::from_utf8(path)
                .map_err(|_| "remote Git tree path is not UTF-8".to_string())?;
            if Path::new(path).is_absolute()
                || path.contains('\\')
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(format!("path_traversal_refused: {path}"));
            }
        }
        Ok(())
    }
}

fn init_git_repository(repository_url: &str, destination: &Path) -> Result<(), String> {
    run_git(
        &[
            "init".to_string(),
            "--quiet".to_string(),
            destination.to_string_lossy().into_owned(),
        ],
        None,
    )?;
    run_git(
        &[
            "-C".to_string(),
            destination.to_string_lossy().into_owned(),
            "remote".to_string(),
            "add".to_string(),
            "origin".to_string(),
            repository_url.to_string(),
        ],
        None,
    )?;
    Ok(())
}

fn fetch_git_commit(destination: &Path, resolved_commit: &str) -> Result<(), String> {
    run_git(
        &[
            "-C".to_string(),
            destination.to_string_lossy().into_owned(),
            "fetch".to_string(),
            "--filter=blob:none".to_string(),
            "--no-tags".to_string(),
            "--no-recurse-submodules".to_string(),
            "--depth=1".to_string(),
            "origin".to_string(),
            resolved_commit.to_string(),
        ],
        None,
    )?;
    Ok(())
}

fn parse_tree_metadata(output: &str) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
    let mut entries = Vec::new();
    for entry in output.as_bytes().split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            return Err("remote Git tree entry is malformed".to_string());
        };
        let header = std::str::from_utf8(&entry[..tab])
            .map_err(|_| "remote Git tree header is not UTF-8".to_string())?;
        let path = std::str::from_utf8(&entry[tab + 1..])
            .map_err(|_| "remote Git tree path is not UTF-8".to_string())?;
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 3 {
            return Err("remote Git tree header is incomplete".to_string());
        }
        let kind = match fields[0] {
            "100644" | "100755" => RemoteTreeEntryKind::Regular,
            "120000" => RemoteTreeEntryKind::Symlink,
            "160000" => RemoteTreeEntryKind::Gitlink,
            _ => RemoteTreeEntryKind::Special,
        };
        let size = fields
            .get(3)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        entries.push(RemoteTreeEntry {
            path: path.to_string(),
            kind,
            size,
        });
    }
    Ok(Some(entries))
}

fn run_git(args: &[String], current_dir: Option<&Path>) -> Result<String, String> {
    let no_hooks_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg(format!("core.hooksPath={no_hooks_path}"));
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let output = command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", no_hooks_path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_ASKPASS")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_PROXY_COMMAND")
        .output()
        .map_err(|error| format!("cannot start system git: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "system git failed ({}): {}",
            output.status,
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

trait EmptyText {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str;
}

impl EmptyText for str {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}
