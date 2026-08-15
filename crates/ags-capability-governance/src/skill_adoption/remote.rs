#![cfg_attr(not(unix), allow(dead_code))]

use super::model::{
    AdoptionContext, InstalledSkillRecord, MaterializedBodyNode, ResolvedSource, SourceSpec,
};
#[cfg(unix)]
use super::model::{BodyRevision, RiskFinding};
#[cfg(unix)]
use super::source::audit_remote_source_snapshots;
use super::source::{parse_github_url, validate_source_subdir};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Stat};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

const CANDIDATE_MANIFEST_SCHEMA: &str = "ags://schema/contract/v2/remote-candidate";
#[cfg_attr(not(unix), allow(dead_code))]
const CANDIDATE_MANIFEST_FILE: &str = "candidate-manifest.json";
#[cfg_attr(not(unix), allow(dead_code))]
const MAX_CANDIDATE_MANIFEST_BYTES: u64 = 64 * 1024;
#[cfg(unix)]
const MAX_GIT_SCAN_DIRECTORIES: usize = 512;
#[cfg(unix)]
const MAX_GIT_SCAN_ENTRIES: usize = 1024;
#[cfg(unix)]
const MAX_GIT_METADATA_RESULTS: usize = 64;
#[cfg(unix)]
const MAX_GIT_SCAN_NAME_BYTES: usize = 255;
#[cfg(unix)]
const MAX_GIT_CLEANUP_ENTRIES: usize = 16 * 1024;
const MAX_GIT_PROCESS_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const GIT_PROCESS_TIMEOUT: Duration = Duration::from_secs(60);
static CANDIDATE_STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(all(test, unix))]
thread_local! {
    static AFTER_GIT_METADATA_NAMED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_CANDIDATE_ROOT_PATH_CHECK_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_FRESH_STAGE_CREATE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_CACHED_MANIFEST_READ_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_HELD_STAGE_REVALIDATE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_CANDIDATE_MANIFEST_WRITE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

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
        destination: &HeldCheckout,
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
        destination: &HeldCheckout,
        _subdir: &str,
        _license_paths: &[String],
    ) -> Result<(), String>;

    /// A backend may inspect its checkout's Git tree for gitlinks and unsafe
    /// paths.  The materialized-tree audit below remains mandatory for every
    /// backend.
    fn validate_checkout(&self, _destination: &HeldCheckout) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(unix)]
pub struct HeldCheckout {
    root: crate::shared_skill_source::DescriptorRoot,
}

#[cfg(unix)]
impl HeldCheckout {
    pub fn create_dir_all(&self, relative: &Path) -> Result<(), String> {
        if relative.as_os_str().is_empty() {
            return Ok(());
        }
        self.root.create_relative_directory(
            relative,
            Mode::from_raw_mode(0o700),
            "backend checkout",
        )?;
        Ok(())
    }

    pub fn write(&self, relative: &Path, bytes: &[u8]) -> Result<(), String> {
        self.root.write_relative_file(
            relative,
            bytes,
            Mode::from_raw_mode(0o600),
            "backend checkout",
        )
    }

    pub fn copy_from(&self, source: &Path) -> Result<(), String> {
        copy_into_held_checkout(source, self, Path::new(""))
    }

    pub fn symlink(&self, relative: &Path, target: &Path) -> Result<(), String> {
        self.root
            .create_relative_symlink(relative, target, "backend checkout")
    }
}

#[cfg(not(unix))]
pub struct HeldCheckout;

#[cfg(not(unix))]
impl HeldCheckout {
    pub fn create_dir_all(&self, _relative: &Path) -> Result<(), String> {
        Err("descriptor_semantics_unavailable_for_held_checkout".to_string())
    }

    pub fn write(&self, _relative: &Path, _bytes: &[u8]) -> Result<(), String> {
        Err("descriptor_semantics_unavailable_for_held_checkout".to_string())
    }

    pub fn copy_from(&self, _source: &Path) -> Result<(), String> {
        Err("descriptor_semantics_unavailable_for_held_checkout".to_string())
    }

    pub fn symlink(&self, _relative: &Path, _target: &Path) -> Result<(), String> {
        Err("descriptor_semantics_unavailable_for_held_checkout".to_string())
    }
}

#[cfg(unix)]
fn copy_into_held_checkout(
    source: &Path,
    destination: &HeldCheckout,
    relative: &Path,
) -> Result<(), String> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let child_relative = relative.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            destination.create_dir_all(&child_relative)?;
            copy_into_held_checkout(&entry.path(), destination, &child_relative)?;
        } else if file_type.is_file() {
            destination.write(
                &child_relative,
                &fs::read(entry.path()).map_err(|error| error.to_string())?,
            )?;
        } else if file_type.is_symlink() {
            destination.symlink(
                &child_relative,
                &fs::read_link(entry.path()).map_err(|error| error.to_string())?,
            )?;
        } else {
            return Err("fixture source contains a special file".to_string());
        }
    }
    Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateManifest {
    schema_version: String,
    candidate_identity: String,
    repository_url: String,
    resolved_commit: String,
    subdir: String,
    body_hash: String,
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
    let resolved_commit = resolved_commit.to_ascii_lowercase();
    let candidate_identity = ags_platform::sha256(
        &serde_json::to_vec(&(
            CANDIDATE_MANIFEST_SCHEMA,
            repository_url,
            &resolved_commit,
            &subdir,
        ))
        .map_err(|error| format!("cannot serialize candidate identity: {error}"))?,
    );
    let candidate_name = candidate_identity.trim_start_matches("sha256:");
    let candidates_root = context.candidate_home.join("candidates");
    let candidate_root = candidates_root.join(candidate_name);
    let checkout_root = candidate_root.join("checkout");
    #[cfg(unix)]
    {
        let held_candidates = open_candidate_store(&context.candidate_home)?;
        let expected_manifest = CandidateManifest {
            schema_version: CANDIDATE_MANIFEST_SCHEMA.to_string(),
            candidate_identity: candidate_identity.clone(),
            repository_url: repository_url.to_string(),
            resolved_commit: resolved_commit.clone(),
            subdir: subdir.clone(),
            body_hash: String::new(),
        };
        if let Ok(held_candidate) =
            held_candidates.open_relative_directory(Path::new(candidate_name), "candidate root")
        {
            if validate_cached_candidate_at(&held_candidate, &expected_manifest).is_err() {
                quarantine_candidate(&candidates_root, &candidate_root, candidate_name)?;
            }
        }
        if held_candidates
            .open_relative_directory(Path::new(candidate_name), "candidate root")
            .is_err()
        {
            materialize_candidate(
                &held_candidates,
                candidate_name,
                repository_url,
                &resolved_commit,
                &subdir,
                &expected_manifest,
                backend,
            )?;
        }
        let held_candidate =
            held_candidates.open_relative_directory(Path::new(candidate_name), "candidate root")?;
        validate_cached_candidate_at(&held_candidate, &expected_manifest)?;
        let skill_dir = checkout_root.join(&subdir);
        let checkout_snapshot =
            super::materialize::snapshot_skill_source_at(&held_candidate, Path::new("checkout"))?;
        let skill_snapshot = super::materialize::snapshot_skill_source_at(
            &held_candidate,
            &Path::new("checkout").join(&subdir),
        )?;
        let audited = audit_remote_source_snapshots(
            skill_dir.clone(),
            &checkout_root,
            &subdir,
            Vec::new(),
            skill_snapshot,
            &checkout_snapshot,
        )?;
        held_candidate
            .revalidate("candidate root")
            .map_err(|_| "candidate_root_identity_drift".to_string())?;
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
    #[cfg(not(unix))]
    {
        let _ = (
            context,
            source,
            backend,
            repository_url,
            requested_ref,
            subdir,
            &resolved_commit,
            &candidate_identity,
            candidate_name,
            candidates_root,
            candidate_root,
            checkout_root,
        );
        Err("descriptor_semantics_unavailable_for_candidate_store".to_string())
    }
}

#[cfg(unix)]
fn open_candidate_store(
    runtime_home: &Path,
) -> Result<crate::shared_skill_source::DescriptorRoot, String> {
    let runtime_parent = runtime_home
        .parent()
        .ok_or_else(|| "runtime home has no authorized parent".to_string())?;
    let runtime_name = runtime_home
        .file_name()
        .ok_or_else(|| "runtime home has no directory name".to_string())?;
    let held_parent = crate::shared_skill_source::DescriptorRoot::open_absolute(
        runtime_parent,
        "candidate store authority",
    )?;
    held_parent.create_relative_directory(
        &PathBuf::from(runtime_name).join("candidates"),
        Mode::from_raw_mode(0o700),
        "candidate store",
    )
}

#[allow(clippy::too_many_arguments)]
#[allow(unreachable_code)]
#[cfg(unix)]
fn materialize_candidate(
    candidates_root: &crate::shared_skill_source::DescriptorRoot,
    candidate_name: &str,
    repository_url: &str,
    resolved_commit: &str,
    subdir: &str,
    expected_manifest: &CandidateManifest,
    backend: &dyn GitBackend,
) -> Result<(), String> {
    let stage_name = format!(
        ".stage-{}-{}-{candidate_name}",
        std::process::id(),
        unique_candidate_suffix()
    );
    let held_stage = candidates_root.create_relative_directory(
        Path::new(&stage_name),
        Mode::from_raw_mode(0o700),
        "candidate stage root",
    )?;
    let stage_root = held_stage.path().to_path_buf();
    let candidate_root = candidates_root.path().join(candidate_name);
    #[cfg(test)]
    AFTER_FRESH_STAGE_CREATE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    held_stage
        .revalidate("candidate stage root")
        .map_err(|_| "candidate_stage_root_identity_drift".to_string())?;
    #[cfg(test)]
    AFTER_HELD_STAGE_REVALIDATE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    let held_checkout = HeldCheckout {
        root: held_stage.create_relative_directory(
            Path::new("checkout"),
            Mode::from_raw_mode(0o700),
            "candidate checkout",
        )?,
    };
    let result = (|| -> Result<(), String> {
        let tree_metadata =
            backend.prepare_checkout(repository_url, resolved_commit, &held_checkout)?;
        let license_paths = tree_metadata
            .as_deref()
            .map(|entries| preflight_selected_tree(entries, subdir))
            .transpose()?
            .unwrap_or_default();
        backend.materialize_selected(
            repository_url,
            resolved_commit,
            &held_checkout,
            subdir,
            &license_paths,
        )?;
        backend.validate_checkout(&held_checkout)?;
        for path in find_git_metadata_held(&held_checkout.root)? {
            held_checkout
                .root
                .remove_relative_tree(&path, "Git metadata")?;
        }
        let checkout_snapshot =
            super::materialize::snapshot_skill_source_at(&held_checkout.root, Path::new(""))?;
        if let Some(entries) = tree_metadata.as_deref() {
            validate_snapshot_tree_metadata(&checkout_snapshot, entries, subdir)?;
        }
        validate_snapshot_license_files(&checkout_snapshot, &license_paths)?;
        let selected_snapshot =
            super::materialize::snapshot_skill_source_at(&held_checkout.root, Path::new(subdir))?;
        let mut manifest = expected_manifest.clone();
        manifest.body_hash = selected_snapshot.source_hash;
        #[cfg(unix)]
        {
            write_candidate_manifest_at(&held_stage, &manifest)?;
            #[cfg(test)]
            AFTER_CANDIDATE_MANIFEST_WRITE_HOOK.with(|slot| {
                if let Some(hook) = slot.borrow_mut().take() {
                    hook();
                }
            });
        }
        #[cfg(not(unix))]
        return Err("descriptor_semantics_unavailable_for_candidate_manifest_write".to_string());
        held_stage
            .revalidate("candidate stage root")
            .map_err(|_| "candidate_stage_root_identity_drift".to_string())?;
        candidates_root
            .revalidate("candidate store")
            .map_err(|_| "candidate_store_identity_drift".to_string())?;
        rustix::fs::renameat(
            candidates_root.descriptor(),
            Path::new(&stage_name),
            candidates_root.descriptor(),
            Path::new(candidate_name),
        )
        .map_err(|error| {
            format!(
                "cannot publish sealed candidate {}: {error}",
                candidate_root.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage_root);
        // Preserve the fail-closed contract: an acquisition that
        // never publishes or quarantines a candidate must not leave an empty
        // cache directory behind.
        let _ = fs::remove_dir(candidates_root.path());
    }
    result
}

#[allow(unreachable_code)]
#[cfg(all(test, unix))]
fn validate_cached_candidate(
    candidate_root: &Path,
    expected: &CandidateManifest,
) -> Result<(), String> {
    let parent = candidate_root
        .parent()
        .ok_or_else(|| "candidate root has no parent".to_string())?;
    let name = candidate_root
        .file_name()
        .ok_or_else(|| "candidate root has no name".to_string())?;
    let held_parent =
        crate::shared_skill_source::DescriptorRoot::open_absolute(parent, "candidate store")?;
    let held_candidate =
        held_parent.open_relative_directory(Path::new(name), "candidate root named check")?;
    #[cfg(test)]
    AFTER_CANDIDATE_ROOT_PATH_CHECK_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    held_candidate
        .revalidate("candidate root")
        .map_err(|_| "candidate_root_identity_drift".to_string())?;
    validate_cached_candidate_at(&held_candidate, expected)
}

#[allow(unreachable_code)]
#[cfg(unix)]
fn validate_cached_candidate_at(
    held_candidate: &crate::shared_skill_source::DescriptorRoot,
    expected: &CandidateManifest,
) -> Result<(), String> {
    #[cfg(unix)]
    let manifest = read_candidate_manifest_at(held_candidate)?;
    #[cfg(test)]
    AFTER_CACHED_MANIFEST_READ_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    #[cfg(not(unix))]
    return Err("descriptor_semantics_unavailable_for_candidate_manifest_read".to_string());
    #[cfg(not(unix))]
    let manifest = expected.clone();
    if manifest.schema_version != expected.schema_version
        || manifest.candidate_identity != expected.candidate_identity
        || manifest.repository_url != expected.repository_url
        || manifest.resolved_commit != expected.resolved_commit
        || manifest.subdir != expected.subdir
        || manifest.body_hash.is_empty()
    {
        return Err("candidate_manifest_identity_mismatch".to_string());
    }
    let checkout_snapshot =
        super::materialize::snapshot_skill_source_at(held_candidate, Path::new("checkout"))?;
    if checkout_snapshot
        .nodes
        .iter()
        .map(materialized_node_path)
        .any(|path| path.split('/').any(|component| component == ".git"))
    {
        return Err("isolated Git metadata remains after cleanup".to_string());
    }
    let skill_relative = Path::new("checkout").join(&manifest.subdir);
    let skill_snapshot =
        super::materialize::snapshot_skill_source_at(held_candidate, &skill_relative)?;
    if skill_snapshot.source_hash != manifest.body_hash {
        return Err("candidate_manifest_body_hash_mismatch".to_string());
    }
    held_candidate
        .revalidate("candidate root")
        .map_err(|_| "candidate_root_identity_drift".to_string())?;
    Ok(())
}

#[cfg(all(test, unix))]
fn read_candidate_manifest(candidate_root: &Path) -> Result<CandidateManifest, String> {
    let held_candidate = crate::shared_skill_source::DescriptorRoot::open_absolute(
        candidate_root,
        "candidate root",
    )?;
    read_candidate_manifest_at(&held_candidate)
}

#[cfg(unix)]
fn read_candidate_manifest_at(
    candidate_root: &crate::shared_skill_source::DescriptorRoot,
) -> Result<CandidateManifest, String> {
    let path = candidate_root.path().join(CANDIDATE_MANIFEST_FILE);
    let observed = crate::shared_skill_source::observe_bounded_regular_file_at(
        candidate_root,
        Path::new(CANDIDATE_MANIFEST_FILE),
        MAX_CANDIDATE_MANIFEST_BYTES,
        "candidate manifest",
    )
    .map_err(|error| {
        if error.contains("exceeds 65536 bytes") {
            "candidate_manifest_too_large".to_string()
        } else if error.contains("must be a regular file") {
            "candidate_manifest_not_regular_file".to_string()
        } else {
            error
        }
    })?;
    serde_json::from_slice(&observed.bytes).map_err(|error| {
        format!(
            "cannot parse candidate manifest {}: {error}",
            path.display()
        )
    })
}

#[cfg(all(test, unix))]
fn write_candidate_manifest(
    candidate_root: &Path,
    manifest: &CandidateManifest,
) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = (candidate_root, manifest);
        return Err("descriptor_semantics_unavailable_for_candidate_manifest_write".to_string());
    }

    #[cfg(unix)]
    {
        let held_candidate = crate::shared_skill_source::DescriptorRoot::open_absolute(
            candidate_root,
            "candidate stage root",
        )?;
        write_candidate_manifest_at(&held_candidate, manifest)
    }
}

#[cfg(all(test, unix))]
thread_local! {
    static AFTER_CANDIDATE_MANIFEST_ROOT_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
fn write_candidate_manifest_at(
    candidate_root: &crate::shared_skill_source::DescriptorRoot,
    manifest: &CandidateManifest,
) -> Result<(), String> {
    let path = candidate_root.path().join(CANDIDATE_MANIFEST_FILE);
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("cannot serialize candidate manifest: {error}"))?;
    if bytes.len() as u64 > MAX_CANDIDATE_MANIFEST_BYTES {
        return Err("candidate_manifest_too_large".to_string());
    }
    let root_fd = candidate_root.descriptor();
    let root_opened_before = rustix::fs::fstat(root_fd)
        .map_err(|error| format!("cannot stat held candidate stage root: {error}"))?;
    #[cfg(test)]
    AFTER_CANDIDATE_MANIFEST_ROOT_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
    let file_fd = rustix::fs::openat(
        root_fd,
        CANDIDATE_MANIFEST_FILE,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(|error| {
        format!(
            "cannot create candidate manifest {}: {error}",
            path.display()
        )
    })?;
    let mut file = fs::File::from(file_fd);
    file.write_all(&bytes).map_err(|error| {
        format!(
            "cannot write candidate manifest {}: {error}",
            path.display()
        )
    })?;
    file.sync_all()
        .map_err(|error| format!("cannot sync candidate manifest {}: {error}", path.display()))?;
    let opened_after = rustix::fs::fstat(&file)
        .map_err(|error| format!("cannot stat written candidate manifest: {error}"))?;
    let named_after =
        rustix::fs::statat(root_fd, CANDIDATE_MANIFEST_FILE, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot revalidate candidate manifest path: {error}"))?;
    let root_opened_after = rustix::fs::fstat(root_fd)
        .map_err(|error| format!("cannot revalidate held candidate stage root: {error}"))?;
    if FileType::from_raw_mode(opened_after.st_mode) != FileType::RegularFile
        || git_stat_binding(&opened_after) != git_stat_binding(&named_after)
        || u64::try_from(opened_after.st_size).ok() != Some(bytes.len() as u64)
        || (u32::from(opened_after.st_mode) & 0o7777) != 0o600
        || git_stat_identity(&root_opened_before) != git_stat_identity(&root_opened_after)
    {
        return Err("candidate_manifest_write_identity_drift".to_string());
    }
    candidate_root.revalidate("candidate stage root")?;
    Ok(())
}

fn quarantine_candidate(
    candidates_root: &Path,
    candidate_root: &Path,
    candidate_name: &str,
) -> Result<(), String> {
    let quarantine_root = candidates_root.join("quarantine");
    ensure_directory_not_symlink(&quarantine_root, "candidate quarantine")?;
    fs::create_dir_all(&quarantine_root).map_err(|error| {
        format!(
            "cannot create candidate quarantine {}: {error}",
            quarantine_root.display()
        )
    })?;
    let quarantined = quarantine_root.join(format!(
        "{candidate_name}-{}-{}",
        std::process::id(),
        unique_candidate_suffix()
    ));
    fs::rename(candidate_root, &quarantined).map_err(|error| {
        format!(
            "cannot quarantine invalid candidate {}: {error}",
            candidate_root.display()
        )
    })?;
    let checkout = quarantined.join("checkout");
    if fs::symlink_metadata(&checkout).is_ok() {
        remove_git_metadata(&checkout)?;
        ensure_no_git_metadata(&checkout)?;
    }
    Ok(())
}

fn unique_candidate_suffix() -> String {
    let sequence = CANDIDATE_STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{sequence:x}")
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

fn materialized_node_path(node: &MaterializedBodyNode) -> &str {
    match node {
        MaterializedBodyNode::Directory { relative_path, .. }
        | MaterializedBodyNode::RegularFile { relative_path, .. } => relative_path,
    }
}

fn validate_snapshot_tree_metadata(
    snapshot: &super::materialize::SkillSourceSnapshot,
    entries: &[RemoteTreeEntry],
    subdir: &str,
) -> Result<(), String> {
    let selected_prefix = (!subdir.is_empty()).then(|| format!("{subdir}/"));
    for entry in entries {
        if !(subdir.is_empty()
            || entry.path == subdir
            || selected_prefix
                .as_deref()
                .is_some_and(|prefix| entry.path.starts_with(prefix))
            || is_root_license_path(&entry.path))
        {
            continue;
        }
        let node = snapshot
            .nodes
            .iter()
            .find(|node| materialized_node_path(node) == entry.path)
            .ok_or_else(|| format!("selected remote tree entry is missing {}", entry.path))?;
        match (entry.kind, node) {
            (RemoteTreeEntryKind::Directory, MaterializedBodyNode::Directory { .. }) => {}
            (RemoteTreeEntryKind::Regular, MaterializedBodyNode::RegularFile { bytes, .. })
                if bytes.len() as u64 <= super::source::MAX_FILE_BYTES => {}
            (RemoteTreeEntryKind::Symlink, _) => {
                return Err(format!("symlink_refused: {}", entry.path))
            }
            (RemoteTreeEntryKind::Gitlink, _) => {
                return Err(format!("submodule_refused: {}", entry.path))
            }
            _ => return Err(format!("special_file_refused: {}", entry.path)),
        }
    }
    Ok(())
}

fn validate_snapshot_license_files(
    snapshot: &super::materialize::SkillSourceSnapshot,
    license_paths: &[String],
) -> Result<(), String> {
    for path in license_paths {
        let Some(MaterializedBodyNode::RegularFile { bytes, .. }) = snapshot
            .nodes
            .iter()
            .find(|node| materialized_node_path(node) == path)
        else {
            return Err(format!("remote selected license is missing: {path}"));
        };
        if bytes.len() as u64 > super::source::MAX_FILE_BYTES {
            return Err(format!("skill source file exceeds byte budget: {path}"));
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

#[cfg(unix)]
fn remove_git_metadata(root: &Path) -> Result<(), String> {
    const MAX_PASSES: usize = 4;
    for _ in 0..MAX_PASSES {
        let git_paths = find_git_metadata(root)?;
        if git_paths.is_empty() {
            return Ok(());
        }
        for git_path in git_paths {
            remove_git_metadata_path(root, &git_path)?;
        }
        std::thread::yield_now();
    }
    ensure_no_git_metadata(root)
}

#[cfg(not(unix))]
fn remove_git_metadata(_root: &Path) -> Result<(), String> {
    Err("descriptor_semantics_unavailable_for_git_metadata_cleanup".to_string())
}

#[cfg(unix)]
fn ensure_no_git_metadata(root: &Path) -> Result<(), String> {
    let remaining = find_git_metadata(root)?;
    if remaining.is_empty() {
        return Ok(());
    }
    Err(format!(
        "isolated Git metadata remains after cleanup: {}",
        remaining
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(not(unix))]
fn ensure_no_git_metadata(_root: &Path) -> Result<(), String> {
    Err("descriptor_semantics_unavailable_for_git_metadata_cleanup".to_string())
}

#[cfg(unix)]
fn find_git_metadata(root: &Path) -> Result<Vec<PathBuf>, String> {
    let held = open_git_scan_root(root)?;
    let mut budget = GitScanBudget::default();
    let mut found = Vec::new();
    scan_git_directory(root, &held.root_fd, Path::new(""), &mut budget, &mut found)?;
    revalidate_git_scan_root(&held)?;
    found.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(found)
}

#[cfg(unix)]
fn find_git_metadata_held(
    root: &crate::shared_skill_source::DescriptorRoot,
) -> Result<Vec<PathBuf>, String> {
    let mut budget = GitScanBudget::default();
    let mut found = Vec::new();
    scan_git_directory(
        root.path(),
        root.descriptor(),
        Path::new(""),
        &mut budget,
        &mut found,
    )?;
    root.revalidate("candidate checkout")?;
    let mut relative = found
        .into_iter()
        .map(|path| {
            path.strip_prefix(root.path())
                .map(Path::to_path_buf)
                .map_err(|_| "git metadata path escaped held checkout".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    relative.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(relative)
}

#[cfg(unix)]
struct HeldGitScanRoot {
    parent_fd: std::os::fd::OwnedFd,
    root_fd: std::os::fd::OwnedFd,
    name: std::ffi::OsString,
    binding: GitStatBinding,
}

#[cfg(unix)]
#[derive(Default)]
struct GitScanBudget {
    directories: usize,
    entries: usize,
}

#[cfg(unix)]
#[derive(Default)]
struct GitCleanupBudget {
    entries: usize,
}

#[cfg(unix)]
type GitStatBinding = (u64, u64, u32, i128, i128, i128, i128, i128);

#[cfg(unix)]
fn git_stat_binding(stat: &Stat) -> GitStatBinding {
    (
        stat.st_dev as u64,
        stat.st_ino,
        stat.st_mode as u32,
        stat.st_size as i128,
        stat.st_mtime as i128,
        stat.st_mtime_nsec as i128,
        stat.st_ctime as i128,
        stat.st_ctime_nsec as i128,
    )
}

#[cfg(unix)]
fn git_stat_identity(stat: &Stat) -> (u64, u64, u32) {
    (stat.st_dev as u64, stat.st_ino, stat.st_mode as u32)
}

#[cfg(unix)]
fn open_git_scan_root(root: &Path) -> Result<HeldGitScanRoot, String> {
    let parent = root
        .parent()
        .ok_or_else(|| format!("candidate checkout has no parent: {}", root.display()))?;
    let name = root
        .file_name()
        .ok_or_else(|| format!("candidate checkout has no name: {}", root.display()))?
        .to_os_string();
    let parent_fd = rustix::fs::open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open candidate checkout parent: {error}"))?;
    let named =
        rustix::fs::statat(&parent_fd, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            format!(
                "cannot inspect candidate checkout {}: {error}",
                root.display()
            )
        })?;
    if FileType::from_raw_mode(named.st_mode) != FileType::Directory {
        return Err(format!(
            "symlink_refused: candidate checkout {}",
            root.display()
        ));
    }
    let root_fd = rustix::fs::openat(
        &parent_fd,
        &name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open candidate checkout {}: {error}", root.display()))?;
    let opened = rustix::fs::fstat(&root_fd)
        .map_err(|error| format!("cannot stat candidate checkout fd: {error}"))?;
    if git_stat_binding(&named) != git_stat_binding(&opened) {
        return Err("git_metadata_root_identity_drift".to_string());
    }
    Ok(HeldGitScanRoot {
        parent_fd,
        root_fd,
        name,
        binding: git_stat_binding(&opened),
    })
}

#[cfg(unix)]
fn revalidate_git_scan_root(root: &HeldGitScanRoot) -> Result<(), String> {
    let opened = rustix::fs::fstat(&root.root_fd)
        .map_err(|error| format!("cannot revalidate candidate checkout fd: {error}"))?;
    let named = rustix::fs::statat(&root.parent_fd, &root.name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot revalidate candidate checkout path: {error}"))?;
    if root.binding != git_stat_binding(&opened) || root.binding != git_stat_binding(&named) {
        return Err("git_metadata_root_identity_drift".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn revalidate_git_scan_root_identity(root: &HeldGitScanRoot) -> Result<(), String> {
    let opened = rustix::fs::fstat(&root.root_fd)
        .map_err(|error| format!("cannot revalidate candidate checkout fd: {error}"))?;
    let named = rustix::fs::statat(&root.parent_fd, &root.name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot revalidate candidate checkout path: {error}"))?;
    let expected = (root.binding.0, root.binding.1, root.binding.2);
    if expected != git_stat_identity(&opened) || expected != git_stat_identity(&named) {
        return Err("git_metadata_root_identity_drift".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn scan_git_directory(
    root: &Path,
    directory_fd: &impl std::os::fd::AsFd,
    relative: &Path,
    budget: &mut GitScanBudget,
    found: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let directory_before = rustix::fs::fstat(directory_fd)
        .map_err(|error| format!("cannot stat isolated checkout directory: {error}"))?;
    for entry in Dir::read_from(directory_fd)
        .map_err(|error| format!("cannot duplicate isolated checkout directory fd: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("cannot enumerate isolated checkout: {error}"))?;
        let name_bytes = entry.file_name().to_bytes();
        if matches!(name_bytes, b"." | b"..") {
            continue;
        }
        if budget.entries >= MAX_GIT_SCAN_ENTRIES {
            return Err("git_metadata_entry_budget_exceeded".to_string());
        }
        budget.entries += 1;
        if name_bytes.len() > MAX_GIT_SCAN_NAME_BYTES {
            return Err("git_metadata_name_budget_exceeded".to_string());
        }
        let name = std::ffi::OsStr::from_bytes(name_bytes);
        let child_relative = relative.join(name);
        let child_path = root.join(&child_relative);
        let named_before = rustix::fs::statat(directory_fd, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| {
                format!(
                    "cannot inspect isolated checkout {}: {error}",
                    child_path.display()
                )
            })?;
        #[cfg(test)]
        AFTER_GIT_METADATA_NAMED_STAT_HOOK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        let kind = FileType::from_raw_mode(named_before.st_mode);
        if name_bytes == b".git" {
            if !matches!(
                kind,
                FileType::Directory | FileType::RegularFile | FileType::Symlink
            ) {
                return Err(format!("special_file_refused: {}", child_path.display()));
            }
            if found.len() >= MAX_GIT_METADATA_RESULTS {
                return Err("git_metadata_result_budget_exceeded".to_string());
            }
            found.push(child_path);
        } else if kind == FileType::Directory {
            if budget.directories >= MAX_GIT_SCAN_DIRECTORIES {
                return Err("git_metadata_directory_budget_exceeded".to_string());
            }
            budget.directories += 1;
            let child_fd = rustix::fs::openat(
                directory_fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                format!(
                    "cannot open isolated checkout {}: {error}",
                    child_path.display()
                )
            })?;
            let opened = rustix::fs::fstat(&child_fd)
                .map_err(|error| format!("cannot stat isolated checkout fd: {error}"))?;
            if git_stat_binding(&named_before) != git_stat_binding(&opened) {
                return Err("git_metadata_directory_identity_drift".to_string());
            }
            scan_git_directory(root, &child_fd, &child_relative, budget, found)?;
            let opened_after = rustix::fs::fstat(&child_fd)
                .map_err(|error| format!("cannot revalidate isolated checkout fd: {error}"))?;
            let named_after = rustix::fs::statat(directory_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate isolated checkout path: {error}"))?;
            if git_stat_binding(&opened) != git_stat_binding(&opened_after)
                || git_stat_binding(&opened) != git_stat_binding(&named_after)
            {
                return Err("git_metadata_directory_identity_drift".to_string());
            }
        }
    }
    let directory_after = rustix::fs::fstat(directory_fd)
        .map_err(|error| format!("cannot revalidate isolated checkout directory: {error}"))?;
    if git_stat_binding(&directory_before) != git_stat_binding(&directory_after) {
        return Err("git_metadata_directory_identity_drift".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn remove_git_metadata_path(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "git_metadata_path_escapes_checkout".to_string())?;
    let name = relative
        .file_name()
        .ok_or_else(|| "git metadata target has no name".to_string())?;
    let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
    let held = open_git_scan_root(root)?;
    let parent_fd = open_git_relative_directory(&held.root_fd, parent_relative)?;
    let stat = match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot inspect Git metadata {}: {error}",
                path.display()
            ))
        }
    };
    match FileType::from_raw_mode(stat.st_mode) {
        FileType::Directory => {
            let mut budget = GitCleanupBudget::default();
            remove_git_directory_at(&parent_fd, name, path, &mut budget)?;
        }
        FileType::RegularFile | FileType::Symlink => {
            rustix::fs::unlinkat(&parent_fd, name, AtFlags::empty()).map_err(|error| {
                format!("cannot remove Git metadata {}: {error}", path.display())
            })?;
        }
        _ => return Err(format!("special_file_refused: {}", path.display())),
    }
    revalidate_git_scan_root_identity(&held)?;
    Ok(())
}

#[cfg(unix)]
fn open_git_relative_directory(
    root_fd: &impl std::os::fd::AsFd,
    relative: &Path,
) -> Result<std::os::fd::OwnedFd, String> {
    let mut current = rustix::io::dup(root_fd)
        .map_err(|error| format!("cannot duplicate checkout root fd: {error}"))?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("git_metadata_relative_path_invalid".to_string());
        };
        let named = rustix::fs::statat(&current, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot inspect Git metadata parent: {error}"))?;
        if FileType::from_raw_mode(named.st_mode) != FileType::Directory {
            return Err("git_metadata_parent_not_directory".to_string());
        }
        let child = rustix::fs::openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| format!("cannot open Git metadata parent: {error}"))?;
        let opened = rustix::fs::fstat(&child)
            .map_err(|error| format!("cannot stat Git metadata parent: {error}"))?;
        if git_stat_binding(&named) != git_stat_binding(&opened) {
            return Err("git_metadata_parent_identity_drift".to_string());
        }
        current = child;
    }
    Ok(current)
}

#[cfg(unix)]
fn remove_git_directory_at(
    parent_fd: &impl std::os::fd::AsFd,
    name: &std::ffi::OsStr,
    display: &Path,
    budget: &mut GitCleanupBudget,
) -> Result<(), String> {
    let named =
        rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| {
            format!(
                "cannot inspect Git directory {}: {error}",
                display.display()
            )
        })?;
    let directory = rustix::fs::openat(
        parent_fd,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("cannot open Git directory {}: {error}", display.display()))?;
    let opened = rustix::fs::fstat(&directory)
        .map_err(|error| format!("cannot stat Git directory {}: {error}", display.display()))?;
    if git_stat_binding(&named) != git_stat_binding(&opened) {
        return Err("git_metadata_cleanup_identity_drift".to_string());
    }
    for entry in Dir::read_from(&directory).map_err(|error| {
        format!(
            "cannot enumerate Git directory {}: {error}",
            display.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("cannot enumerate Git metadata: {error}"))?;
        let name_bytes = entry.file_name().to_bytes();
        if matches!(name_bytes, b"." | b"..") {
            continue;
        }
        if budget.entries >= MAX_GIT_CLEANUP_ENTRIES {
            return Err("git_metadata_cleanup_entry_budget_exceeded".to_string());
        }
        budget.entries += 1;
        if name_bytes.len() > MAX_GIT_SCAN_NAME_BYTES {
            return Err("git_metadata_name_budget_exceeded".to_string());
        }
        let child_name = std::ffi::OsStr::from_bytes(name_bytes);
        let child_display = display.join(child_name);
        let child = rustix::fs::statat(&directory, child_name, AtFlags::SYMLINK_NOFOLLOW).map_err(
            |error| {
                format!(
                    "cannot inspect Git metadata {}: {error}",
                    child_display.display()
                )
            },
        )?;
        match FileType::from_raw_mode(child.st_mode) {
            FileType::Directory => {
                remove_git_directory_at(&directory, child_name, &child_display, budget)?;
            }
            FileType::RegularFile | FileType::Symlink => {
                rustix::fs::unlinkat(&directory, child_name, AtFlags::empty()).map_err(
                    |error| {
                        format!(
                            "cannot remove Git metadata {}: {error}",
                            child_display.display()
                        )
                    },
                )?;
            }
            _ => return Err(format!("special_file_refused: {}", child_display.display())),
        }
    }
    let opened_after = rustix::fs::fstat(&directory)
        .map_err(|error| format!("cannot revalidate Git directory: {error}"))?;
    let named_after = rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("cannot revalidate Git directory path: {error}"))?;
    if git_stat_identity(&opened) != git_stat_identity(&opened_after)
        || git_stat_identity(&opened) != git_stat_identity(&named_after)
    {
        return Err("git_metadata_cleanup_identity_drift".to_string());
    }
    rustix::fs::unlinkat(parent_fd, name, AtFlags::REMOVEDIR)
        .map_err(|error| format!("cannot remove Git directory {}: {error}", display.display()))?;
    Ok(())
}

#[cfg(all(test, unix))]
mod descriptor_git_metadata_tests {
    use super::*;

    #[test]
    fn queued_child_replacement_with_symlink_fails_without_visiting_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("checkout");
        let child = root.join("queued-child");
        let moved = root.join("queued-child-old");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(outside.join(".git")).unwrap();
        let child_for_hook = child.clone();
        let outside_for_hook = outside.clone();
        AFTER_GIT_METADATA_NAMED_STAT_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::rename(&child_for_hook, &moved).unwrap();
                std::os::unix::fs::symlink(&outside_for_hook, &child_for_hook).unwrap();
            }));
        });

        assert!(
            find_git_metadata(&root).is_err(),
            "queued pathname traversal accepted a child replaced by an outside symlink"
        );
        assert!(
            outside.join(".git").is_dir(),
            "metadata cleanup reached outside the held checkout root"
        );
    }
}

#[cfg(all(test, unix))]
mod descriptor_candidate_manifest_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn manifest(identity: &str) -> CandidateManifest {
        CandidateManifest {
            schema_version: CANDIDATE_MANIFEST_SCHEMA.to_string(),
            candidate_identity: identity.to_string(),
            repository_url: "https://github.com/example/repository".to_string(),
            resolved_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            subdir: "skill".to_string(),
            body_hash: "sha256:body".to_string(),
        }
    }

    #[test]
    fn manifest_read_rejects_named_file_replaced_by_outside_symlink() {
        let temp = tempfile::TempDir::new().unwrap();
        let temp_root = temp.path().to_path_buf();
        let candidate = temp_root.join("candidate");
        fs::create_dir(&candidate).unwrap();
        let path = candidate.join(CANDIDATE_MANIFEST_FILE);
        fs::write(&path, serde_json::to_vec(&manifest("inside")).unwrap()).unwrap();
        let outside = temp_root.join("outside.json");
        fs::write(&outside, serde_json::to_vec(&manifest("outside")).unwrap()).unwrap();
        let path_for_hook = path.clone();
        let outside_for_hook = outside.clone();
        crate::shared_skill_source::set_after_bounded_file_named_stat_hook(Box::new(move || {
            fs::remove_file(&path_for_hook).unwrap();
            symlink(&outside_for_hook, &path_for_hook).unwrap();
        }));

        assert!(
            read_candidate_manifest(&candidate).is_err(),
            "manifest reader followed a symlink installed after the named stat"
        );
        assert_eq!(
            fs::read(&outside).unwrap(),
            serde_json::to_vec(&manifest("outside")).unwrap()
        );
    }

    #[test]
    fn manifest_read_rejects_oversized_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let candidate = temp.path().join("candidate");
        fs::create_dir(&candidate).unwrap();
        fs::write(
            candidate.join(CANDIDATE_MANIFEST_FILE),
            vec![b' '; MAX_CANDIDATE_MANIFEST_BYTES as usize + 1],
        )
        .unwrap();

        assert_eq!(
            read_candidate_manifest(&candidate).unwrap_err(),
            "candidate_manifest_too_large"
        );
    }

    #[test]
    fn manifest_read_rejects_growth_after_opened_stat() {
        let temp = tempfile::TempDir::new().unwrap();
        let candidate = temp.path().join("candidate");
        fs::create_dir(&candidate).unwrap();
        let path = candidate.join(CANDIDATE_MANIFEST_FILE);
        fs::write(&path, serde_json::to_vec(&manifest("inside")).unwrap()).unwrap();
        crate::shared_skill_source::set_after_bounded_file_opened_stat_hook(Box::new(move || {
            fs::write(path, vec![b' '; MAX_CANDIDATE_MANIFEST_BYTES as usize + 1]).unwrap();
        }));

        assert_eq!(
            read_candidate_manifest(&candidate).unwrap_err(),
            "candidate_manifest_too_large"
        );
    }

    #[test]
    fn manifest_write_refuses_preexisting_symlink_without_touching_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let temp_root = temp.path().to_path_buf();
        let candidate = temp_root.join("candidate");
        fs::create_dir(&candidate).unwrap();
        let outside = temp_root.join("outside.json");
        fs::write(&outside, b"outside-sentinel").unwrap();
        symlink(&outside, candidate.join(CANDIDATE_MANIFEST_FILE)).unwrap();

        assert!(
            write_candidate_manifest(&candidate, &manifest("inside")).is_err(),
            "manifest writer followed a preexisting symlink"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"outside-sentinel");
    }

    #[test]
    fn manifest_write_rejects_candidate_root_swap_without_touching_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let temp_root = temp.path().to_path_buf();
        let candidate = temp_root.join("candidate");
        fs::create_dir(&candidate).unwrap();
        let moved = temp_root.join("candidate-held");
        let outside = temp_root.join("outside-candidate");
        fs::create_dir(&outside).unwrap();
        let candidate_for_hook = candidate.clone();
        let outside_for_hook = outside.clone();
        AFTER_CANDIDATE_MANIFEST_ROOT_OPEN_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::rename(&candidate_for_hook, &moved).unwrap();
                symlink(&outside_for_hook, &candidate_for_hook).unwrap();
            }));
        });

        let error = write_candidate_manifest(&candidate, &manifest("inside")).unwrap_err();
        assert!(error.contains("root_identity_drift"), "{error}");
        assert!(!outside.join(CANDIDATE_MANIFEST_FILE).exists());
    }

    #[test]
    fn manifest_io_has_no_pathname_read_or_write_fallback() {
        let source = include_str!("remote.rs");
        let manifest_io = source
            .split_once("fn read_candidate_manifest")
            .unwrap()
            .1
            .split_once("fn quarantine_candidate")
            .unwrap()
            .0;
        assert!(!manifest_io.contains(concat!("fs::", "read(")));
        assert!(!manifest_io.contains(concat!("fs::", "write(")));
    }
}

#[cfg(all(test, unix))]
mod descriptor_candidate_root_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct FixtureBackend;

    impl GitBackend for FixtureBackend {
        fn resolve_commit(
            &self,
            _repository_url: &str,
            _requested_ref: Option<&str>,
        ) -> Result<String, String> {
            Ok("a".repeat(40))
        }

        fn materialize_selected(
            &self,
            _repository_url: &str,
            _resolved_commit: &str,
            destination: &HeldCheckout,
            _subdir: &str,
            _license_paths: &[String],
        ) -> Result<(), String> {
            destination.create_dir_all(Path::new("skill"))?;
            destination.write(Path::new("LICENSE"), b"MIT License\n")?;
            destination.write(
                Path::new("skill/SKILL.md"),
                b"---\nname: candidate-root-team\ndescription: Fixture.\n---\n",
            )
        }
    }

    fn context(temp: &tempfile::TempDir) -> AdoptionContext {
        AdoptionContext {
            authority_root: Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .unwrap()
                .to_path_buf(),
            runtime_home: temp.path().join("runtime"),
            candidate_home: temp.path().join("runtime"),
            host_home: temp.path().join("home"),
            snapshot_discovery: super::super::model::SnapshotDiscovery::Offline,
        }
    }

    fn source() -> SourceSpec {
        SourceSpec::Git {
            url: "file:///fixture/candidate-root".to_string(),
            requested_ref: None,
            tracking_ref: None,
            subdir: Some("skill".to_string()),
        }
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn remote_candidate_uses_disposable_authority_not_installed_runtime() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut context = context(&temp);
        context.candidate_home = temp.path().join("plan-scratch");
        fs::create_dir_all(&context.runtime_home).unwrap();
        fs::write(context.runtime_home.join("sentinel"), b"installed-runtime").unwrap();

        let candidate =
            acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend).unwrap();

        assert!(candidate
            .checkout_root
            .starts_with(context.candidate_home.join("candidates")));
        assert!(!context.runtime_home.join("candidates").exists());
        assert_eq!(
            fs::read(context.runtime_home.join("sentinel")).unwrap(),
            b"installed-runtime"
        );
    }

    #[test]
    fn cached_candidate_replacement_after_path_check_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context(&temp);
        let first =
            acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend).unwrap();
        let candidate = first.checkout_root.parent().unwrap().to_path_buf();
        let outside = temp.path().join("outside-candidate");
        fs::rename(&candidate, &outside).unwrap();
        fs::create_dir(&candidate).unwrap();
        let expected = read_candidate_manifest(&outside).unwrap();
        let candidate_for_hook = candidate.clone();
        let outside_for_hook = outside.clone();
        AFTER_CANDIDATE_ROOT_PATH_CHECK_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::rename(
                    &candidate_for_hook,
                    candidate_for_hook.with_file_name("checked-candidate"),
                )
                .unwrap();
                symlink(&outside_for_hook, &candidate_for_hook).unwrap();
            }));
        });

        let error = validate_cached_candidate(&candidate, &expected)
            .expect_err("candidate root replacement must fail closed");
        assert!(error.contains("identity_drift"), "{error}");
        assert!(outside.join("checkout/skill/SKILL.md").is_file());
    }

    #[test]
    fn fresh_stage_replacement_never_materializes_into_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context(&temp);
        let candidates_root = context.runtime_home.join("candidates");
        let outside = temp.path().join("outside-stage");
        fs::create_dir(&outside).unwrap();
        let candidates_for_hook = candidates_root.clone();
        let outside_for_hook = outside.clone();
        AFTER_FRESH_STAGE_CREATE_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let stage = fs::read_dir(&candidates_for_hook)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".stage-"))
                    })
                    .unwrap();
                let held = candidates_for_hook.join("held-stage");
                fs::rename(&stage, &held).unwrap();
                symlink(&outside_for_hook, &stage).unwrap();
            }));
        });

        let error = acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend)
            .expect_err("fresh stage replacement must fail closed");
        assert!(!outside.join("checkout/skill/SKILL.md").exists());
        assert!(!error.is_empty());
    }

    #[test]
    fn cached_root_replacement_after_manifest_read_never_scans_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context(&temp);
        let first =
            acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend).unwrap();
        let candidate = first.checkout_root.parent().unwrap().to_path_buf();
        let outside = temp.path().join("outside-matching-candidate");
        copy_tree(&candidate, &outside);
        let outside_skill = outside.join("checkout/skill/SKILL.md");
        let outside_before = fs::read(&outside_skill).unwrap();
        let candidate_for_hook = candidate.clone();
        let outside_for_hook = outside.clone();
        AFTER_CACHED_MANIFEST_READ_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::rename(
                    &candidate_for_hook,
                    candidate_for_hook.with_file_name("held-cached-candidate"),
                )
                .unwrap();
                symlink(outside_for_hook, candidate_for_hook).unwrap();
            }));
        });
        let expected = read_candidate_manifest(&outside).unwrap();

        let error = validate_cached_candidate(&candidate, &expected)
            .expect_err("cached root replacement after manifest read must fail closed");
        assert!(error.contains("identity_drift"), "{error}");
        assert_eq!(fs::read(outside_skill).unwrap(), outside_before);
    }

    #[test]
    fn held_stage_replacement_before_backend_never_writes_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context(&temp);
        let candidates_root = context.runtime_home.join("candidates");
        let outside = temp.path().join("outside-before-backend");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"outside unchanged").unwrap();
        let candidates_for_hook = candidates_root.clone();
        let outside_for_hook = outside.clone();
        AFTER_HELD_STAGE_REVALIDATE_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let stage = fs::read_dir(&candidates_for_hook)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".stage-")
                    })
                    .unwrap();
                fs::rename(
                    &stage,
                    candidates_for_hook.join("held-stage-after-revalidate"),
                )
                .unwrap();
                symlink(outside_for_hook, stage).unwrap();
            }));
        });

        let result = acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend);
        assert!(result.is_err(), "stage replacement was accepted");
        assert_eq!(
            fs::read(outside.join("sentinel")).unwrap(),
            b"outside unchanged"
        );
        assert!(!outside.join("checkout/skill/SKILL.md").exists());
        assert!(!candidates_root.join(candidate_name_for(&source())).exists());
    }

    #[test]
    fn stage_replacement_after_manifest_write_is_not_published() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = context(&temp);
        let candidates_root = context.runtime_home.join("candidates");
        let outside = temp.path().join("outside-before-publish");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"outside unchanged").unwrap();
        let candidates_for_hook = candidates_root.clone();
        let outside_for_hook = outside.clone();
        AFTER_CANDIDATE_MANIFEST_WRITE_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let stage = fs::read_dir(&candidates_for_hook)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with(".stage-")
                    })
                    .unwrap();
                fs::rename(
                    &stage,
                    candidates_for_hook.join("held-stage-after-manifest"),
                )
                .unwrap();
                symlink(outside_for_hook, stage).unwrap();
            }));
        });

        let result = acquire_remote_candidate_with_backend(&context, &source(), &FixtureBackend);
        assert!(result.is_err(), "replaced stage was published");
        assert_eq!(
            fs::read(outside.join("sentinel")).unwrap(),
            b"outside unchanged"
        );
        assert!(!candidates_root.join(candidate_name_for(&source())).exists());
    }

    fn candidate_name_for(source: &SourceSpec) -> String {
        let SourceSpec::Git { url, subdir, .. } = source else {
            unreachable!()
        };
        ags_platform::sha256(
            serde_json::to_vec(&(
                CANDIDATE_MANIFEST_SCHEMA,
                url,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                subdir.as_deref().unwrap_or_default(),
            ))
            .unwrap(),
        )
        .trim_start_matches("sha256:")
        .to_string()
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
        destination: &HeldCheckout,
    ) -> Result<Option<Vec<RemoteTreeEntry>>, String> {
        init_git_repository(repository_url, destination)?;
        fetch_git_commit(destination, resolved_commit)?;
        let output = run_git_held(
            destination,
            &[
                "ls-tree".to_string(),
                "-r".to_string(),
                "-l".to_string(),
                "-z".to_string(),
                resolved_commit.to_string(),
            ],
        )?;
        parse_tree_metadata(&output)
    }

    fn materialize_selected(
        &self,
        _repository_url: &str,
        resolved_commit: &str,
        destination: &HeldCheckout,
        subdir: &str,
        license_paths: &[String],
    ) -> Result<(), String> {
        run_git_held(
            destination,
            &[
                "sparse-checkout".to_string(),
                "init".to_string(),
                "--no-cone".to_string(),
            ],
        )?;
        let mut patterns = Vec::new();
        if subdir.is_empty() {
            // The repository root is the complete selected Skill body. A
            // SKILL.md-only sparse checkout would make every referenced root
            // file disappear and then fail metadata validation. Materialize
            // the full tracked root so hashing and audit cover the exact body.
            patterns.push("/*".to_string());
        } else {
            patterns.push(format!("/{subdir}/**"));
        }
        patterns.extend(license_paths.iter().map(|path| format!("/{path}")));
        let mut sparse_args = vec![
            "sparse-checkout".to_string(),
            "set".to_string(),
            "--no-cone".to_string(),
            "--".to_string(),
        ];
        sparse_args.extend(patterns);
        run_git_held(destination, &sparse_args)?;
        run_git_held(
            destination,
            &[
                "checkout".to_string(),
                "--detach".to_string(),
                "--force".to_string(),
                resolved_commit.to_string(),
            ],
        )?;
        Ok(())
    }

    fn validate_checkout(&self, destination: &HeldCheckout) -> Result<(), String> {
        let output = run_git_held(
            destination,
            &[
                "ls-tree".to_string(),
                "-r".to_string(),
                "-z".to_string(),
                "HEAD".to_string(),
            ],
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

fn init_git_repository(repository_url: &str, destination: &HeldCheckout) -> Result<(), String> {
    run_git_held(
        destination,
        &["init".to_string(), "--quiet".to_string(), ".".to_string()],
    )?;
    run_git_held(
        destination,
        &[
            "remote".to_string(),
            "add".to_string(),
            "origin".to_string(),
            repository_url.to_string(),
        ],
    )?;
    Ok(())
}

fn fetch_git_commit(destination: &HeldCheckout, resolved_commit: &str) -> Result<(), String> {
    run_git_held(
        destination,
        &[
            "fetch".to_string(),
            "--filter=blob:none".to_string(),
            "--no-tags".to_string(),
            "--no-recurse-submodules".to_string(),
            "--depth=1".to_string(),
            "origin".to_string(),
            resolved_commit.to_string(),
        ],
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
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", no_hooks_path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_ASKPASS")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_PROXY_COMMAND");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
    }
    let (status, stdout, stderr) =
        run_bounded_git_process(&mut command, "system git", GIT_PROCESS_TIMEOUT)?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(format!("system git failed ({}): {}", status, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

#[cfg(unix)]
fn run_git_held(destination: &HeldCheckout, args: &[String]) -> Result<String, String> {
    use std::os::unix::process::CommandExt as _;

    let no_hooks_path = "/dev/null";
    let held = rustix::io::dup(destination.root.descriptor())
        .map_err(|error| format!("cannot duplicate held checkout for git: {error}"))?;
    let mut command = Command::new("git");
    command
        .arg("-c")
        .arg(format!("core.hooksPath={no_hooks_path}"))
        .args(args);
    unsafe {
        command.pre_exec(move || {
            rustix::process::fchdir(&held).map_err(std::io::Error::from)?;
            rustix::process::setsid()
                .map(|_| ())
                .map_err(std::io::Error::from)
        });
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", no_hooks_path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_ASKPASS")
        .env_remove("GIT_SSH")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_PROXY_COMMAND");
    let (status, stdout, stderr) = run_bounded_git_process(
        &mut command,
        "system git in held checkout",
        GIT_PROCESS_TIMEOUT,
    )?;
    if !status.success() {
        return Err(format!(
            "system git failed ({}): {}",
            status,
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

fn run_bounded_git_process(
    command: &mut Command,
    label: &str,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start {label}: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{label} stdout pipe is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{label} stderr pipe is unavailable"))?;
    let stdout_reader = std::thread::spawn(move || drain_bounded_git_output(stdout));
    let stderr_reader = std::thread::spawn(move || drain_bounded_git_output(stderr));
    let started = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll {label}: {error}"))?
        {
            break (status, false);
        }
        if started.elapsed() >= timeout {
            terminate_git_process(&mut child)?;
            let status = child
                .wait()
                .map_err(|error| format!("cannot reap timed-out {label}: {error}"))?;
            break (status, true);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let (stdout, stdout_overflow) = stdout_reader
        .join()
        .map_err(|_| format!("{label} stdout reader panicked"))??;
    let (stderr, stderr_overflow) = stderr_reader
        .join()
        .map_err(|_| format!("{label} stderr reader panicked"))??;
    if timed_out {
        return Err(format!("{label} exceeded {} ms", timeout.as_millis()));
    }
    if stdout_overflow || stderr_overflow {
        return Err(format!(
            "{label} output exceeded {MAX_GIT_PROCESS_OUTPUT_BYTES} bytes"
        ));
    }
    Ok((status, stdout, stderr))
}

fn drain_bounded_git_output(mut reader: impl Read) -> Result<(Vec<u8>, bool), String> {
    let mut bytes = Vec::new();
    let mut overflow = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("cannot read system git output: {error}"))?;
        if read == 0 {
            break;
        }
        let remaining = MAX_GIT_PROCESS_OUTPUT_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..read.min(remaining)]);
        overflow |= read > remaining;
    }
    Ok((bytes, overflow))
}

#[cfg(unix)]
fn terminate_git_process(child: &mut std::process::Child) -> Result<(), String> {
    let pid = rustix::process::Pid::from_raw(child.id() as _)
        .ok_or_else(|| "system git returned an invalid process id".to_string())?;
    match rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(format!("cannot terminate timed-out system git: {error}")),
    }
}

#[cfg(not(unix))]
fn terminate_git_process(child: &mut std::process::Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("cannot terminate timed-out system git: {error}"))
}

#[cfg(not(unix))]
fn run_git_held(_destination: &HeldCheckout, _args: &[String]) -> Result<String, String> {
    Err("descriptor_semantics_unavailable_for_held_git_checkout".to_string())
}

#[cfg(all(test, unix))]
mod bounded_process_tests {
    use super::*;

    #[test]
    fn git_process_timeout_terminates_the_dedicated_process_group() {
        use std::os::unix::process::CommandExt as _;

        let mut command = Command::new("/bin/sleep");
        command.arg("5");
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
        let error =
            run_bounded_git_process(&mut command, "timeout fixture", Duration::from_millis(50))
                .unwrap_err();
        assert!(error.contains("exceeded 50 ms"), "{error}");
    }

    #[test]
    fn git_process_output_is_drained_but_rejected_over_budget() {
        let temp = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        let large = temp.path().join("large.bin");
        std::fs::write(&large, vec![b'x'; MAX_GIT_PROCESS_OUTPUT_BYTES + 1]).unwrap();
        let hash = Command::new("git")
            .args(["hash-object", "-w", "large.bin"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert!(hash.status.success());
        let hash = String::from_utf8(hash.stdout).unwrap();
        let mut command = Command::new("git");
        command
            .args(["cat-file", "blob", hash.trim()])
            .current_dir(temp.path());
        use std::os::unix::process::CommandExt as _;
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::from)
            });
        }
        let error = run_bounded_git_process(&mut command, "output fixture", Duration::from_secs(5))
            .unwrap_err();
        assert!(error.contains("output exceeded"), "{error}");
    }
}
