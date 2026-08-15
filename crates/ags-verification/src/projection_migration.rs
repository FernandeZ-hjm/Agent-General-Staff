//! Fail-closed ownership planning and apply for lightweight v2 projection.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::path::Component;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
const OWNERSHIP_RELATIVE_PATH: &str = ".ags/ownership-v2.json";
#[cfg(unix)]
const OWNERSHIP_SCHEMA: &str = "ags://schema/contract/v2/project-ownership";
#[cfg(unix)]
const MAX_ROLLBACK_BYTES: usize = 1024 * 1024;
#[cfg(unix)]
const QUARANTINE_STATE_NAME: &str = ".ags-projection-state";

#[cfg(unix)]
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::ffi::{CString, OsStr};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationDisposition {
    Create,
    ReclaimExactOwned,
    PreserveUnowned,
    PreserveModified,
}

/// Opaque, process-local migration plan.
///
/// The fields are private and the type is intentionally not serializable or
/// deserializable. Callers can inspect its read-only projection but can obtain
/// an apply-capable value only from [`plan_projection_migration`]. Apply still
/// recomputes ownership from the recorded owned hash and current bytes.
#[derive(Clone, Debug)]
pub struct ProjectionMigration {
    #[cfg(unix)]
    canonical_workspace: PathBuf,
    #[cfg(unix)]
    relative_path: PathBuf,
    canonical_path: PathBuf,
    current_sha256: Option<String>,
    recorded_owned_sha256: Option<String>,
    disposition: MigrationDisposition,
    #[cfg(unix)]
    parent_identity: FileIdentity,
    #[cfg(unix)]
    workspace_fd: Arc<OwnedFd>,
    #[cfg(unix)]
    workspace_identity: FileIdentity,
    #[cfg(unix)]
    target_identity: Option<FileIdentity>,
}

impl ProjectionMigration {
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn current_sha256(&self) -> Option<&str> {
        self.current_sha256.as_deref()
    }

    pub fn recorded_owned_sha256(&self) -> Option<&str> {
        self.recorded_owned_sha256.as_deref()
    }

    pub fn disposition(&self) -> MigrationDisposition {
        self.disposition
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectionMigrationReceipt {
    pub schema_version: String,
    pub canonical_path: String,
    pub disposition: MigrationDisposition,
    pub previous_sha256: Option<String>,
    pub result_sha256: Option<String>,
    pub changed: bool,
}

/// One generated file in the complete desired project projection.
///
/// Omitting a previously owned path requests deletion. Paths absent from the
/// ownership manifest remain user-owned and are never inferred as generated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProjectionFile {
    relative_path: PathBuf,
    bytes: Vec<u8>,
}

impl ProjectProjectionFile {
    pub fn write(relative_path: impl Into<PathBuf>, bytes: impl AsRef<[u8]>) -> Self {
        Self {
            relative_path: relative_path.into(),
            bytes: bytes.as_ref().to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProjectionDisposition {
    Create,
    UpdateExactOwned,
    DeleteExactOwned,
    AlreadyCurrent,
    PreserveUnowned,
    PreserveModified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectionConflict {
    pub relative_path: String,
    pub disposition: ProjectProjectionDisposition,
    pub current_sha256: Option<String>,
    pub details_uri: String,
}

/// Opaque transaction plan for files, profile, and ownership metadata.
///
/// The desired file list is complete: an exact-last-applied owned entry that
/// is omitted is deleted, while unowned or modified bytes are preserved and
/// surfaced as content-addressed conflicts.
#[derive(Clone, Debug)]
pub struct ProjectProjectionPlan {
    #[cfg(unix)]
    canonical_workspace: PathBuf,
    #[cfg(unix)]
    workspace_identity: FileIdentity,
    #[cfg(unix)]
    workspace_fd: Arc<OwnedFd>,
    #[cfg(unix)]
    entries: Vec<ProjectProjectionEntry>,
    planned_directories: Vec<PathBuf>,
    conflicts: Vec<ProjectionConflict>,
    details: BTreeMap<String, Vec<u8>>,
    #[cfg(unix)]
    previous_ownership_bytes: Option<Vec<u8>>,
    #[cfg(unix)]
    previous_ownership_sha256: Option<String>,
    #[cfg(unix)]
    next_ownership_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectProjectionMutation {
    CreateDirectory {
        relative_path: PathBuf,
        mode: u32,
    },
    WriteFile {
        relative_path: PathBuf,
        previous_bytes: Option<Vec<u8>>,
        next_bytes: Vec<u8>,
        mode: u32,
    },
    DeleteFile {
        relative_path: PathBuf,
        previous_bytes: Vec<u8>,
    },
}

impl ProjectProjectionPlan {
    pub fn planned_directories(&self) -> &[PathBuf] {
        &self.planned_directories
    }

    pub fn conflicts(&self) -> &[ProjectionConflict] {
        &self.conflicts
    }

    pub fn resolve_details(&self, uri: &str) -> Option<&[u8]> {
        self.details.get(uri).map(Vec::as_slice)
    }

    #[cfg(unix)]
    pub fn materialized_mutations(&self) -> Vec<ProjectProjectionMutation> {
        let mut mutations = self
            .planned_directories
            .iter()
            .cloned()
            .map(|relative_path| ProjectProjectionMutation::CreateDirectory {
                relative_path,
                mode: 0o755,
            })
            .collect::<Vec<_>>();
        for entry in &self.entries {
            match entry.disposition {
                ProjectProjectionDisposition::Create
                | ProjectProjectionDisposition::UpdateExactOwned => {
                    mutations.push(ProjectProjectionMutation::WriteFile {
                        relative_path: entry.relative_path.clone(),
                        previous_bytes: entry.previous_bytes.clone(),
                        next_bytes: entry
                            .desired_bytes
                            .clone()
                            .expect("write disposition has desired bytes"),
                        mode: 0o600,
                    });
                }
                ProjectProjectionDisposition::DeleteExactOwned => {
                    mutations.push(ProjectProjectionMutation::DeleteFile {
                        relative_path: entry.relative_path.clone(),
                        previous_bytes: entry
                            .previous_bytes
                            .clone()
                            .expect("delete disposition has previous bytes"),
                    });
                }
                ProjectProjectionDisposition::AlreadyCurrent
                | ProjectProjectionDisposition::PreserveUnowned
                | ProjectProjectionDisposition::PreserveModified => {}
            }
        }
        if self.previous_ownership_bytes.as_deref() != Some(self.next_ownership_bytes.as_slice()) {
            mutations.push(ProjectProjectionMutation::WriteFile {
                relative_path: PathBuf::from(OWNERSHIP_RELATIVE_PATH),
                previous_bytes: self.previous_ownership_bytes.clone(),
                next_bytes: self.next_ownership_bytes.clone(),
                mode: 0o600,
            });
        }
        mutations
    }
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectProjectionEntry {
    relative_path: PathBuf,
    desired_bytes: Option<Vec<u8>>,
    previous_bytes: Option<Vec<u8>>,
    previous_sha256: Option<String>,
    recorded_sha256: Option<String>,
    disposition: ProjectProjectionDisposition,
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnershipManifest {
    schema_version: String,
    entries: BTreeMap<String, OwnershipEntry>,
}

#[cfg(unix)]
impl Default for OwnershipManifest {
    fn default() -> Self {
        Self {
            schema_version: OWNERSHIP_SCHEMA.to_string(),
            entries: BTreeMap::new(),
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnershipEntry {
    last_applied_sha256: String,
}

#[cfg(unix)]
struct LoadedOwnership {
    bytes: Option<Vec<u8>>,
    sha256: Option<String>,
    manifest: OwnershipManifest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectProjectionReceipt {
    pub schema_version: String,
    pub changed: bool,
    pub ownership_sha256: String,
    pub files: Vec<ProjectProjectionFileReceipt>,
    pub conflicts: Vec<ProjectionConflict>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectProjectionFileReceipt {
    pub relative_path: String,
    pub disposition: ProjectProjectionDisposition,
    pub changed: bool,
}

#[cfg(all(test, unix))]
type DirectoryCreateHook<'a> = &'a mut dyn FnMut(&Path) -> Result<(), String>;

#[cfg(all(test, unix))]
struct ProjectApplyHooks<'a> {
    after_workspace_validation: Option<&'a mut dyn FnMut() -> Result<(), String>>,
    after_directory_create: Option<DirectoryCreateHook<'a>>,
    before_manifest_commit: Option<&'a mut dyn FnMut() -> Result<(), String>>,
    after_manifest_commit: Option<&'a mut dyn FnMut() -> Result<(), String>>,
}

#[cfg(all(test, unix))]
impl ProjectApplyHooks<'_> {
    fn noop() -> Self {
        Self {
            after_workspace_validation: None,
            after_directory_create: None,
            before_manifest_commit: None,
            after_manifest_commit: None,
        }
    }
}

/// Plan one complete project projection without granting serializable apply
/// authority. Missing parent directories are recorded for creation at apply.
#[cfg(unix)]
pub fn plan_project_projection(
    workspace: &Path,
    desired: &[ProjectProjectionFile],
) -> Result<ProjectProjectionPlan, String> {
    let canonical_workspace = workspace.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize projection workspace {}: {error}",
            workspace.display()
        )
    })?;
    let workspace_fd = Arc::new(open_workspace(&canonical_workspace)?);
    let workspace_identity = identity_of_fd(workspace_fd.as_raw_fd())?;

    let LoadedOwnership {
        bytes: previous_ownership_bytes,
        sha256: previous_ownership_sha256,
        manifest,
    } = load_ownership_manifest_from_fd(workspace_fd.as_raw_fd())?;
    let mut desired_by_path = BTreeMap::<String, Vec<u8>>::new();
    for file in desired {
        validate_relative(&file.relative_path)?;
        let relative = normalized_relative_string(&file.relative_path)?;
        if relative == OWNERSHIP_RELATIVE_PATH {
            return Err("ownership metadata is managed by the projection transaction".to_string());
        }
        if desired_by_path
            .insert(relative.clone(), file.bytes.clone())
            .is_some()
        {
            return Err(format!("duplicate desired projection path: {relative}"));
        }
    }

    let mut all_paths = BTreeSet::new();
    all_paths.extend(desired_by_path.keys().cloned());
    all_paths.extend(manifest.entries.keys().cloned());

    let mut next_manifest = manifest.clone();
    let mut entries = Vec::new();
    let mut conflicts = Vec::new();
    let mut details = BTreeMap::new();
    for relative in all_paths {
        let relative_path = PathBuf::from(&relative);
        validate_relative(&relative_path)?;
        let previous = inspect_projection_file_fd(
            workspace_fd.as_raw_fd(),
            &relative_path,
            Some(MAX_ROLLBACK_BYTES),
            || Ok(()),
        )?;
        let previous_sha256 = previous
            .as_ref()
            .map(|snapshot| snapshot.snapshot.sha256.clone());
        let recorded_sha256 = manifest
            .entries
            .get(&relative)
            .map(|entry| entry.last_applied_sha256.clone());
        let desired_bytes = desired_by_path.get(&relative).cloned();
        let desired_sha256 = desired_bytes.as_deref().map(ags_platform::sha256);

        let disposition = match (
            previous_sha256.as_deref(),
            recorded_sha256.as_deref(),
            desired_sha256.as_deref(),
        ) {
            (None, _, Some(_)) => ProjectProjectionDisposition::Create,
            (Some(_), None, Some(_)) => ProjectProjectionDisposition::PreserveUnowned,
            (Some(current), Some(recorded), Some(_)) if current != recorded => {
                ProjectProjectionDisposition::PreserveModified
            }
            (Some(current), Some(_), Some(desired)) if current == desired => {
                ProjectProjectionDisposition::AlreadyCurrent
            }
            (Some(_), Some(_), Some(_)) => ProjectProjectionDisposition::UpdateExactOwned,
            (Some(current), Some(recorded), None) if current == recorded => {
                ProjectProjectionDisposition::DeleteExactOwned
            }
            (Some(_), Some(_), None) => ProjectProjectionDisposition::PreserveModified,
            (None, Some(_), None) => ProjectProjectionDisposition::AlreadyCurrent,
            (None, None, None) | (Some(_), None, None) => {
                return Err(format!(
                    "internal projection set included an unowned deletion candidate: {relative}"
                ));
            }
        };
        let previous_bytes = match disposition {
            ProjectProjectionDisposition::UpdateExactOwned
            | ProjectProjectionDisposition::DeleteExactOwned => {
                let captured = previous
                    .as_ref()
                    .expect("owned mutation disposition has current bytes");
                if captured.exceeded_limit {
                    return Err(format!(
                        "risk-escalated: exact-owned rollback source exceeds {} bytes: {}",
                        MAX_ROLLBACK_BYTES,
                        relative_path.display()
                    ));
                }
                captured.bytes.clone()
            }
            _ => None,
        };

        match disposition {
            ProjectProjectionDisposition::Create
            | ProjectProjectionDisposition::UpdateExactOwned
            | ProjectProjectionDisposition::AlreadyCurrent
                if desired_sha256.is_some() =>
            {
                next_manifest.entries.insert(
                    relative.clone(),
                    OwnershipEntry {
                        last_applied_sha256: desired_sha256
                            .clone()
                            .expect("guarded desired digest"),
                    },
                );
            }
            ProjectProjectionDisposition::DeleteExactOwned
            | ProjectProjectionDisposition::AlreadyCurrent
                if desired_sha256.is_none() =>
            {
                next_manifest.entries.remove(&relative);
            }
            ProjectProjectionDisposition::PreserveUnowned => {
                next_manifest.entries.remove(&relative);
            }
            ProjectProjectionDisposition::PreserveModified => {}
            _ => {}
        }

        if matches!(
            disposition,
            ProjectProjectionDisposition::PreserveUnowned
                | ProjectProjectionDisposition::PreserveModified
        ) {
            let detail = serde_json::to_vec(&serde_json::json!({
                "schema_version": "ags://schema/contract/v2/project-projection-conflict",
                "relative_path": relative,
                "disposition": disposition,
                "current_sha256": previous_sha256,
                "recorded_last_applied_sha256": recorded_sha256,
                "desired_sha256": desired_sha256,
            }))
            .map_err(|error| format!("cannot encode projection conflict details: {error}"))?;
            let digest = ags_platform::sha256(&detail);
            let details_uri = format!(
                "ags-details://sha256/{}",
                digest.strip_prefix("sha256:").unwrap_or(&digest)
            );
            details.insert(details_uri.clone(), detail);
            conflicts.push(ProjectionConflict {
                relative_path: relative.clone(),
                disposition,
                current_sha256: previous_sha256.clone(),
                details_uri,
            });
        }

        entries.push(ProjectProjectionEntry {
            relative_path,
            desired_bytes,
            previous_bytes,
            previous_sha256,
            recorded_sha256,
            disposition,
        });
    }

    let mut next_ownership_bytes = serde_json::to_vec_pretty(&next_manifest)
        .map_err(|error| format!("cannot encode ownership-v2 metadata: {error}"))?;
    next_ownership_bytes.push(b'\n');
    let mut projected_paths = entries
        .iter()
        .filter_map(|entry| {
            entry
                .desired_bytes
                .as_ref()
                .map(|_| entry.relative_path.clone())
        })
        .collect::<Vec<_>>();
    projected_paths.push(PathBuf::from(OWNERSHIP_RELATIVE_PATH));
    let planned_directories =
        planned_projection_directories_from_fd(workspace_fd.as_raw_fd(), &projected_paths)?;

    Ok(ProjectProjectionPlan {
        canonical_workspace,
        #[cfg(unix)]
        workspace_identity,
        #[cfg(unix)]
        workspace_fd,
        entries,
        planned_directories,
        conflicts,
        details,
        previous_ownership_bytes,
        previous_ownership_sha256,
        next_ownership_bytes,
    })
}

#[cfg(not(unix))]
pub fn plan_project_projection(
    _workspace: &Path,
    _desired: &[ProjectProjectionFile],
) -> Result<ProjectProjectionPlan, String> {
    Err("project projection planning is blocked: no audited fd-relative backend".to_string())
}

/// Apply a complete projection as one recoverable transaction.
///
/// Generated files and exact-last-applied deletions are applied first and the
/// ownership manifest is committed last. Any failure attempts descriptor-bound
/// rollback of every earlier effect and removes only directories created by
/// this transaction that remain empty.
#[cfg(unix)]
pub fn apply_project_projection(
    plan: &ProjectProjectionPlan,
) -> Result<ProjectProjectionReceipt, String> {
    #[cfg(test)]
    return apply_project_projection_with_hooks(plan, &mut ProjectApplyHooks::noop());
    #[cfg(not(test))]
    return apply_project_projection_impl(plan, None);
}

#[cfg(not(unix))]
pub fn apply_project_projection(
    _plan: &ProjectProjectionPlan,
) -> Result<ProjectProjectionReceipt, String> {
    Err("project projection apply is blocked: no audited fd-relative backend".to_string())
}

#[cfg(all(test, unix))]
fn apply_project_projection_with_hooks(
    plan: &ProjectProjectionPlan,
    hooks: &mut ProjectApplyHooks<'_>,
) -> Result<ProjectProjectionReceipt, String> {
    apply_project_projection_impl(plan, Some(hooks))
}

#[cfg(unix)]
fn apply_project_projection_impl(
    plan: &ProjectProjectionPlan,
    #[cfg(test)] mut hooks: Option<&mut ProjectApplyHooks<'_>>,
    #[cfg(not(test))] _hooks: Option<()>,
) -> Result<ProjectProjectionReceipt, String> {
    let workspace_fd = Arc::clone(&plan.workspace_fd);
    if identity_of_fd(workspace_fd.as_raw_fd())? != plan.workspace_identity {
        return Err("project projection workspace identity changed after planning".to_string());
    }
    verify_workspace_path_binding(
        &plan.canonical_workspace,
        workspace_fd.as_raw_fd(),
        plan.workspace_identity,
    )?;
    let canonical_workspace = plan.canonical_workspace.clone();
    #[cfg(test)]
    if let Some(hook) = hooks
        .as_mut()
        .and_then(|hooks| hooks.after_workspace_validation.as_mut())
    {
        hook()?;
    }
    verify_workspace_path_binding(
        &plan.canonical_workspace,
        workspace_fd.as_raw_fd(),
        plan.workspace_identity,
    )?;
    let quarantine_state = establish_quarantine_state(workspace_fd.as_raw_fd())?;

    let mut created_directories = Vec::new();
    for relative in &plan.planned_directories {
        #[cfg(unix)]
        let created = match create_projection_directory(workspace_fd.as_raw_fd(), relative) {
            Ok(created) => created,
            Err(error) => {
                let rollback =
                    rollback_created_directories(&created_directories, &quarantine_state);
                return Err(join_rollback_error(error, rollback));
            }
        };
        #[cfg(unix)]
        created_directories.push(created);
        #[cfg(test)]
        if let Some(hook) = hooks
            .as_mut()
            .and_then(|hooks| hooks.after_directory_create.as_mut())
        {
            if let Err(error) = hook(relative) {
                let rollback =
                    rollback_created_directories(&created_directories, &quarantine_state);
                return Err(join_rollback_error(error, rollback));
            }
        }
    }

    for entry in &plan.entries {
        let current = match inspect_projection_file_fd(
            workspace_fd.as_raw_fd(),
            &entry.relative_path,
            None,
            || Ok(()),
        ) {
            Ok(current) => current.map(|current| current.snapshot.sha256),
            Err(error) => {
                let rollback =
                    rollback_created_directories(&created_directories, &quarantine_state);
                return Err(join_rollback_error(error, rollback));
            }
        };
        if current != entry.previous_sha256 {
            let rollback = rollback_created_directories(&created_directories, &quarantine_state);
            return Err(join_rollback_error(
                format!(
                    "project projection target bytes changed after planning: {}",
                    entry.relative_path.display()
                ),
                rollback,
            ));
        }
    }
    let ownership_current = match inspect_projection_file_fd(
        workspace_fd.as_raw_fd(),
        Path::new(OWNERSHIP_RELATIVE_PATH),
        None,
        || Ok(()),
    ) {
        Ok(current) => current.map(|current| current.snapshot.sha256),
        Err(error) => {
            let rollback = rollback_created_directories(&created_directories, &quarantine_state);
            return Err(join_rollback_error(error, rollback));
        }
    };
    if ownership_current != plan.previous_ownership_sha256 {
        let directory_rollback =
            rollback_created_directories(&created_directories, &quarantine_state);
        return Err(join_rollback_error(
            "ownership-v2 metadata changed after planning".to_string(),
            directory_rollback,
        ));
    }

    enum AppliedEffect {
        Write {
            migration: ProjectionMigration,
            replacement: Vec<u8>,
            previous: Option<Vec<u8>>,
        },
        Delete {
            relative_path: PathBuf,
            previous: Vec<u8>,
        },
    }

    let mut effects = Vec::<AppliedEffect>::new();
    let mut file_receipts = Vec::new();
    let mut changed = false;
    let apply_result = (|| -> Result<(), String> {
        for entry in &plan.entries {
            let entry_changed = match entry.disposition {
                ProjectProjectionDisposition::Create
                | ProjectProjectionDisposition::UpdateExactOwned => {
                    let replacement = entry
                        .desired_bytes
                        .as_ref()
                        .expect("write disposition has desired bytes")
                        .clone();
                    let recorded = match entry.disposition {
                        ProjectProjectionDisposition::Create => None,
                        ProjectProjectionDisposition::UpdateExactOwned => {
                            entry.recorded_sha256.as_deref()
                        }
                        _ => unreachable!(),
                    };
                    let migration = plan_projection_migration_from_root(
                        &canonical_workspace,
                        Arc::clone(&plan.workspace_fd),
                        &entry.relative_path,
                        recorded,
                    )?;
                    let receipt = apply_projection_migration_with_state(
                        &migration,
                        &quarantine_state,
                        &replacement,
                    )?;
                    if !receipt.changed {
                        return Err(format!(
                            "project projection write was unexpectedly preserved: {}",
                            entry.relative_path.display()
                        ));
                    }
                    effects.push(AppliedEffect::Write {
                        migration,
                        replacement,
                        previous: entry.previous_bytes.clone(),
                    });
                    true
                }
                ProjectProjectionDisposition::DeleteExactOwned => {
                    let migration = plan_projection_migration_from_root(
                        &canonical_workspace,
                        Arc::clone(&plan.workspace_fd),
                        &entry.relative_path,
                        entry.recorded_sha256.as_deref(),
                    )?;
                    delete_exact_projection_with_state(&migration, &quarantine_state, || Ok(()))?;
                    effects.push(AppliedEffect::Delete {
                        relative_path: entry.relative_path.clone(),
                        previous: entry
                            .previous_bytes
                            .clone()
                            .expect("delete disposition has previous bytes"),
                    });
                    true
                }
                ProjectProjectionDisposition::AlreadyCurrent
                | ProjectProjectionDisposition::PreserveUnowned
                | ProjectProjectionDisposition::PreserveModified => false,
            };
            changed |= entry_changed;
            file_receipts.push(ProjectProjectionFileReceipt {
                relative_path: normalized_relative_string(&entry.relative_path)?,
                disposition: entry.disposition,
                changed: entry_changed,
            });
        }

        #[cfg(test)]
        if let Some(hook) = hooks
            .as_mut()
            .and_then(|hooks| hooks.before_manifest_commit.as_mut())
        {
            hook()?;
        }
        #[cfg(unix)]
        verify_projected_file_set(plan, workspace_fd.as_raw_fd(), "before manifest")?;
        if plan.previous_ownership_bytes.as_deref() != Some(&plan.next_ownership_bytes) {
            let ownership_migration = plan_projection_migration_from_root(
                &canonical_workspace,
                Arc::clone(&plan.workspace_fd),
                Path::new(OWNERSHIP_RELATIVE_PATH),
                plan.previous_ownership_sha256.as_deref(),
            )?;
            let receipt = apply_projection_migration_with_state(
                &ownership_migration,
                &quarantine_state,
                &plan.next_ownership_bytes,
            )?;
            if !receipt.changed {
                return Err("ownership-v2 commit was unexpectedly preserved".to_string());
            }
            effects.push(AppliedEffect::Write {
                migration: ownership_migration,
                replacement: plan.next_ownership_bytes.clone(),
                previous: plan.previous_ownership_bytes.clone(),
            });
            changed = true;
        }
        #[cfg(test)]
        if let Some(hook) = hooks
            .as_mut()
            .and_then(|hooks| hooks.after_manifest_commit.as_mut())
        {
            hook()?;
        }
        #[cfg(unix)]
        {
            verify_projected_file_set(plan, workspace_fd.as_raw_fd(), "after manifest")?;
            verify_fd_bound_digest(
                workspace_fd.as_raw_fd(),
                Path::new(OWNERSHIP_RELATIVE_PATH),
                Some(ags_platform::sha256(&plan.next_ownership_bytes)),
                "after manifest ownership",
            )?;
        }
        Ok(())
    })();

    if let Err(error) = apply_result {
        let mut rollback_errors = Vec::new();
        for effect in effects.iter().rev() {
            let rollback = match effect {
                AppliedEffect::Write {
                    migration,
                    replacement,
                    previous,
                } => recover_projection_migration_with_state(
                    migration,
                    &quarantine_state,
                    replacement,
                    previous.as_deref(),
                    || Ok(()),
                ),
                AppliedEffect::Delete {
                    relative_path,
                    previous,
                } => plan_projection_migration_from_root(
                    &canonical_workspace,
                    Arc::clone(&plan.workspace_fd),
                    relative_path,
                    None,
                )
                .and_then(|migration| {
                        apply_projection_migration_with_state(
                            &migration,
                            &quarantine_state,
                            previous,
                        )
                        .and_then(|receipt| {
                            if receipt.changed
                                && receipt.disposition == MigrationDisposition::Create
                            {
                                Ok(())
                            } else {
                                Err(format!(
                                    "risk-escalated: delete rollback preserved an appeared user path instead of restoring owned bytes: {}",
                                    relative_path.display()
                                ))
                            }
                        })
                    }),
            };
            if let Err(rollback_error) = rollback {
                rollback_errors.push(rollback_error);
            }
        }
        let directory_rollback =
            rollback_created_directories(&created_directories, &quarantine_state);
        if let Err(rollback_error) = directory_rollback {
            rollback_errors.push(rollback_error);
        }
        if rollback_errors.is_empty() {
            return Err(error);
        }
        return Err(format!(
            "{error}; projection rollback also failed: {}",
            rollback_errors.join("; ")
        ));
    }

    Ok(ProjectProjectionReceipt {
        schema_version: "ags://schema/contract/v2/project-projection-receipt".to_string(),
        changed,
        ownership_sha256: ags_platform::sha256(&plan.next_ownership_bytes),
        files: file_receipts,
        conflicts: plan.conflicts.clone(),
    })
}

#[cfg(unix)]
fn normalized_relative_string(relative: &Path) -> Result<String, String> {
    validate_relative(relative)?;
    Ok(relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_string_lossy().into_owned(),
            _ => unreachable!("validated projection component"),
        })
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(unix)]
fn verify_projected_file_set(
    plan: &ProjectProjectionPlan,
    workspace_fd: i32,
    stage: &str,
) -> Result<(), String> {
    for entry in &plan.entries {
        let expected = match entry.disposition {
            ProjectProjectionDisposition::Create
            | ProjectProjectionDisposition::UpdateExactOwned => {
                entry.desired_bytes.as_deref().map(ags_platform::sha256)
            }
            ProjectProjectionDisposition::DeleteExactOwned => None,
            ProjectProjectionDisposition::AlreadyCurrent => {
                entry.desired_bytes.as_deref().map(ags_platform::sha256)
            }
            ProjectProjectionDisposition::PreserveUnowned
            | ProjectProjectionDisposition::PreserveModified => entry.previous_sha256.clone(),
        };
        verify_fd_bound_digest(workspace_fd, &entry.relative_path, expected, stage)?;
    }
    Ok(())
}

#[cfg(unix)]
fn verify_fd_bound_digest(
    workspace_fd: i32,
    relative: &Path,
    expected: Option<String>,
    stage: &str,
) -> Result<(), String> {
    let opened = open_parent_from_workspace_fd(workspace_fd, relative).map_err(|error| {
        format!(
            "{stage} revalidation failed for {}: {error}",
            relative.display()
        )
    })?;
    let actual = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)
        .map_err(|error| {
            format!(
                "{stage} revalidation failed for {}: {error}",
                relative.display()
            )
        })?
        .map(|snapshot| snapshot.sha256);
    if actual != expected {
        return Err(format!(
            "{stage} revalidation detected projection drift: {}",
            relative.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn load_ownership_manifest_from_fd(workspace_fd: i32) -> Result<LoadedOwnership, String> {
    let bytes = read_projection_file_from_fd(
        workspace_fd,
        Path::new(OWNERSHIP_RELATIVE_PATH),
        MAX_ROLLBACK_BYTES,
        || Ok(()),
    )?;
    let Some(bytes) = bytes else {
        return Ok(LoadedOwnership {
            bytes: None,
            sha256: None,
            manifest: OwnershipManifest::default(),
        });
    };
    let digest = ags_platform::sha256(&bytes);
    let manifest: OwnershipManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid .ags/ownership-v2.json: {error}"))?;
    if manifest.schema_version != OWNERSHIP_SCHEMA {
        return Err(format!(
            "unsupported ownership metadata schema: {}",
            manifest.schema_version
        ));
    }
    for (relative, entry) in &manifest.entries {
        validate_relative(Path::new(relative))?;
        if normalized_relative_string(Path::new(relative))? != *relative {
            return Err(format!("ownership entry path is not canonical: {relative}"));
        }
        if relative == OWNERSHIP_RELATIVE_PATH {
            return Err("ownership metadata cannot own itself".to_string());
        }
        let payload = entry
            .last_applied_sha256
            .strip_prefix("sha256:")
            .unwrap_or_default();
        if payload.len() != 64
            || !payload
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "ownership entry has an invalid last-applied digest: {relative}"
            ));
        }
    }
    Ok(LoadedOwnership {
        bytes: Some(bytes),
        sha256: Some(digest),
        manifest,
    })
}

#[cfg(test)]
fn read_projection_file_inner(
    workspace: &Path,
    relative: &Path,
    before_read: impl FnOnce() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    #[cfg(unix)]
    {
        let workspace_fd = open_workspace(workspace)?;
        read_projection_file_from_fd(
            workspace_fd.as_raw_fd(),
            relative,
            MAX_ROLLBACK_BYTES,
            before_read,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (workspace, relative, before_read);
        Err("projection read is blocked: no audited fd-relative backend".to_string())
    }
}

#[cfg(unix)]
fn read_projection_file_from_fd(
    workspace_fd: i32,
    relative: &Path,
    limit: usize,
    before_open: impl FnOnce() -> Result<(), String>,
) -> Result<Option<Vec<u8>>, String> {
    let captured = inspect_projection_file_fd(workspace_fd, relative, Some(limit), before_open)?;
    let Some(captured) = captured else {
        return Ok(None);
    };
    if captured.exceeded_limit {
        return Err(format!(
            "risk-escalated: projection file exceeds audited read limit of {limit} bytes: {}",
            relative.display()
        ));
    }
    Ok(captured.bytes)
}

#[cfg(unix)]
fn planned_projection_directories_from_fd<I, P>(
    workspace_fd: i32,
    relative_files: I,
) -> Result<Vec<PathBuf>, String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    planned_projection_directories_from_fd_inner(workspace_fd, relative_files, || Ok(()), || Ok(()))
}

#[cfg(unix)]
fn planned_projection_directories_from_fd_inner<I, P>(
    workspace_fd: i32,
    relative_files: I,
    before_scan: impl FnOnce() -> Result<(), String>,
    after_scan: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<PathBuf>, String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    before_scan()?;
    let mut missing = BTreeSet::new();
    for relative in relative_files {
        let relative = relative.as_ref();
        validate_relative(relative)?;
        let mut parent = PathBuf::new();
        let mut current = duplicate_fd(workspace_fd)?;
        let mut ancestor_missing = false;
        let components = relative.components().collect::<Vec<_>>();
        for component in &components[..components.len() - 1] {
            let Component::Normal(component) = component else {
                unreachable!("validated projection component")
            };
            parent.push(component);
            if ancestor_missing {
                missing.insert(parent.clone());
                continue;
            }
            let name = cstring(component)?;
            match open_directory_at_optional(current.as_raw_fd(), &name)? {
                Some(opened) => current = opened,
                None => {
                    ancestor_missing = true;
                    missing.insert(parent.clone());
                }
            }
        }
    }
    after_scan()?;
    let mut directories = missing.into_iter().collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    Ok(directories)
}

#[cfg(unix)]
struct CreatedDirectory {
    parent_fd: OwnedFd,
    name: CString,
    identity: FileIdentity,
    relative: PathBuf,
}

#[cfg(unix)]
struct QuarantineState {
    fd: OwnedFd,
    identity: FileIdentity,
}

#[cfg(unix)]
fn create_projection_directory(
    workspace_fd: i32,
    relative: &Path,
) -> Result<CreatedDirectory, String> {
    create_projection_directory_with_after_mkdir(workspace_fd, relative, || Ok(()))
}

#[cfg(unix)]
fn create_projection_directory_with_after_mkdir(
    workspace_fd: i32,
    relative: &Path,
    after_mkdir: impl FnOnce() -> Result<(), String>,
) -> Result<CreatedDirectory, String> {
    validate_relative(relative)?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => cstring(value),
            _ => unreachable!("validated directory component"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = duplicate_fd(workspace_fd)?;
    for component in &components[..components.len() - 1] {
        current = open_directory_at(current.as_raw_fd(), component)?;
    }
    let name = components
        .last()
        .expect("validated directory has at least one component")
        .clone();
    // SAFETY: current is a retained directory descriptor and name is one
    // NUL-terminated component. mkdirat never follows a final symlink.
    if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o755) } != 0 {
        return Err(format!(
            "cannot create planned projection directory {}: {}",
            relative.display(),
            std::io::Error::last_os_error()
        ));
    }
    if let Err(error) = after_mkdir() {
        return Err(format!(
            "risk-escalated: created_directory_residue preserved after mkdir for {}: {error}",
            relative.display()
        ));
    }
    let created = match open_directory_at(current.as_raw_fd(), &name) {
        Ok(created) => created,
        Err(error) => {
            return Err(format!(
                "risk-escalated: created_directory_residue preserved because identity cannot be proven for {}: {error}",
                relative.display()
            ));
        }
    };
    let identity = identity_of_fd(created.as_raw_fd()).map_err(|error| {
        format!(
            "risk-escalated: created_directory_residue preserved because identity cannot be proven for {}: {error}",
            relative.display()
        )
    })?;
    Ok(CreatedDirectory {
        parent_fd: current,
        name,
        identity,
        relative: relative.to_path_buf(),
    })
}

#[cfg(unix)]
fn rollback_created_directories(
    directories: &[CreatedDirectory],
    state: &QuarantineState,
) -> Result<(), String> {
    rollback_created_directories_inner(directories, state, |_| Ok(()))
}

#[cfg(unix)]
fn rollback_created_directories_inner(
    directories: &[CreatedDirectory],
    state: &QuarantineState,
    before_move: impl FnMut(&CreatedDirectory) -> Result<(), String>,
) -> Result<(), String> {
    rollback_created_directories_inner_with_identity_probe(
        directories,
        state,
        before_move,
        |_, fd| identity_of_fd(fd),
    )
}

#[cfg(unix)]
fn rollback_created_directories_inner_with_identity_probe(
    directories: &[CreatedDirectory],
    state: &QuarantineState,
    mut before_move: impl FnMut(&CreatedDirectory) -> Result<(), String>,
    mut identity_probe: impl FnMut(&CreatedDirectory, i32) -> Result<FileIdentity, String>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    for directory in directories.iter().rev() {
        let current = match open_directory_at(directory.parent_fd.as_raw_fd(), &directory.name) {
            Ok(opened) => opened,
            Err(error) => {
                errors.push(format!(
                    "risk-escalated: created directory path changed before rollback {}: {error}",
                    directory.relative.display()
                ));
                continue;
            }
        };
        let current_identity = match identity_probe(directory, current.as_raw_fd()) {
            Ok(identity) => identity,
            Err(error) => {
                errors.push(format!(
                    "risk-escalated: created_directory_residue preserved because rollback identity cannot be proven for {}: {error}",
                    directory.relative.display()
                ));
                continue;
            }
        };
        if current_identity != directory.identity {
            errors.push(format!(
                "risk-escalated: created_directory_residue preserved because identity changed before rollback: {}",
                directory.relative.display()
            ));
            continue;
        }
        if let Err(error) = before_move(directory) {
            errors.push(error);
            continue;
        }
        match identity_probe(directory, current.as_raw_fd()) {
            Ok(identity) if identity == directory.identity => {}
            Ok(_) => {
                errors.push(format!(
                    "risk-escalated: created_directory_residue preserved because identity changed immediately before rollback: {}",
                    directory.relative.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "risk-escalated: created_directory_residue preserved because final rollback identity cannot be proven for {}: {error}",
                    directory.relative.display()
                ));
                continue;
            }
        }
        if directory.identity.device != state.identity.device {
            errors.push(format!(
                "before-effect blocked: created_directory_residue preserved because rollback crosses quarantine filesystem (EXDEV): {}",
                directory.relative.display()
            ));
            continue;
        }
        let (quarantine_name, quarantine_fd, quarantine_identity) = create_quarantine_at(state)?;
        let item = CString::new("directory").expect("static quarantine name has no NUL");
        if let Err(error) = rename_no_replace_between(
            directory.parent_fd.as_raw_fd(),
            &directory.name,
            quarantine_fd.as_raw_fd(),
            &item,
        ) {
            let _ = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
            errors.push(format!(
                "risk-escalated: cannot quarantine created directory {}: {error}",
                directory.relative.display()
            ));
            continue;
        }
        let moved = open_directory_at(quarantine_fd.as_raw_fd(), &item);
        let moved_matches = moved
            .as_ref()
            .ok()
            .and_then(|moved| identity_of_fd(moved.as_raw_fd()).ok())
            == Some(directory.identity);
        if !moved_matches {
            let restored = rename_no_replace_between(
                quarantine_fd.as_raw_fd(),
                &item,
                directory.parent_fd.as_raw_fd(),
                &directory.name,
            );
            if restored.is_err() {
                errors.push(format!(
                    "risk-escalated: substituted directory quarantined and restore failed: {}",
                    directory.relative.display()
                ));
            } else {
                let _ = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
                errors.push(format!(
                    "risk-escalated: directory substitution detected and restored: {}",
                    directory.relative.display()
                ));
            }
            continue;
        }
        // SAFETY: the only rmdir targets the identity-verified directory now
        // held under the private quarantine descriptor.
        if unsafe { libc::unlinkat(quarantine_fd.as_raw_fd(), item.as_ptr(), libc::AT_REMOVEDIR) }
            != 0
        {
            errors.push(format!(
                "cannot remove quarantined created directory {}: {}",
                directory.relative.display(),
                std::io::Error::last_os_error()
            ));
            continue;
        }
        if let Err(error) = remove_empty_quarantine(state, &quarantine_name, quarantine_identity) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(unix)]
fn join_rollback_error(error: String, rollback: Result<(), String>) -> String {
    match rollback {
        Ok(()) => error,
        Err(rollback) => format!("{error}; projection rollback also failed: {rollback}"),
    }
}

#[cfg(unix)]
pub fn plan_projection_migration(
    workspace: &Path,
    relative_path: &Path,
    recorded_owned_sha256: Option<&str>,
) -> Result<ProjectionMigration, String> {
    validate_relative(relative_path)?;
    let workspace = workspace.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize workspace {}: {error}",
            workspace.display()
        )
    })?;
    let workspace_fd = Arc::new(open_workspace(&workspace)?);
    plan_projection_migration_from_root(
        &workspace,
        workspace_fd,
        relative_path,
        recorded_owned_sha256,
    )
}

#[cfg(not(unix))]
pub fn plan_projection_migration(
    _workspace: &Path,
    _relative_path: &Path,
    _recorded_owned_sha256: Option<&str>,
) -> Result<ProjectionMigration, String> {
    Err("projection planning is blocked: no audited fd-relative backend".to_string())
}

#[cfg(unix)]
fn plan_projection_migration_from_root(
    workspace: &Path,
    workspace_fd: Arc<OwnedFd>,
    relative_path: &Path,
    recorded_owned_sha256: Option<&str>,
) -> Result<ProjectionMigration, String> {
    let workspace_identity = identity_of_fd(workspace_fd.as_raw_fd())?;
    let inspected = inspect_target_from_fd(workspace, workspace_fd.as_raw_fd(), relative_path)?;
    let disposition = classify(inspected.sha256.as_deref(), recorded_owned_sha256);
    Ok(ProjectionMigration {
        canonical_path: inspected.canonical_path,
        canonical_workspace: workspace.to_path_buf(),
        relative_path: relative_path.to_path_buf(),
        current_sha256: inspected.sha256,
        recorded_owned_sha256: recorded_owned_sha256.map(ToOwned::to_owned),
        disposition,
        #[cfg(unix)]
        parent_identity: inspected.parent_identity,
        #[cfg(unix)]
        workspace_fd,
        #[cfg(unix)]
        workspace_identity,
        #[cfg(unix)]
        target_identity: inspected.target_identity,
    })
}

/// Apply an opaque plan through a no-follow, descriptor-relative commit.
///
/// Unsupported platforms fail closed. On macOS and Linux, create uses an
/// atomic no-replace rename and replacement uses an atomic exchange. The
/// exchanged-out object is verified before it is removed; a substituted
/// symlink or file is exchanged back without following or modifying it.
pub fn apply_projection_migration(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
) -> Result<ProjectionMigrationReceipt, String> {
    apply_projection_migration_inner(plan, replacement_bytes, || Ok(()))
}

/// Recover one previously applied projection using the same descriptor-bound
/// identity rules. A created file is unlinked only if its current identity and
/// bytes still match the applied replacement; a replacement is exchanged back
/// only when the replacement hash is still exact.
#[cfg(unix)]
pub fn recover_projection_migration(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
    previous_bytes: Option<&[u8]>,
) -> Result<(), String> {
    recover_projection_migration_inner(plan, replacement_bytes, previous_bytes, || Ok(()))
}

#[cfg(unix)]
fn recover_projection_migration_inner(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
    previous_bytes: Option<&[u8]>,
    before_unlink: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let quarantine_state = establish_quarantine_state(plan.workspace_fd.as_raw_fd())?;
    recover_projection_migration_with_state(
        plan,
        &quarantine_state,
        replacement_bytes,
        previous_bytes,
        before_unlink,
    )
}

#[cfg(unix)]
fn recover_projection_migration_with_state(
    plan: &ProjectionMigration,
    quarantine_state: &QuarantineState,
    replacement_bytes: &[u8],
    previous_bytes: Option<&[u8]>,
    before_unlink: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let replacement_sha256 = ags_platform::sha256(replacement_bytes);
    if let Some(previous) = previous_bytes {
        let recovery = plan_projection_migration_from_root(
            &plan.canonical_workspace,
            Arc::clone(&plan.workspace_fd),
            &plan.relative_path,
            Some(&replacement_sha256),
        )?;
        let receipt = apply_projection_migration_with_state(&recovery, quarantine_state, previous)?;
        if !receipt.changed {
            return Err("projection recovery did not restore previous bytes".to_string());
        }
        return Ok(());
    }

    let opened = open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)?;
    if opened.identity != plan.parent_identity {
        return Err("projection recovery parent identity changed".to_string());
    }
    let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)?
        .ok_or_else(|| "created projection disappeared before recovery".to_string())?;
    if current.sha256 != replacement_sha256 {
        return Err("created projection bytes changed before recovery".to_string());
    }
    quarantine_then_delete(
        quarantine_state,
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        opened.identity,
        current.identity,
        &replacement_sha256,
        before_unlink,
    )?;
    verify_parent_binding_from_root(
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        plan.parent_identity,
    )?;
    sync_directory(opened.fd.as_raw_fd())
}

#[cfg(not(unix))]
pub fn recover_projection_migration(
    _plan: &ProjectionMigration,
    _replacement_bytes: &[u8],
    _previous_bytes: Option<&[u8]>,
) -> Result<(), String> {
    Err("projection recovery is blocked: no audited fd-relative backend".to_string())
}

#[cfg(all(test, unix))]
fn delete_exact_projection_inner(
    plan: &ProjectionMigration,
    before_unlink: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let quarantine_state = establish_quarantine_state(plan.workspace_fd.as_raw_fd())?;
    delete_exact_projection_with_state(plan, &quarantine_state, before_unlink)
}

#[cfg(unix)]
fn delete_exact_projection_with_state(
    plan: &ProjectionMigration,
    quarantine_state: &QuarantineState,
    before_unlink: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if plan.disposition != MigrationDisposition::ReclaimExactOwned {
        return Err("projection deletion requires exact last-applied ownership".to_string());
    }
    verify_workspace_path_binding(
        &plan.canonical_workspace,
        plan.workspace_fd.as_raw_fd(),
        plan.workspace_identity,
    )?;
    let opened = open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)?;
    if opened.identity != plan.parent_identity {
        return Err("projection deletion parent identity changed".to_string());
    }
    let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)?
        .ok_or_else(|| "projection deletion target disappeared".to_string())?;
    ensure_snapshot_matches_plan(plan, Some(&current))?;
    quarantine_then_delete(
        quarantine_state,
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        opened.identity,
        current.identity,
        &current.sha256,
        before_unlink,
    )?;
    verify_parent_binding_from_root(
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        opened.identity,
    )?;
    sync_directory(opened.fd.as_raw_fd())
}

#[cfg(unix)]
enum AtomicEffect {
    Created {
        staged_identity: FileIdentity,
        staged_sha256: String,
    },
    Exchanged {
        staged_name: CString,
        staged_identity: FileIdentity,
        staged_sha256: String,
        old_identity: FileIdentity,
        old_sha256: String,
    },
}

#[cfg(unix)]
struct AtomicEffectGuard<'a> {
    plan: &'a ProjectionMigration,
    quarantine_state: &'a QuarantineState,
    parent_fd: i32,
    parent_identity: FileIdentity,
    target_name: &'a CString,
    effect: AtomicEffect,
}

#[cfg(unix)]
impl AtomicEffectGuard<'_> {
    fn recover(self, cause: String) -> String {
        let recovery = match self.effect {
            AtomicEffect::Created {
                staged_identity,
                staged_sha256,
            } => {
                let current = inspect_at(self.parent_fd, self.target_name).and_then(|snapshot| {
                    snapshot.ok_or_else(|| {
                        "created projection disappeared before guarded recovery".to_string()
                    })
                });
                match current {
                    Ok(snapshot)
                        if snapshot.identity == staged_identity
                            && snapshot.sha256 == staged_sha256 =>
                    {
                        quarantine_then_delete(
                            self.quarantine_state,
                            self.plan.workspace_fd.as_raw_fd(),
                            &self.plan.relative_path,
                            self.parent_identity,
                            staged_identity,
                            &staged_sha256,
                            || Ok(()),
                        )
                    }
                    Ok(_) => Err(
                        "guarded create recovery refused an unknown target inode or digest"
                            .to_string(),
                    ),
                    Err(error) => Err(error),
                }
            }
            AtomicEffect::Exchanged {
                staged_name,
                staged_identity,
                staged_sha256,
                old_identity,
                old_sha256,
            } => {
                let target = inspect_at(self.parent_fd, self.target_name);
                let old_name = inspect_at(self.parent_fd, &staged_name);
                let exact_pair = matches!(
                    (target.as_ref(), old_name.as_ref()),
                    (Ok(Some(target)), Ok(Some(old)))
                        if target.identity == staged_identity
                            && target.sha256 == staged_sha256
                            && old.identity == old_identity
                            && old.sha256 == old_sha256
                );
                if !exact_pair {
                    Err(
                        "guarded exchange recovery refused unknown target or old-name inode/digest"
                            .to_string(),
                    )
                } else if let Err(error) = verify_parent_binding_from_root(
                    self.plan.workspace_fd.as_raw_fd(),
                    &self.plan.relative_path,
                    self.parent_identity,
                ) {
                    Err(error)
                } else if let Err(error) =
                    rename_exchange(self.parent_fd, &staged_name, self.target_name)
                {
                    Err(error)
                } else {
                    let restored_target = inspect_at(self.parent_fd, self.target_name);
                    let staged_at_temp = inspect_at(self.parent_fd, &staged_name);
                    let exact_restored_pair = matches!(
                        (restored_target.as_ref(), staged_at_temp.as_ref()),
                        (Ok(Some(target)), Ok(Some(staged)))
                            if target.identity == old_identity
                                && target.sha256 == old_sha256
                                && staged.identity == staged_identity
                                && staged.sha256 == staged_sha256
                    );
                    if !exact_restored_pair {
                        Err(
                            "guarded exchange recovery found unknown objects after exchange"
                                .to_string(),
                        )
                    } else {
                        sibling_relative_path(&self.plan.relative_path, &staged_name).and_then(
                            |staged_relative| {
                                quarantine_then_delete(
                                    self.quarantine_state,
                                    self.plan.workspace_fd.as_raw_fd(),
                                    &staged_relative,
                                    self.parent_identity,
                                    staged_identity,
                                    &staged_sha256,
                                    || Ok(()),
                                )
                            },
                        )
                    }
                }
            }
        };
        match recovery {
            Ok(()) => cause,
            Err(error) => format!(
                "{cause}; risk-escalated: atomic projection effect recovery failed: {error}"
            ),
        }
    }

    fn disarm(self) {}
}

#[cfg(unix)]
fn sibling_relative_path(relative: &Path, name: &CString) -> Result<PathBuf, String> {
    let name = OsStr::from_bytes(name.as_bytes());
    let mut sibling = relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    sibling.push(name);
    validate_relative(&sibling)?;
    Ok(sibling)
}

#[cfg(unix)]
fn verify_staged_name(
    parent_fd: i32,
    temp_name: &CString,
    staged_identity: FileIdentity,
    staged_sha256: &str,
) -> Result<(), String> {
    let current = inspect_at(parent_fd, temp_name)?
        .ok_or_else(|| "staged projection public name disappeared".to_string())?;
    if current.identity != staged_identity || current.sha256 != staged_sha256 {
        return Err(
            "staged projection public name resolves to an unknown inode or digest".to_string(),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn cleanup_staged_name(
    plan: &ProjectionMigration,
    quarantine_state: &QuarantineState,
    parent_identity: FileIdentity,
    temp_name: &CString,
    staged_identity: FileIdentity,
    staged_sha256: &str,
    cause: String,
) -> String {
    let cleanup = sibling_relative_path(&plan.relative_path, temp_name).and_then(|relative| {
        quarantine_then_delete(
            quarantine_state,
            plan.workspace_fd.as_raw_fd(),
            &relative,
            parent_identity,
            staged_identity,
            staged_sha256,
            || Ok(()),
        )
    });
    match cleanup {
        Ok(()) => cause,
        Err(error) => {
            format!("{cause}; risk-escalated: staged projection cleanup refused or failed: {error}")
        }
    }
}

#[cfg(unix)]
fn apply_projection_migration_inner(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
    before_commit: impl FnOnce() -> Result<(), String>,
) -> Result<ProjectionMigrationReceipt, String> {
    apply_projection_migration_inner_with_after(plan, replacement_bytes, before_commit, || Ok(()))
}

#[cfg(unix)]
fn apply_projection_migration_inner_with_after(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
    before_commit: impl FnOnce() -> Result<(), String>,
    after_atomic_effect: impl FnOnce() -> Result<(), String>,
) -> Result<ProjectionMigrationReceipt, String> {
    apply_projection_migration_inner_with_faults(
        plan,
        replacement_bytes,
        before_commit,
        || Ok(()),
        after_atomic_effect,
        || Ok(()),
    )
}

#[cfg(unix)]
fn apply_projection_migration_inner_with_faults(
    plan: &ProjectionMigration,
    replacement_bytes: &[u8],
    before_commit: impl FnOnce() -> Result<(), String>,
    before_file_fsync: impl FnOnce() -> Result<(), String>,
    after_atomic_effect: impl FnOnce() -> Result<(), String>,
    before_parent_fsync: impl FnOnce() -> Result<(), String>,
) -> Result<ProjectionMigrationReceipt, String> {
    let quarantine_state = establish_quarantine_state(plan.workspace_fd.as_raw_fd())?;
    apply_projection_migration_inner_with_state_and_faults(
        plan,
        &quarantine_state,
        replacement_bytes,
        before_commit,
        before_file_fsync,
        after_atomic_effect,
        before_parent_fsync,
    )
}

#[cfg(unix)]
fn apply_projection_migration_with_state(
    plan: &ProjectionMigration,
    quarantine_state: &QuarantineState,
    replacement_bytes: &[u8],
) -> Result<ProjectionMigrationReceipt, String> {
    apply_projection_migration_inner_with_state_and_faults(
        plan,
        quarantine_state,
        replacement_bytes,
        || Ok(()),
        || Ok(()),
        || Ok(()),
        || Ok(()),
    )
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn apply_projection_migration_inner_with_state_and_faults(
    plan: &ProjectionMigration,
    quarantine_state: &QuarantineState,
    replacement_bytes: &[u8],
    before_commit: impl FnOnce() -> Result<(), String>,
    before_file_fsync: impl FnOnce() -> Result<(), String>,
    after_atomic_effect: impl FnOnce() -> Result<(), String>,
    before_parent_fsync: impl FnOnce() -> Result<(), String>,
) -> Result<ProjectionMigrationReceipt, String> {
    if !conditional_rename_supported() {
        return Err(
            "projection apply is blocked: this Unix platform has no audited conditional rename backend"
                .to_string(),
        );
    }
    verify_workspace_path_binding(
        &plan.canonical_workspace,
        plan.workspace_fd.as_raw_fd(),
        plan.workspace_identity,
    )?;

    let opened = open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)?;
    if opened.identity != plan.parent_identity {
        return Err("projection parent identity changed after planning".to_string());
    }
    let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)?;
    ensure_snapshot_matches_plan(plan, current.as_ref())?;

    let recomputed = classify(
        current.as_ref().map(|snapshot| snapshot.sha256.as_str()),
        plan.recorded_owned_sha256.as_deref(),
    );
    if recomputed != plan.disposition {
        return Err(format!(
            "projection plan ownership disposition was tampered: planned={:?}, recomputed={recomputed:?}",
            plan.disposition
        ));
    }

    if matches!(
        recomputed,
        MigrationDisposition::PreserveUnowned | MigrationDisposition::PreserveModified
    ) {
        return Ok(receipt(
            plan,
            recomputed,
            plan.current_sha256.clone(),
            false,
        ));
    }

    let (temp_name, mut temp_file, staged_identity) = create_temp_at(opened.fd.as_raw_fd())?;
    let replacement_sha256 = ags_platform::sha256(replacement_bytes);
    let write_result = temp_file
        .write_all(replacement_bytes)
        .and_then(|_| temp_file.flush())
        .map_err(|error| format!("cannot stage projection replacement: {error}"))
        .and_then(|_| before_file_fsync())
        .and_then(|_| {
            temp_file
                .sync_all()
                .map_err(|error| format!("cannot fsync staged projection replacement: {error}"))
        })
        .and_then(|_| {
            let current_identity = identity_of_fd(temp_file.as_raw_fd())?;
            if current_identity != staged_identity {
                return Err("staged projection descriptor identity changed".to_string());
            }
            Ok(())
        });
    drop(temp_file);
    if let Err(error) = write_result {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged_identity,
            &replacement_sha256,
            error,
        ));
    }
    let staged = TargetSnapshot {
        identity: staged_identity,
        sha256: replacement_sha256.clone(),
    };
    if let Err(error) = verify_staged_name(
        opened.fd.as_raw_fd(),
        &temp_name,
        staged.identity,
        &staged.sha256,
    ) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }

    if let Err(error) = verify_parent_binding_from_root(
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        opened.identity,
    ) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }
    let immediately_before = match inspect_at(opened.fd.as_raw_fd(), &opened.file_name) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(cleanup_staged_name(
                plan,
                quarantine_state,
                opened.identity,
                &temp_name,
                staged.identity,
                &staged.sha256,
                error,
            ));
        }
    };
    if let Err(error) = ensure_snapshot_matches_plan(plan, immediately_before.as_ref()) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }
    if let Err(error) = before_commit() {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }
    if let Err(error) = verify_parent_binding_from_root(
        plan.workspace_fd.as_raw_fd(),
        &plan.relative_path,
        opened.identity,
    ) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }
    let final_before_commit = match inspect_at(opened.fd.as_raw_fd(), &opened.file_name) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(cleanup_staged_name(
                plan,
                quarantine_state,
                opened.identity,
                &temp_name,
                staged.identity,
                &staged.sha256,
                error,
            ));
        }
    };
    if let Err(error) = ensure_snapshot_matches_plan(plan, final_before_commit.as_ref()) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }
    if let Err(error) = verify_staged_name(
        opened.fd.as_raw_fd(),
        &temp_name,
        staged.identity,
        &staged.sha256,
    ) {
        return Err(cleanup_staged_name(
            plan,
            quarantine_state,
            opened.identity,
            &temp_name,
            staged.identity,
            &staged.sha256,
            error,
        ));
    }

    match recomputed {
        MigrationDisposition::Create => {
            if let Err(error) =
                rename_no_replace(opened.fd.as_raw_fd(), &temp_name, &opened.file_name)
            {
                return Err(cleanup_staged_name(
                    plan,
                    quarantine_state,
                    opened.identity,
                    &temp_name,
                    staged.identity,
                    &staged.sha256,
                    format!("projection create target changed during final commit: {error}"),
                ));
            }
            let guard = AtomicEffectGuard {
                plan,
                quarantine_state,
                parent_fd: opened.fd.as_raw_fd(),
                parent_identity: opened.identity,
                target_name: &opened.file_name,
                effect: AtomicEffect::Created {
                    staged_identity: staged.identity,
                    staged_sha256: staged.sha256.clone(),
                },
            };
            if let Err(error) = after_atomic_effect() {
                return Err(guard.recover(error));
            }
            let result = match inspect_at(opened.fd.as_raw_fd(), &opened.file_name) {
                Ok(Some(result)) => result,
                Ok(None) => {
                    return Err(
                        guard.recover("projection create vanished after commit".to_string())
                    );
                }
                Err(error) => return Err(guard.recover(error)),
            };
            if result.identity != staged.identity
                || result.sha256 != staged.sha256
                || result.sha256 != replacement_sha256
                || verify_parent_binding_from_root(
                    plan.workspace_fd.as_raw_fd(),
                    &plan.relative_path,
                    opened.identity,
                )
                .is_err()
            {
                return Err(guard.recover(
                    "projection create identity changed during final commit".to_string(),
                ));
            }
            if let Err(error) = before_parent_fsync() {
                return Err(guard.recover(error));
            }
            if let Err(error) = sync_directory(opened.fd.as_raw_fd()) {
                return Err(guard.recover(error));
            }
            guard.disarm();
            Ok(receipt(plan, recomputed, Some(result.sha256), true))
        }
        MigrationDisposition::ReclaimExactOwned => {
            if let Err(error) =
                rename_exchange(opened.fd.as_raw_fd(), &temp_name, &opened.file_name)
            {
                return Err(cleanup_staged_name(
                    plan,
                    quarantine_state,
                    opened.identity,
                    &temp_name,
                    staged.identity,
                    &staged.sha256,
                    format!("projection target changed during final exchange: {error}"),
                ));
            }

            let guard = AtomicEffectGuard {
                plan,
                quarantine_state,
                parent_fd: opened.fd.as_raw_fd(),
                parent_identity: opened.identity,
                target_name: &opened.file_name,
                effect: AtomicEffect::Exchanged {
                    staged_name: temp_name.clone(),
                    staged_identity: staged.identity,
                    staged_sha256: staged.sha256.clone(),
                    old_identity: immediately_before
                        .as_ref()
                        .expect("replacement disposition has an existing target")
                        .identity,
                    old_sha256: immediately_before
                        .as_ref()
                        .expect("replacement disposition has an existing target")
                        .sha256
                        .clone(),
                },
            };
            if let Err(error) = after_atomic_effect() {
                return Err(guard.recover(error));
            }

            let exchanged_out = inspect_at(opened.fd.as_raw_fd(), &temp_name);
            let expected_old = exchanged_out.as_ref().ok().and_then(Option::as_ref);
            let old_matches = expected_old.is_some_and(|snapshot| {
                Some(snapshot.identity) == plan.target_identity
                    && Some(snapshot.sha256.as_str()) == plan.current_sha256.as_deref()
            });
            let result = inspect_at(opened.fd.as_raw_fd(), &opened.file_name);
            let parent_stable = verify_parent_binding_from_root(
                plan.workspace_fd.as_raw_fd(),
                &plan.relative_path,
                opened.identity,
            )
            .is_ok();
            let replacement_matches =
                result
                    .as_ref()
                    .ok()
                    .and_then(Option::as_ref)
                    .is_some_and(|snapshot| {
                        snapshot.identity == staged.identity
                            && snapshot.sha256 == staged.sha256
                            && snapshot.sha256 == replacement_sha256
                    });

            if !old_matches || !replacement_matches || !parent_stable {
                return Err(guard.recover(
                    "projection substitution detected during final exchange; original path restored"
                        .to_string(),
                ));
            }

            if let Err(error) = before_parent_fsync() {
                return Err(guard.recover(error));
            }
            if let Err(error) = sync_directory(opened.fd.as_raw_fd()) {
                return Err(guard.recover(error));
            }
            let old_relative = sibling_relative_path(&plan.relative_path, &temp_name)?;
            let old = expected_old.expect("old object inspection succeeded and matched");
            if let Err(error) = quarantine_then_delete(
                quarantine_state,
                plan.workspace_fd.as_raw_fd(),
                &old_relative,
                opened.identity,
                old.identity,
                &old.sha256,
                || Ok(()),
            ) {
                return Err(guard.recover(error));
            }
            guard.disarm();
            if let Err(error) = sync_directory(opened.fd.as_raw_fd()) {
                return Err(format!(
                    "risk-escalated: replacement committed but parent fsync failed after old bytes were unlinked: {error}"
                ));
            }
            let result = result
                .expect("replacement inspection succeeded")
                .expect("replacement exists");
            Ok(receipt(plan, recomputed, Some(result.sha256), true))
        }
        MigrationDisposition::PreserveUnowned | MigrationDisposition::PreserveModified => {
            unreachable!("preserve dispositions returned before staging")
        }
    }
}

#[cfg(not(unix))]
fn apply_projection_migration_inner(
    _plan: &ProjectionMigration,
    _replacement_bytes: &[u8],
    _before_commit: impl FnOnce() -> Result<(), String>,
) -> Result<ProjectionMigrationReceipt, String> {
    Err(
        "projection apply is blocked: no audited fd-relative no-follow backend is available on this platform"
            .to_string(),
    )
}

#[cfg(unix)]
fn receipt(
    plan: &ProjectionMigration,
    disposition: MigrationDisposition,
    result_sha256: Option<String>,
    changed: bool,
) -> ProjectionMigrationReceipt {
    ProjectionMigrationReceipt {
        schema_version: "ags://schema/contract/v2/projection-migration-receipt".to_string(),
        canonical_path: plan.canonical_path.to_string_lossy().into_owned(),
        disposition,
        previous_sha256: plan.current_sha256.clone(),
        result_sha256,
        changed,
    }
}

#[cfg(unix)]
fn classify(
    current_sha256: Option<&str>,
    recorded_owned_sha256: Option<&str>,
) -> MigrationDisposition {
    match (current_sha256, recorded_owned_sha256) {
        (None, _) => MigrationDisposition::Create,
        (Some(actual), Some(expected)) if actual == expected => {
            MigrationDisposition::ReclaimExactOwned
        }
        (Some(_), Some(_)) => MigrationDisposition::PreserveModified,
        (Some(_), None) => MigrationDisposition::PreserveUnowned,
    }
}

#[cfg(unix)]
fn validate_relative(relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(format!(
            "projection path must be a non-empty relative path: {}",
            relative.display()
        ));
    }
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "projection path must contain only normal components: {}",
            relative.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct InspectedTarget {
    canonical_path: PathBuf,
    sha256: Option<String>,
    parent_identity: FileIdentity,
    target_identity: Option<FileIdentity>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
#[derive(Debug)]
struct TargetSnapshot {
    identity: FileIdentity,
    sha256: String,
}

#[cfg(unix)]
struct CapturedTarget {
    snapshot: TargetSnapshot,
    bytes: Option<Vec<u8>>,
    exceeded_limit: bool,
}

#[cfg(unix)]
struct OpenedParent {
    fd: OwnedFd,
    identity: FileIdentity,
    file_name: CString,
}

#[cfg(unix)]
fn inspect_target_from_fd(
    workspace: &Path,
    workspace_fd: i32,
    relative: &Path,
) -> Result<InspectedTarget, String> {
    let opened = open_parent_from_workspace_fd(workspace_fd, relative)?;
    let target = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)?;
    Ok(InspectedTarget {
        canonical_path: workspace.join(relative),
        sha256: target.as_ref().map(|snapshot| snapshot.sha256.clone()),
        parent_identity: opened.identity,
        target_identity: target.map(|snapshot| snapshot.identity),
    })
}

#[cfg(unix)]
fn open_workspace(workspace: &Path) -> Result<OwnedFd, String> {
    let workspace_name = cstring(workspace.as_os_str())?;
    // SAFETY: O_NOFOLLOW and O_DIRECTORY reject a changed symlink binding.
    let raw_fd = unsafe {
        libc::open(
            workspace_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw_fd < 0 {
        return Err(format!(
            "cannot open canonical workspace without following links: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: open returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
}

#[cfg(unix)]
fn verify_workspace_path_binding(
    workspace: &Path,
    retained_fd: i32,
    expected: FileIdentity,
) -> Result<(), String> {
    if identity_of_fd(retained_fd)? != expected {
        return Err("project projection retained workspace identity changed".to_string());
    }
    let reopened = open_workspace(workspace)
        .map_err(|error| format!("project projection workspace path binding changed: {error}"))?;
    if identity_of_fd(reopened.as_raw_fd())? != expected {
        return Err("project projection workspace path binding changed after planning".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn duplicate_fd(fd: i32) -> Result<OwnedFd, String> {
    // SAFETY: fcntl duplicates a live descriptor with close-on-exec.
    let raw_fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if raw_fd < 0 {
        return Err(format!(
            "cannot duplicate projection directory descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fcntl returned a new owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
}

#[cfg(unix)]
fn open_directory_at(directory_fd: i32, name: &CString) -> Result<OwnedFd, String> {
    open_directory_at_optional(directory_fd, name)?.ok_or_else(|| {
        "projection parent component cannot be opened without following links: not found"
            .to_string()
    })
}

#[cfg(unix)]
fn open_directory_at_optional(
    directory_fd: i32,
    name: &CString,
) -> Result<Option<OwnedFd>, String> {
    // SAFETY: name is one NUL-terminated component; O_NOFOLLOW and O_DIRECTORY
    // reject symlinks and non-directories.
    let raw_fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if raw_fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOENT) {
            return Ok(None);
        }
        return Err(format!(
            "projection parent component cannot be opened without following links: {}",
            error
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(Some(unsafe { OwnedFd::from_raw_fd(raw_fd) }))
}

#[cfg(unix)]
fn open_parent_from_workspace_fd(
    workspace_fd: i32,
    relative: &Path,
) -> Result<OpenedParent, String> {
    validate_relative(relative)?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err("projection path contains a non-normal component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let file_name = cstring(
        components
            .last()
            .ok_or_else(|| "projection path has no file name".to_string())?,
    )?;
    let mut current = duplicate_fd(workspace_fd)?;
    for component in &components[..components.len() - 1] {
        let name = cstring(component)?;
        current = open_directory_at(current.as_raw_fd(), &name)?;
    }
    let identity = identity_of_fd(current.as_raw_fd())?;
    Ok(OpenedParent {
        fd: current,
        identity,
        file_name,
    })
}

#[cfg(unix)]
fn open_parent_optional_from_workspace_fd(
    workspace_fd: i32,
    relative: &Path,
) -> Result<Option<OpenedParent>, String> {
    validate_relative(relative)?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err("projection path contains a non-normal component".to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let file_name = cstring(
        components
            .last()
            .ok_or_else(|| "projection path has no file name".to_string())?,
    )?;
    let mut current = duplicate_fd(workspace_fd)?;
    for component in &components[..components.len() - 1] {
        let name = cstring(component)?;
        let Some(next) = open_directory_at_optional(current.as_raw_fd(), &name)? else {
            return Ok(None);
        };
        current = next;
    }
    let identity = identity_of_fd(current.as_raw_fd())?;
    Ok(Some(OpenedParent {
        fd: current,
        identity,
        file_name,
    }))
}

#[cfg(unix)]
fn verify_parent_binding_from_root(
    workspace_fd: i32,
    relative: &Path,
    expected: FileIdentity,
) -> Result<(), String> {
    let reopened = open_parent_from_workspace_fd(workspace_fd, relative)?;
    if reopened.identity != expected {
        return Err("projection parent path was renamed or substituted".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn inspect_at(directory_fd: i32, name: &CString) -> Result<Option<TargetSnapshot>, String> {
    inspect_at_captured(directory_fd, name, None)
        .map(|captured| captured.map(|captured| captured.snapshot))
}

#[cfg(unix)]
fn inspect_projection_file_fd(
    workspace_fd: i32,
    relative: &Path,
    capture_limit: Option<usize>,
    before_open: impl FnOnce() -> Result<(), String>,
) -> Result<Option<CapturedTarget>, String> {
    let Some(opened) = open_parent_optional_from_workspace_fd(workspace_fd, relative)? else {
        return Ok(None);
    };
    before_open()?;
    let captured = inspect_at_captured(opened.fd.as_raw_fd(), &opened.file_name, capture_limit)?;
    let rebound =
        open_parent_optional_from_workspace_fd(workspace_fd, relative).map_err(|error| {
            format!(
                "projection parent binding changed during descriptor read {}: {error}",
                relative.display()
            )
        })?;
    let rebound = rebound.ok_or_else(|| {
        format!(
            "projection parent binding disappeared during descriptor read: {}",
            relative.display()
        )
    })?;
    if rebound.identity != opened.identity {
        return Err(format!(
            "projection parent binding changed during descriptor read: {}",
            relative.display()
        ));
    }
    Ok(captured)
}

#[cfg(unix)]
fn inspect_at_captured(
    directory_fd: i32,
    name: &CString,
    capture_limit: Option<usize>,
) -> Result<Option<CapturedTarget>, String> {
    // SAFETY: directory_fd is open and name is a NUL-terminated single path
    // component. O_NOFOLLOW makes a final symlink an error, never a target.
    let raw_fd = unsafe {
        libc::openat(
            directory_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if raw_fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOENT) {
            return Ok(None);
        }
        return Err(format!(
            "projection target cannot be opened without following links: {error}"
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let identity = identity_of_fd(owned.as_raw_fd())?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage and owned is a live descriptor.
    if unsafe { libc::fstat(owned.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "cannot inspect projection target descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fstat succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err("projection target must be a regular file".to_string());
    }
    let mut file = std::fs::File::from(owned);
    let mut hasher = Sha256::new();
    let mut bytes = capture_limit.map(|limit| Vec::with_capacity(limit.min(64 * 1024)));
    let mut exceeded_limit = false;
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| format!("cannot hash projection target descriptor: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        if let (Some(limit), Some(captured)) = (capture_limit, bytes.as_mut()) {
            if captured.len().saturating_add(read) <= limit {
                captured.extend_from_slice(&chunk[..read]);
            } else {
                exceeded_limit = true;
                bytes = None;
            }
        }
    }
    Ok(Some(CapturedTarget {
        snapshot: TargetSnapshot {
            identity,
            sha256: format!("sha256:{:x}", hasher.finalize()),
        },
        bytes,
        exceeded_limit,
    }))
}

#[cfg(unix)]
fn ensure_snapshot_matches_plan(
    plan: &ProjectionMigration,
    current: Option<&TargetSnapshot>,
) -> Result<(), String> {
    let identity = current.map(|snapshot| snapshot.identity);
    let sha256 = current.map(|snapshot| snapshot.sha256.as_str());
    if identity != plan.target_identity || sha256 != plan.current_sha256.as_deref() {
        return Err("projection target identity or bytes changed after planning".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn identity_of_fd(fd: i32) -> Result<FileIdentity, String> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage and fd is a live descriptor.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "cannot inspect projection descriptor identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fstat succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    Ok(FileIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
    })
}

#[cfg(unix)]
fn cstring(value: &OsStr) -> Result<CString, String> {
    CString::new(value.as_bytes()).map_err(|_| "projection path contains a NUL byte".to_string())
}

#[cfg(unix)]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn create_temp_at(directory_fd: i32) -> Result<(CString, std::fs::File, FileIdentity), String> {
    for _ in 0..64 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = CString::new(format!(
            ".ags-projection-{}-{sequence}.tmp",
            std::process::id()
        ))
        .expect("generated temp name has no NUL");
        // SAFETY: directory_fd is open and name is a valid NUL-terminated
        // component. O_EXCL and O_NOFOLLOW prevent substitution.
        let fd = unsafe {
            libc::openat(
                directory_fd,
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if fd >= 0 {
            // SAFETY: openat returned a new owned descriptor.
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            let identity = identity_of_fd(file.as_raw_fd()).map_err(|error| {
                format!(
                    "risk-escalated: cannot prove newly created staged inode; preserved {}: {error}",
                    name.to_string_lossy()
                )
            })?;
            return Ok((name, file, identity));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(format!("cannot create projection temp file: {error}"));
        }
    }
    Err("cannot allocate a unique projection temp file".to_string())
}

#[cfg(unix)]
fn unlink_validated_private_file(directory_fd: i32, name: &CString) -> Result<(), String> {
    // SAFETY: directory_fd is open and name is a NUL-terminated component.
    if unsafe { libc::unlinkat(directory_fd, name.as_ptr(), 0) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "cannot remove projection temp file: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(unix)]
fn quarantine_then_delete(
    state: &QuarantineState,
    workspace_fd: i32,
    relative: &Path,
    expected_parent: FileIdentity,
    expected_identity: FileIdentity,
    expected_sha256: &str,
    before_move: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    quarantine_then_delete_inner(
        state,
        workspace_fd,
        relative,
        expected_parent,
        expected_identity,
        expected_sha256,
        before_move,
        || Ok(()),
        || Ok(()),
        || Ok(()),
        || Ok(()),
    )
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn quarantine_then_delete_inner(
    state: &QuarantineState,
    workspace_fd: i32,
    relative: &Path,
    expected_parent: FileIdentity,
    expected_identity: FileIdentity,
    expected_sha256: &str,
    before_move: impl FnOnce() -> Result<(), String>,
    after_move: impl FnOnce() -> Result<(), String>,
    before_unlink: impl FnOnce() -> Result<(), String>,
    before_quarantine_fsync: impl FnOnce() -> Result<(), String>,
    before_parent_fsync: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let source = open_parent_from_workspace_fd(workspace_fd, relative)?;
    if source.identity != expected_parent {
        return Err("projection quarantine parent identity changed".to_string());
    }
    if source.identity.device != state.identity.device {
        return Err(
            "before-effect blocked: projection quarantine source crosses state filesystem (EXDEV)"
                .to_string(),
        );
    }
    if !conditional_rename_supported() {
        return Err(
            "before-effect blocked: projection quarantine rename is unsupported (ENOTSUP)"
                .to_string(),
        );
    }
    let current = inspect_at(source.fd.as_raw_fd(), &source.file_name)?
        .ok_or_else(|| "projection quarantine target disappeared".to_string())?;
    if current.identity != expected_identity || current.sha256 != expected_sha256 {
        return Err("projection quarantine refused changed target bytes or identity".to_string());
    }

    let (quarantine_name, quarantine_fd, quarantine_identity) = create_quarantine_at(state)?;
    let item_name = CString::new("item").expect("static quarantine item name has no NUL");
    if let Err(error) = before_move() {
        let _ = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
        return Err(format!(
            "before-effect blocked: projection quarantine compatibility check failed: {error}"
        ));
    }
    if let Err(error) = rename_no_replace_between(
        source.fd.as_raw_fd(),
        &source.file_name,
        quarantine_fd.as_raw_fd(),
        &item_name,
    ) {
        let _ = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
        return Err(format!(
            "before-effect blocked: projection quarantine move was not committed (EXDEV/ENOTSUP or substitution): {error}"
        ));
    }
    if let Err(error) = after_move() {
        let restore = rename_no_replace_between(
            quarantine_fd.as_raw_fd(),
            &item_name,
            source.fd.as_raw_fd(),
            &source.file_name,
        );
        let cleanup = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
        return Err(join_rollback_error(error, restore.and(cleanup)));
    }

    let moved = match inspect_at(quarantine_fd.as_raw_fd(), &item_name) {
        Ok(Some(moved)) => moved,
        Ok(None) => {
            return Err(
                "risk-escalated: projection quarantine item vanished after atomic move".to_string(),
            );
        }
        Err(error) => {
            let restore = rename_no_replace_between(
                quarantine_fd.as_raw_fd(),
                &item_name,
                source.fd.as_raw_fd(),
                &source.file_name,
            );
            let cleanup = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
            return Err(join_rollback_error(error, restore.and(cleanup)));
        }
    };
    if moved.identity != expected_identity || moved.sha256 != expected_sha256 {
        let restored = rename_no_replace_between(
            quarantine_fd.as_raw_fd(),
            &item_name,
            source.fd.as_raw_fd(),
            &source.file_name,
        );
        if let Err(error) = restored {
            return Err(format!(
                "risk-escalated: projection substitution quarantined but safe restore failed: {error}; quarantine={}",
                quarantine_name.to_string_lossy()
            ));
        }
        remove_empty_quarantine(state, &quarantine_name, quarantine_identity)?;
        return Err(
            "projection substitution detected after quarantine move; substituted bytes restored"
                .to_string(),
        );
    }

    if let Err(error) = before_unlink() {
        let restore = rename_no_replace_between(
            quarantine_fd.as_raw_fd(),
            &item_name,
            source.fd.as_raw_fd(),
            &source.file_name,
        );
        let cleanup = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
        return Err(join_rollback_error(error, restore.and(cleanup)));
    }

    // The only unlink is now against a validated object inside the private
    // quarantine directory, never against the attacker-controlled source name.
    if let Err(error) = unlink_validated_private_file(quarantine_fd.as_raw_fd(), &item_name) {
        let restore = rename_no_replace_between(
            quarantine_fd.as_raw_fd(),
            &item_name,
            source.fd.as_raw_fd(),
            &source.file_name,
        );
        let cleanup = remove_empty_quarantine(state, &quarantine_name, quarantine_identity);
        return Err(join_rollback_error(error, restore.and(cleanup)));
    }
    if let Err(error) = before_quarantine_fsync() {
        return Err(format!(
            "risk-escalated: quarantined projection was unlinked before fsync failed: {error}"
        ));
    }
    sync_directory(quarantine_fd.as_raw_fd()).map_err(|error| {
        format!("risk-escalated: quarantined projection was unlinked before fsync failed: {error}")
    })?;
    remove_empty_quarantine(state, &quarantine_name, quarantine_identity).map_err(
        |error| format!("risk-escalated: projection deletion committed before quarantine cleanup failed: {error}"),
    )?;
    if let Err(error) = before_parent_fsync() {
        return Err(format!(
            "risk-escalated: projection deletion committed before parent fsync failed: {error}"
        ));
    }
    sync_directory(source.fd.as_raw_fd()).map_err(|error| {
        format!("risk-escalated: projection deletion committed before parent fsync failed: {error}")
    })
}

#[cfg(unix)]
fn establish_quarantine_state(workspace_fd: i32) -> Result<QuarantineState, String> {
    let name = CString::new(QUARANTINE_STATE_NAME).expect("static state name has no NUL");
    // SAFETY: workspace_fd is retained and name is one component. Creation is
    // state setup only; failures leave any unproved residue untouched.
    let created = if unsafe { libc::mkdirat(workspace_fd, name.as_ptr(), 0o700) } == 0 {
        true
    } else {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            false
        } else {
            return Err(format!(
                "before-effect blocked: cannot establish AGS projection state directory: {error}"
            ));
        }
    };
    let fd = open_directory_at(workspace_fd, &name).map_err(|error| {
        if created {
            format!(
                "risk-escalated: created_directory_residue preserved because AGS projection state identity cannot be proven: {error}"
            )
        } else {
            format!(
                "before-effect blocked: AGS projection state directory cannot be opened safely: {error}"
            )
        }
    })?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat is writable and fd is a retained directory descriptor.
    if unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "risk-escalated: created_directory_residue preserved because AGS projection state metadata cannot be proven: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fstat succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    let identity = FileIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
    };
    let workspace_identity = identity_of_fd(workspace_fd)?;
    let mode = stat.st_mode & 0o7777;
    // SAFETY: geteuid has no preconditions.
    let owner_matches = stat.st_uid == unsafe { libc::geteuid() };
    if !owner_matches || mode != 0o700 || identity.device != workspace_identity.device {
        let prefix = if created {
            "risk-escalated: created_directory_residue preserved"
        } else {
            "before-effect blocked"
        };
        return Err(format!(
            "{prefix}: AGS projection state requires current owner, mode 0700, and workspace filesystem"
        ));
    }
    if created {
        sync_directory(workspace_fd).map_err(|error| {
            format!(
                "risk-escalated: created_directory_residue preserved because AGS projection state parent sync failed: {error}"
            )
        })?;
    }
    Ok(QuarantineState { fd, identity })
}

#[cfg(unix)]
fn create_quarantine_at(
    state: &QuarantineState,
) -> Result<(CString, OwnedFd, FileIdentity), String> {
    for _ in 0..64 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = CString::new(format!(".ags-quarantine-{}-{sequence}", std::process::id()))
            .expect("generated quarantine name has no NUL");
        // SAFETY: the retained state fd was validated as current-owner 0700;
        // same-credential processes are inside the documented trust boundary.
        if unsafe { libc::mkdirat(state.fd.as_raw_fd(), name.as_ptr(), 0o700) } == 0 {
            let fd = open_directory_at(state.fd.as_raw_fd(), &name).map_err(|error| {
                format!(
                    "risk-escalated: created_directory_residue preserved because quarantine identity cannot be proven: {error}"
                )
            })?;
            let identity = identity_of_fd(fd.as_raw_fd()).map_err(|error| {
                format!(
                    "risk-escalated: created_directory_residue preserved because quarantine identity cannot be proven: {error}"
                )
            })?;
            if identity.device != state.identity.device {
                return Err(
                    "before-effect blocked: projection quarantine state crossed filesystems (EXDEV)"
                        .to_string(),
                );
            }
            return Ok((name, fd, identity));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(format!("cannot create projection quarantine: {error}"));
        }
    }
    Err("cannot allocate a unique projection quarantine".to_string())
}

#[cfg(unix)]
fn remove_empty_quarantine(
    state: &QuarantineState,
    name: &CString,
    expected: FileIdentity,
) -> Result<(), String> {
    let opened = open_directory_at(state.fd.as_raw_fd(), name)?;
    if identity_of_fd(opened.as_raw_fd())? != expected {
        return Err("risk-escalated: projection quarantine identity changed".to_string());
    }
    // SAFETY: this name is inside the retained, validated state-dir FD and the
    // child identity was revalidated; it is not a public workspace basename.
    if unsafe { libc::unlinkat(state.fd.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "cannot remove projection quarantine: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_no_replace_between(
    from_fd: i32,
    from: &CString,
    to_fd: i32,
    to: &CString,
) -> Result<(), String> {
    use rustix::fs::{renameat_with, RenameFlags};
    use std::os::fd::BorrowedFd;

    // SAFETY: descriptors remain live for the duration of this call.
    let from_fd = unsafe { BorrowedFd::borrow_raw(from_fd) };
    // SAFETY: descriptors remain live for the duration of this call.
    let to_fd = unsafe { BorrowedFd::borrow_raw(to_fd) };
    renameat_with(
        from_fd,
        from.as_c_str(),
        to_fd,
        to.as_c_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| error.to_string())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn rename_no_replace_between(
    _from_fd: i32,
    _from: &CString,
    _to_fd: i32,
    _to: &CString,
) -> Result<(), String> {
    Err("conditional cross-directory rename is unavailable".to_string())
}

#[cfg(unix)]
fn sync_directory(directory_fd: i32) -> Result<(), String> {
    // SAFETY: directory_fd is a live directory descriptor.
    if unsafe { libc::fsync(directory_fd) } == 0 {
        Ok(())
    } else {
        Err(format!(
            "cannot sync projection parent directory: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(target_os = "macos")]
fn conditional_rename_supported() -> bool {
    true
}

#[cfg(target_os = "linux")]
fn conditional_rename_supported() -> bool {
    true
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn conditional_rename_supported() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn rename_no_replace(directory_fd: i32, from: &CString, to: &CString) -> Result<(), String> {
    // SAFETY: both names are NUL-terminated components under the same live
    // directory descriptor. RENAME_EXCL never replaces an appeared target.
    let result = unsafe {
        libc::renameatx_np(
            directory_fd,
            from.as_ptr(),
            directory_fd,
            to.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    rename_result(result)
}

#[cfg(target_os = "linux")]
fn rename_no_replace(directory_fd: i32, from: &CString, to: &CString) -> Result<(), String> {
    // SAFETY: both names are NUL-terminated components under the same live
    // directory descriptor. RENAME_NOREPLACE never replaces an appeared target.
    let result = unsafe {
        libc::renameat2(
            directory_fd,
            from.as_ptr(),
            directory_fd,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    rename_result(result)
}

#[cfg(target_os = "macos")]
fn rename_exchange(directory_fd: i32, from: &CString, to: &CString) -> Result<(), String> {
    // SAFETY: both names are NUL-terminated components under the same live
    // directory descriptor. RENAME_SWAP atomically exchanges path entries.
    let result = unsafe {
        libc::renameatx_np(
            directory_fd,
            from.as_ptr(),
            directory_fd,
            to.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    rename_result(result)
}

#[cfg(target_os = "linux")]
fn rename_exchange(directory_fd: i32, from: &CString, to: &CString) -> Result<(), String> {
    // SAFETY: both names are NUL-terminated components under the same live
    // directory descriptor. RENAME_EXCHANGE atomically exchanges path entries.
    let result = unsafe {
        libc::renameat2(
            directory_fd,
            from.as_ptr(),
            directory_fd,
            to.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    rename_result(result)
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn rename_no_replace(_directory_fd: i32, _from: &CString, _to: &CString) -> Result<(), String> {
    Err("conditional rename is unavailable".to_string())
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn rename_exchange(_directory_fd: i32, _from: &CString, _to: &CString) -> Result<(), String> {
    Err("conditional exchange is unavailable".to_string())
}

#[cfg(unix)]
fn rename_result(result: i32) -> Result<(), String> {
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/projection-migration/owned-generated.txt"
    ));
    const UNOWNED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/projection-migration/unowned-user.txt"
    ));
    const MODIFIED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/projection-migration/modified-user.txt"
    ));
    const PRISTINE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/projection-migration/pristine-generated.txt"
    ));

    #[cfg(unix)]
    fn is_generated_projection_temp_name(path: &Path) -> bool {
        let prefix = format!(".ags-projection-{}-", std::process::id());
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|suffix| suffix.strip_suffix(".tmp"))
            .is_some_and(|sequence| {
                !sequence.is_empty() && sequence.bytes().all(|byte| byte.is_ascii_digit())
            })
    }

    #[cfg(unix)]
    fn find_unique_projection_temp(workspace: &Path) -> Result<PathBuf, String> {
        let mut candidates = Vec::new();
        for entry in std::fs::read_dir(workspace).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
                && is_generated_projection_temp_name(&entry.path())
            {
                candidates.push(entry.path());
            }
        }
        match candidates.as_slice() {
            [temp] => Ok(temp.clone()),
            [] => Err("projection temp is missing".to_string()),
            _ => Err(format!(
                "expected one projection temp, found {}",
                candidates.len()
            )),
        }
    }

    #[cfg(unix)]
    fn substitute_projection_temp(
        workspace: &Path,
        saved_staged: &Path,
        substitute: &[u8],
    ) -> Result<(), String> {
        let temp = find_unique_projection_temp(workspace)?;
        std::fs::rename(&temp, saved_staged).map_err(|error| error.to_string())?;
        std::fs::write(temp, substitute).map_err(|error| error.to_string())
    }

    #[test]
    fn pristine_projection_creates_parents_profile_and_ownership_in_one_transaction() {
        let workspace = tempfile::tempdir().unwrap();
        let plan = plan_project_projection(
            workspace.path(),
            &[
                ProjectProjectionFile::write("AGENTS.md", PRISTINE),
                ProjectProjectionFile::write(
                    "config/agent-project-profile.yaml",
                    b"schema_version: ags://schema/contract/v2/project-profile\n",
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            plan.planned_directories(),
            [PathBuf::from(".ags"), PathBuf::from("config")]
        );
        assert!(plan.conflicts().is_empty());

        let receipt = apply_project_projection(&plan).unwrap();
        assert!(receipt.changed);
        assert_eq!(
            std::fs::read(workspace.path().join("AGENTS.md")).unwrap(),
            PRISTINE
        );
        assert!(workspace
            .path()
            .join("config/agent-project-profile.yaml")
            .is_file());
        let ownership: serde_json::Value = serde_json::from_slice(
            &std::fs::read(workspace.path().join(".ags/ownership-v2.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            ownership["schema_version"],
            "ags://schema/contract/v2/project-ownership"
        );
        for entry in ownership["entries"].as_object().unwrap().values() {
            assert_eq!(entry.as_object().unwrap().len(), 1);
            assert!(entry.get("last_applied_sha256").is_some());
            assert!(entry.get("user_owned").is_none());
            assert!(entry.get("owner").is_none());
        }
    }

    #[test]
    fn owned_updates_and_deletes_but_unowned_and_modified_bytes_remain_conflicts() {
        let workspace = tempfile::tempdir().unwrap();
        let initial = plan_project_projection(
            workspace.path(),
            &[
                ProjectProjectionFile::write("AGENTS.md", GENERATED),
                ProjectProjectionFile::write("config/profile.yaml", GENERATED),
            ],
        )
        .unwrap();
        apply_project_projection(&initial).unwrap();

        std::fs::write(workspace.path().join("AGENTS.md"), MODIFIED).unwrap();
        std::fs::write(workspace.path().join("unowned.md"), UNOWNED).unwrap();
        let plan = plan_project_projection(
            workspace.path(),
            &[
                ProjectProjectionFile::write("AGENTS.md", b"new generated\n"),
                ProjectProjectionFile::write("unowned.md", b"replacement\n"),
            ],
        )
        .unwrap();
        assert_eq!(plan.conflicts().len(), 2);
        for conflict in plan.conflicts() {
            assert!(conflict.details_uri.starts_with("ags-details://sha256/"));
            assert!(plan.resolve_details(&conflict.details_uri).is_some());
        }

        let receipt = apply_project_projection(&plan).unwrap();
        assert!(
            receipt.changed,
            "the exact-owned profile deletion must apply"
        );
        assert_eq!(
            std::fs::read(workspace.path().join("AGENTS.md")).unwrap(),
            MODIFIED
        );
        assert_eq!(
            std::fs::read(workspace.path().join("unowned.md")).unwrap(),
            UNOWNED
        );
        assert!(!workspace.path().join("config/profile.yaml").exists());
        let ownership: serde_json::Value = serde_json::from_slice(
            &std::fs::read(workspace.path().join(".ags/ownership-v2.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            ownership["entries"]["AGENTS.md"]["last_applied_sha256"],
            ags_platform::sha256(GENERATED)
        );
        assert!(ownership["entries"].get("unowned.md").is_none());
        assert!(ownership["entries"].get("config/profile.yaml").is_none());
    }

    #[test]
    fn byte_equal_unowned_file_is_preserved_and_never_converted_to_owned() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("AGENTS.md"), GENERATED).unwrap();
        let plan = plan_project_projection(
            workspace.path(),
            &[ProjectProjectionFile::write("AGENTS.md", GENERATED)],
        )
        .unwrap();
        assert_eq!(plan.conflicts().len(), 1);
        assert_eq!(
            plan.conflicts()[0].disposition,
            ProjectProjectionDisposition::PreserveUnowned
        );
        apply_project_projection(&plan).unwrap();
        let ownership: serde_json::Value = serde_json::from_slice(
            &std::fs::read(workspace.path().join(".ags/ownership-v2.json")).unwrap(),
        )
        .unwrap();
        assert!(ownership["entries"].get("AGENTS.md").is_none());
        assert_eq!(
            std::fs::read(workspace.path().join("AGENTS.md")).unwrap(),
            GENERATED
        );
    }

    #[test]
    #[cfg(unix)]
    fn batch_directory_creation_and_rollback_reject_parent_swap_without_touching_decoy() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let moved = outside.path().join("moved-parent");
        let decoy = outside.path().join("decoy");
        std::fs::create_dir(&decoy).unwrap();
        std::fs::write(decoy.join("keep"), b"decoy\n").unwrap();
        let plan = plan_project_projection(
            workspace.path(),
            &[ProjectProjectionFile::write("nested/child/file", GENERATED)],
        )
        .unwrap();
        let mut swapped = false;
        let mut decoy_child_observed = false;
        let error = apply_project_projection_with_hooks(
            &plan,
            &mut ProjectApplyHooks {
                after_directory_create: Some(&mut |relative| {
                    if relative == Path::new("nested") && !swapped {
                        std::fs::rename(workspace.path().join("nested"), &moved)
                            .map_err(|error| error.to_string())?;
                        std::os::unix::fs::symlink(&decoy, workspace.path().join("nested"))
                            .map_err(|error| error.to_string())?;
                        swapped = true;
                    }
                    if relative == Path::new("nested/child") && decoy.join("child").is_dir() {
                        decoy_child_observed = true;
                    }
                    Ok(())
                }),
                ..ProjectApplyHooks::noop()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("parent") || error.contains("directory") || error.contains("symlink"),
            "{error}"
        );
        assert_eq!(std::fs::read(decoy.join("keep")).unwrap(), b"decoy\n");
        assert!(!decoy_child_observed, "mkdir followed the swapped parent");
        assert!(!decoy.join("child").exists());
        assert!(
            moved.is_dir(),
            "swapped AGS directory must not be path-deleted"
        );
    }

    #[test]
    #[cfg(unix)]
    fn batch_apply_retains_workspace_root_capability_across_root_swap() {
        let holder = tempfile::tempdir().unwrap();
        let workspace = holder.path().join("workspace");
        let retained = holder.path().join("retained");
        std::fs::create_dir(&workspace).unwrap();
        let plan = plan_project_projection(
            &workspace,
            &[ProjectProjectionFile::write("new.md", GENERATED)],
        )
        .unwrap();
        let error = apply_project_projection_with_hooks(
            &plan,
            &mut ProjectApplyHooks {
                after_workspace_validation: Some(&mut || {
                    std::fs::rename(&workspace, &retained).map_err(|error| error.to_string())?;
                    std::fs::create_dir(&workspace).map_err(|error| error.to_string())
                }),
                ..ProjectApplyHooks::noop()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("workspace") || error.contains("binding") || error.contains("rollback"),
            "{error}"
        );
        assert!(!workspace.join("new.md").exists());
        assert!(!retained.join("new.md").exists());
    }

    #[test]
    #[cfg(unix)]
    fn directory_planning_uses_retained_root_during_swap_and_restore() {
        let holder = tempfile::tempdir().unwrap();
        let workspace = holder.path().join("workspace");
        let retained = holder.path().join("retained");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(workspace.join("nested")).unwrap();
        let workspace_fd = open_workspace(&workspace).unwrap();
        let planned = planned_projection_directories_from_fd_inner(
            workspace_fd.as_raw_fd(),
            [Path::new("nested/file.md")],
            || {
                std::fs::rename(&workspace, &retained).map_err(|error| error.to_string())?;
                std::fs::create_dir(&workspace).map_err(|error| error.to_string())
            },
            || {
                std::fs::remove_dir(&workspace).map_err(|error| error.to_string())?;
                std::fs::rename(&retained, &workspace).map_err(|error| error.to_string())
            },
        )
        .unwrap();
        assert!(
            planned.is_empty(),
            "retained root already has nested: {planned:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn created_directory_rollback_quarantines_and_restores_substitution() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_fd = open_workspace(workspace.path()).unwrap();
        let quarantine_state = establish_quarantine_state(workspace_fd.as_raw_fd()).unwrap();
        let created =
            create_projection_directory(workspace_fd.as_raw_fd(), Path::new("created")).unwrap();
        let saved = workspace.path().join("saved-created");
        let target = workspace.path().join("created");
        let error = rollback_created_directories_inner(&[created], &quarantine_state, |_| {
            std::fs::rename(&target, &saved).map_err(|error| error.to_string())?;
            std::fs::create_dir(&target).map_err(|error| error.to_string())
        })
        .unwrap_err();
        assert!(error.contains("substitution"), "{error}");
        assert!(
            saved.is_dir(),
            "original created directory must be preserved"
        );
        assert!(
            target.is_dir(),
            "substituted user directory must be restored"
        );
    }

    #[test]
    #[cfg(unix)]
    fn mkdir_open_failure_preserves_created_or_substituted_residue() {
        for substituted in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let workspace_fd = open_workspace(workspace.path()).unwrap();
            let target = workspace.path().join("created");
            let saved = workspace.path().join("saved-created");
            let error = create_projection_directory_with_after_mkdir(
                workspace_fd.as_raw_fd(),
                Path::new("created"),
                || {
                    if substituted {
                        std::fs::rename(&target, &saved).map_err(|error| error.to_string())?;
                        std::fs::create_dir(&target).map_err(|error| error.to_string())?;
                        std::fs::write(target.join("user.keep"), UNOWNED)
                            .map_err(|error| error.to_string())?;
                    }
                    Err("injected post-mkdir open failure".to_string())
                },
            )
            .err()
            .expect("post-mkdir seam must fail");
            assert!(error.contains("risk-escalated"), "{error}");
            assert!(error.contains("created_directory_residue"), "{error}");
            assert!(target.is_dir(), "public residue must never be removed");
            if substituted {
                assert!(saved.is_dir(), "original created directory must remain");
                assert_eq!(std::fs::read(target.join("user.keep")).unwrap(), UNOWNED);
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn rollback_identity_probe_failure_preserves_public_residue() {
        let workspace = tempfile::tempdir().unwrap();
        let workspace_fd = open_workspace(workspace.path()).unwrap();
        let quarantine_state = establish_quarantine_state(workspace_fd.as_raw_fd()).unwrap();
        let created =
            create_projection_directory(workspace_fd.as_raw_fd(), Path::new("created")).unwrap();
        let error = rollback_created_directories_inner_with_identity_probe(
            &[created],
            &quarantine_state,
            |_| Ok(()),
            |_, _| Err("injected identity probe failure".to_string()),
        )
        .unwrap_err();
        assert!(error.contains("risk-escalated"), "{error}");
        assert!(error.contains("created_directory_residue"), "{error}");
        assert!(workspace.path().join("created").is_dir());
    }

    #[test]
    #[cfg(unix)]
    fn revalidation_failures_report_directory_substitution_risk_and_preserve_user_dir() {
        for ownership_failure in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let initial = plan_project_projection(
                workspace.path(),
                &[ProjectProjectionFile::write("stable.md", GENERATED)],
            )
            .unwrap();
            apply_project_projection(&initial).unwrap();
            let plan = plan_project_projection(
                workspace.path(),
                &[
                    ProjectProjectionFile::write("stable.md", GENERATED),
                    ProjectProjectionFile::write("nested/new.md", PRISTINE),
                ],
            )
            .unwrap();
            let saved = workspace.path().join("saved-nested");
            let nested = workspace.path().join("nested");
            let error = apply_project_projection_with_hooks(
                &plan,
                &mut ProjectApplyHooks {
                    after_directory_create: Some(&mut |relative| {
                        if relative == Path::new("nested") {
                            std::fs::rename(&nested, &saved).map_err(|error| error.to_string())?;
                            std::fs::create_dir(&nested).map_err(|error| error.to_string())?;
                            std::fs::write(nested.join("user.keep"), UNOWNED)
                                .map_err(|error| error.to_string())?;
                            if ownership_failure {
                                std::fs::write(
                                    workspace.path().join(OWNERSHIP_RELATIVE_PATH),
                                    b"changed ownership\n",
                                )
                                .map_err(|error| error.to_string())?;
                            } else {
                                std::fs::write(workspace.path().join("stable.md"), MODIFIED)
                                    .map_err(|error| error.to_string())?;
                            }
                        }
                        Ok(())
                    }),
                    ..ProjectApplyHooks::noop()
                },
            )
            .unwrap_err();
            assert!(error.contains("risk-escalated"), "{error}");
            assert_eq!(std::fs::read(nested.join("user.keep")).unwrap(), UNOWNED);
            assert!(saved.is_dir());
        }
    }

    #[test]
    #[cfg(unix)]
    fn delete_quarantines_then_validates_substitution_and_preserves_user_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        let saved = workspace.path().join("saved-owned.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        let error = delete_exact_projection_inner(&plan, || {
            std::fs::rename(&target, &saved).map_err(|error| error.to_string())?;
            std::fs::write(&target, UNOWNED).map_err(|error| error.to_string())
        })
        .unwrap_err();
        assert!(error.contains("substitution"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), UNOWNED);
        assert_eq!(std::fs::read(&saved).unwrap(), GENERATED);
    }

    #[test]
    #[cfg(unix)]
    fn quarantine_before_move_same_inode_digest_substitution_is_restored_not_deleted() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        let saved = workspace.path().join("saved-owned.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        let error = delete_exact_projection_inner(&plan, || {
            std::fs::rename(&target, &saved).map_err(|error| error.to_string())?;
            std::fs::write(&target, GENERATED).map_err(|error| error.to_string())
        })
        .unwrap_err();
        assert!(error.contains("substitution"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), GENERATED);
        assert_eq!(std::fs::read(&saved).unwrap(), GENERATED);
    }

    #[test]
    #[cfg(unix)]
    fn quarantine_state_fd_is_retained_and_compatibility_failures_are_pre_effect() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        let public_state = workspace.path().join(QUARANTINE_STATE_NAME);
        std::fs::create_dir(&public_state).unwrap();
        std::fs::set_permissions(&public_state, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = apply_projection_migration(&plan, PRISTINE).unwrap_err();
        assert!(error.contains("before-effect blocked"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), GENERATED);

        for compatibility_error in ["EXDEV", "ENOTSUP"] {
            let workspace = tempfile::tempdir().unwrap();
            let target = workspace.path().join("owned.md");
            std::fs::write(&target, GENERATED).unwrap();
            let plan = plan_projection_migration(
                workspace.path(),
                Path::new("owned.md"),
                Some(&ags_platform::sha256(GENERATED)),
            )
            .unwrap();
            let state = establish_quarantine_state(plan.workspace_fd.as_raw_fd()).unwrap();
            let opened =
                open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)
                    .unwrap();
            let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)
                .unwrap()
                .unwrap();
            let error = quarantine_then_delete_inner(
                &state,
                plan.workspace_fd.as_raw_fd(),
                &plan.relative_path,
                opened.identity,
                current.identity,
                &current.sha256,
                || Err(compatibility_error.to_string()),
                || Ok(()),
                || Ok(()),
                || Ok(()),
                || Ok(()),
            )
            .unwrap_err();
            assert!(error.contains("before-effect blocked"), "{error}");
            assert!(error.contains(compatibility_error), "{error}");
            assert_eq!(std::fs::read(&target).unwrap(), GENERATED);
        }

        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        let state = establish_quarantine_state(plan.workspace_fd.as_raw_fd()).unwrap();
        let public_state = workspace.path().join(QUARANTINE_STATE_NAME);
        assert_eq!(
            std::fs::metadata(&public_state)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let retained_state = workspace.path().join("retained-state");
        std::fs::rename(&public_state, &retained_state).unwrap();
        std::fs::create_dir(&public_state).unwrap();
        std::fs::set_permissions(&public_state, std::fs::Permissions::from_mode(0o700)).unwrap();
        let opened =
            open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)
                .unwrap();
        let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)
            .unwrap()
            .unwrap();
        quarantine_then_delete(
            &state,
            plan.workspace_fd.as_raw_fd(),
            &plan.relative_path,
            opened.identity,
            current.identity,
            &current.sha256,
            || Ok(()),
        )
        .unwrap();
        assert!(!target.exists());
        assert_eq!(std::fs::read_dir(&public_state).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(&retained_state).unwrap().count(), 0);
    }

    #[test]
    fn public_projection_basenames_have_zero_direct_unlink_or_rmdir_sites() {
        let source = include_str!("projection_migration.rs");
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source prefix exists");
        for forbidden in [
            "unlinkat(current.as_raw_fd()",
            "unlinkat(directory.parent_fd.as_raw_fd()",
            "unlinkat(workspace_fd",
            "std::fs::remove_dir",
        ] {
            assert!(
                !production.contains(forbidden),
                "forbidden public delete: {forbidden}"
            );
        }
        assert_eq!(production.matches("libc::unlinkat(").count(), 3);
        assert!(production.contains("quarantine_fd.as_raw_fd(), item.as_ptr()"));
        assert!(production.contains("state.fd.as_raw_fd(), name.as_ptr()"));
        assert!(production.contains("unlink_validated_private_file"));

        let security = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../SECURITY.md"));
        let protocol = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol/project-projection.md"
        ));
        for contract in [security, protocol] {
            assert!(contract.contains("same credentials"), "{contract}");
            assert!(contract.contains("0700"), "{contract}");
            assert!(contract.contains("trust boundary"), "{contract}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn create_recovery_quarantines_substitution_instead_of_unlinking_user_file() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("created.md");
        let saved = workspace.path().join("saved-created.md");
        let plan =
            plan_projection_migration(workspace.path(), Path::new("created.md"), None).unwrap();
        apply_projection_migration(&plan, GENERATED).unwrap();
        let error = recover_projection_migration_inner(&plan, GENERATED, None, || {
            std::fs::rename(&target, &saved).map_err(|error| error.to_string())?;
            std::fs::write(&target, UNOWNED).map_err(|error| error.to_string())
        })
        .unwrap_err();
        assert!(error.contains("substitution"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), UNOWNED);
        assert_eq!(std::fs::read(&saved).unwrap(), GENERATED);
    }

    #[test]
    #[cfg(unix)]
    fn atomic_effect_guard_recovers_create_replace_and_file_fsync_faults() {
        let workspace = tempfile::tempdir().unwrap();
        let create =
            plan_projection_migration(workspace.path(), Path::new("created.md"), None).unwrap();
        let error = apply_projection_migration_inner_with_after(
            &create,
            GENERATED,
            || Ok(()),
            || Err("injected after rename".to_string()),
        )
        .unwrap_err();
        assert!(error.contains("after rename"), "{error}");
        assert!(!workspace.path().join("created.md").exists());

        std::fs::write(workspace.path().join("owned.md"), GENERATED).unwrap();
        let replace = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        let error = apply_projection_migration_inner_with_after(
            &replace,
            PRISTINE,
            || Ok(()),
            || Err("injected after exchange".to_string()),
        )
        .unwrap_err();
        assert!(error.contains("after exchange"), "{error}");
        assert_eq!(
            std::fs::read(workspace.path().join("owned.md")).unwrap(),
            GENERATED
        );

        let create =
            plan_projection_migration(workspace.path(), Path::new("fsync.md"), None).unwrap();
        let error = apply_projection_migration_inner_with_faults(
            &create,
            GENERATED,
            || Ok(()),
            || Err("injected file fsync".to_string()),
            || Ok(()),
            || Ok(()),
        )
        .unwrap_err();
        assert!(error.contains("file fsync"), "{error}");
        assert!(!workspace.path().join("fsync.md").exists());

        let create =
            plan_projection_migration(workspace.path(), Path::new("parent-sync.md"), None).unwrap();
        let error = apply_projection_migration_inner_with_faults(
            &create,
            GENERATED,
            || Ok(()),
            || Ok(()),
            || Ok(()),
            || Err("injected parent fsync".to_string()),
        )
        .unwrap_err();
        assert!(error.contains("parent fsync"), "{error}");
        assert!(!workspace.path().join("parent-sync.md").exists());
    }

    #[test]
    #[cfg(unix)]
    fn atomic_effect_guard_never_deletes_unknown_create_or_replace_inodes() {
        for (case, create_substitute, replace_substitute) in [
            ("same", GENERATED, PRISTINE),
            ("different", UNOWNED, UNOWNED),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let target = workspace.path().join("created.md");
            let saved_staged = workspace.path().join(format!("created-{case}-staged.md"));
            let plan =
                plan_projection_migration(workspace.path(), Path::new("created.md"), None).unwrap();
            let error = apply_projection_migration_inner_with_after(
                &plan,
                GENERATED,
                || Ok(()),
                || {
                    std::fs::rename(&target, &saved_staged).map_err(|error| error.to_string())?;
                    std::fs::write(&target, create_substitute).map_err(|error| error.to_string())
                },
            )
            .unwrap_err();
            assert!(error.contains("risk-escalated"), "create/{case}: {error}");
            assert_eq!(
                std::fs::read(&target).unwrap(),
                create_substitute,
                "create/{case}"
            );
            assert_eq!(
                std::fs::read(&saved_staged).unwrap(),
                GENERATED,
                "create/{case}"
            );

            let workspace = tempfile::tempdir().unwrap();
            let target = workspace.path().join("owned.md");
            let saved_staged = workspace.path().join(format!("replace-{case}-staged.md"));
            std::fs::write(&target, GENERATED).unwrap();
            let plan = plan_projection_migration(
                workspace.path(),
                Path::new("owned.md"),
                Some(&ags_platform::sha256(GENERATED)),
            )
            .unwrap();
            let error = apply_projection_migration_inner_with_after(
                &plan,
                PRISTINE,
                || Ok(()),
                || {
                    std::fs::rename(&target, &saved_staged).map_err(|error| error.to_string())?;
                    std::fs::write(&target, replace_substitute).map_err(|error| error.to_string())
                },
            )
            .unwrap_err();
            assert!(error.contains("risk-escalated"), "replace/{case}: {error}");
            assert_eq!(
                std::fs::read(&target).unwrap(),
                replace_substitute,
                "replace/{case}"
            );
            assert_eq!(
                std::fs::read(&saved_staged).unwrap(),
                PRISTINE,
                "replace/{case}"
            );
            assert!(
                std::fs::read_dir(workspace.path()).unwrap().any(|entry| {
                    let path = entry.unwrap().path();
                    path.is_file()
                        && std::fs::read(path)
                            .map(|bytes| bytes == GENERATED)
                            .unwrap_or(false)
                }),
                "replace/{case}: old target object must remain"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn atomic_effect_guard_never_exchanges_or_unlinks_unknown_old_name_inode() {
        for (case, substitute) in [("same", GENERATED), ("different", UNOWNED)] {
            let workspace = tempfile::tempdir().unwrap();
            let target = workspace.path().join("owned.md");
            let saved_old = workspace.path().join(format!("replace-{case}-old.md"));
            std::fs::write(&target, GENERATED).unwrap();
            let plan = plan_projection_migration(
                workspace.path(),
                Path::new("owned.md"),
                Some(&ags_platform::sha256(GENERATED)),
            )
            .unwrap();
            let error = apply_projection_migration_inner_with_after(
                &plan,
                PRISTINE,
                || Ok(()),
                || {
                    let temp = find_unique_projection_temp(workspace.path())?;
                    std::fs::rename(&temp, &saved_old).map_err(|error| error.to_string())?;
                    std::fs::write(&temp, substitute).map_err(|error| error.to_string())
                },
            )
            .unwrap_err();
            assert!(error.contains("risk-escalated"), "old-name/{case}: {error}");
            assert_eq!(std::fs::read(&target).unwrap(), PRISTINE, "old-name/{case}");
            assert_eq!(
                std::fs::read(&saved_old).unwrap(),
                GENERATED,
                "old-name/{case}"
            );
            assert!(
                std::fs::read_dir(workspace.path()).unwrap().any(|entry| {
                    let path = entry.unwrap().path();
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(".ags-projection-"))
                        && std::fs::read(path)
                            .map(|bytes| bytes == substitute)
                            .unwrap_or(false)
                }),
                "old-name/{case}: unknown temp inode must remain"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn staged_temp_name_substitution_is_never_committed_or_unlinked() {
        for phase in ["before_file_fsync", "before_commit"] {
            for hook_result in ["err", "ok"] {
                for (bytes_case, substitute) in [("same", GENERATED), ("different", UNOWNED)] {
                    let workspace = tempfile::tempdir().unwrap();
                    let target = workspace.path().join("created.md");
                    let saved_staged = workspace
                        .path()
                        .join(format!("{phase}-{hook_result}-{bytes_case}-staged.md"));
                    let hook_replaced_staged = std::cell::Cell::new(false);
                    let plan =
                        plan_projection_migration(workspace.path(), Path::new("created.md"), None)
                            .unwrap();
                    let mutate = || {
                        substitute_projection_temp(workspace.path(), &saved_staged, substitute)?;
                        hook_replaced_staged.set(true);
                        if hook_result == "err" {
                            Err(format!("injected {phase} hook failure"))
                        } else {
                            Ok(())
                        }
                    };
                    let result = if phase == "before_file_fsync" {
                        apply_projection_migration_inner_with_faults(
                            &plan,
                            GENERATED,
                            || Ok(()),
                            mutate,
                            || Ok(()),
                            || Ok(()),
                        )
                    } else {
                        apply_projection_migration_inner_with_faults(
                            &plan,
                            GENERATED,
                            mutate,
                            || Ok(()),
                            || Ok(()),
                            || Ok(()),
                        )
                    };
                    assert!(
                        hook_replaced_staged.get(),
                        "{phase}/{hook_result}/{bytes_case}: mutation hook did not replace the staged regular temp"
                    );
                    let error = result.unwrap_err();
                    assert!(
                        error.contains("risk-escalated"),
                        "{phase}/{hook_result}/{bytes_case}: {error}"
                    );
                    assert!(
                        !target.exists(),
                        "{phase}/{hook_result}/{bytes_case}: unknown temp committed"
                    );
                    assert_eq!(
                        std::fs::read(&saved_staged).unwrap(),
                        GENERATED,
                        "{phase}/{hook_result}/{bytes_case}: original staged inode lost"
                    );
                    assert!(
                        std::fs::read_dir(workspace.path()).unwrap().any(|entry| {
                            let path = entry.unwrap().path();
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.starts_with(".ags-projection-"))
                                && std::fs::read(path)
                                    .map(|bytes| bytes == substitute)
                                    .unwrap_or(false)
                        }),
                        "{phase}/{hook_result}/{bytes_case}: unknown temp inode was removed"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn projection_fifo_without_writer_is_rejected_without_blocking() {
        let workspace = tempfile::tempdir().unwrap();
        let fifo = workspace.path().join("profile.fifo");
        let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: fifo_name is a valid NUL-terminated path inside the temp workspace.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);

        let workspace_path = workspace.path().to_path_buf();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result =
                plan_projection_migration(&workspace_path, Path::new("profile.fifo"), None);
            let _ = sender.send(result);
        });
        match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(result) => {
                let error = result.unwrap_err();
                assert!(error.contains("regular file"), "{error}");
                worker.join().unwrap();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
                drop(writer);
                let _ = receiver.recv_timeout(std::time::Duration::from_secs(1));
                worker.join().unwrap();
                panic!("projection FIFO inspection blocked waiting for a writer");
            }
            Err(error) => panic!("projection FIFO probe channel failed: {error}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn delete_effect_guard_restores_pre_unlink_faults_and_risk_escalates_post_unlink_fsyncs() {
        for stage in [
            "after_move",
            "before_unlink",
            "quarantine_fsync",
            "parent_fsync",
        ] {
            let workspace = tempfile::tempdir().unwrap();
            let target = workspace.path().join("owned.md");
            std::fs::write(&target, GENERATED).unwrap();
            let plan = plan_projection_migration(
                workspace.path(),
                Path::new("owned.md"),
                Some(&ags_platform::sha256(GENERATED)),
            )
            .unwrap();
            let opened =
                open_parent_from_workspace_fd(plan.workspace_fd.as_raw_fd(), &plan.relative_path)
                    .unwrap();
            let current = inspect_at(opened.fd.as_raw_fd(), &opened.file_name)
                .unwrap()
                .unwrap();
            let quarantine_state =
                establish_quarantine_state(plan.workspace_fd.as_raw_fd()).unwrap();
            let error = quarantine_then_delete_inner(
                &quarantine_state,
                plan.workspace_fd.as_raw_fd(),
                &plan.relative_path,
                opened.identity,
                current.identity,
                &current.sha256,
                || Ok(()),
                || {
                    if stage == "after_move" {
                        Err(stage.to_string())
                    } else {
                        Ok(())
                    }
                },
                || {
                    if stage == "before_unlink" {
                        Err(stage.to_string())
                    } else {
                        Ok(())
                    }
                },
                || {
                    if stage == "quarantine_fsync" {
                        Err(stage.to_string())
                    } else {
                        Ok(())
                    }
                },
                || {
                    if stage == "parent_fsync" {
                        Err(stage.to_string())
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap_err();
            if matches!(stage, "after_move" | "before_unlink") {
                assert_eq!(
                    std::fs::read(&target).unwrap(),
                    GENERATED,
                    "{stage}: {error}"
                );
            } else {
                assert!(error.contains("risk-escalated"), "{stage}: {error}");
                assert!(!target.exists(), "{stage}: {error}");
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn delete_rollback_preserves_appeared_user_file_and_reports_risk() {
        let workspace = tempfile::tempdir().unwrap();
        let initial = plan_project_projection(
            workspace.path(),
            &[ProjectProjectionFile::write("retired.md", GENERATED)],
        )
        .unwrap();
        apply_project_projection(&initial).unwrap();
        let plan = plan_project_projection(workspace.path(), &[]).unwrap();
        let error = apply_project_projection_with_hooks(
            &plan,
            &mut ProjectApplyHooks {
                before_manifest_commit: Some(&mut || {
                    std::fs::write(workspace.path().join("retired.md"), UNOWNED)
                        .map_err(|error| error.to_string())?;
                    Err("injected manifest failure".to_string())
                }),
                ..ProjectApplyHooks::noop()
            },
        )
        .unwrap_err();
        assert!(error.contains("risk-escalated"), "{error}");
        assert_eq!(
            std::fs::read(workspace.path().join("retired.md")).unwrap(),
            UNOWNED
        );
    }

    #[test]
    #[cfg(unix)]
    fn manifest_commit_revalidates_all_files_before_and_after_commit() {
        let workspace = tempfile::tempdir().unwrap();
        let initial = plan_project_projection(
            workspace.path(),
            &[ProjectProjectionFile::write("stable.md", GENERATED)],
        )
        .unwrap();
        apply_project_projection(&initial).unwrap();
        let previous_manifest =
            std::fs::read(workspace.path().join(OWNERSHIP_RELATIVE_PATH)).unwrap();
        let plan = plan_project_projection(
            workspace.path(),
            &[
                ProjectProjectionFile::write("stable.md", GENERATED),
                ProjectProjectionFile::write("new.md", PRISTINE),
            ],
        )
        .unwrap();
        let error = apply_project_projection_with_hooks(
            &plan,
            &mut ProjectApplyHooks {
                before_manifest_commit: Some(&mut || {
                    std::fs::write(workspace.path().join("stable.md"), MODIFIED)
                        .map_err(|error| error.to_string())
                }),
                ..ProjectApplyHooks::noop()
            },
        )
        .unwrap_err();
        assert!(error.contains("before manifest"), "{error}");
        assert_eq!(
            std::fs::read(workspace.path().join(OWNERSHIP_RELATIVE_PATH)).unwrap(),
            previous_manifest
        );
        assert!(!workspace.path().join("new.md").exists());
        assert_eq!(
            std::fs::read(workspace.path().join("stable.md")).unwrap(),
            MODIFIED
        );

        let repair = plan_project_projection(
            workspace.path(),
            &[ProjectProjectionFile::write("stable.md", MODIFIED)],
        )
        .unwrap();
        let error = apply_project_projection_with_hooks(
            &repair,
            &mut ProjectApplyHooks {
                after_manifest_commit: Some(&mut || {
                    std::fs::write(workspace.path().join("stable.md"), UNOWNED)
                        .map_err(|error| error.to_string())
                }),
                ..ProjectApplyHooks::noop()
            },
        )
        .unwrap_err();
        assert!(
            error.contains("after manifest") || error.contains("risk-escalated"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(workspace.path().join("stable.md")).unwrap(),
            UNOWNED
        );
    }

    #[test]
    fn ownership_manifest_rejects_unknown_fields_noncanonical_paths_and_weak_digests() {
        for manifest in [
            serde_json::json!({
                "schema_version": OWNERSHIP_SCHEMA,
                "unexpected": true,
                "entries": {}
            }),
            serde_json::json!({
                "schema_version": OWNERSHIP_SCHEMA,
                "entries": {"a//b": {"last_applied_sha256": format!("sha256:{}", "a".repeat(64))}}
            }),
            serde_json::json!({
                "schema_version": OWNERSHIP_SCHEMA,
                "entries": {"owned": {"last_applied_sha256": "sha256:abc", "owner": "suite"}}
            }),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            std::fs::create_dir(workspace.path().join(".ags")).unwrap();
            std::fs::write(
                workspace.path().join(OWNERSHIP_RELATIVE_PATH),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            assert!(
                plan_project_projection(workspace.path(), &[]).is_err(),
                "{manifest}"
            );
        }
    }

    #[test]
    fn owned_unowned_and_modified_apply_matrix_is_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        let owned_hash = ags_platform::sha256(GENERATED);

        std::fs::write(workspace.path().join("owned.md"), GENERATED).unwrap();
        let owned =
            plan_projection_migration(workspace.path(), Path::new("owned.md"), Some(&owned_hash))
                .unwrap();
        assert_eq!(owned.disposition(), MigrationDisposition::ReclaimExactOwned);
        let receipt = apply_projection_migration(&owned, b"replacement\n").unwrap();
        assert!(receipt.changed);
        assert_eq!(
            std::fs::read(workspace.path().join("owned.md")).unwrap(),
            b"replacement\n"
        );

        std::fs::write(workspace.path().join("unowned.md"), UNOWNED).unwrap();
        let unowned =
            plan_projection_migration(workspace.path(), Path::new("unowned.md"), None).unwrap();
        assert_eq!(unowned.disposition(), MigrationDisposition::PreserveUnowned);
        let receipt = apply_projection_migration(&unowned, b"replacement\n").unwrap();
        assert!(!receipt.changed);
        assert_eq!(
            std::fs::read(workspace.path().join("unowned.md")).unwrap(),
            UNOWNED
        );

        std::fs::write(workspace.path().join("modified.md"), MODIFIED).unwrap();
        let modified = plan_projection_migration(
            workspace.path(),
            Path::new("modified.md"),
            Some(&owned_hash),
        )
        .unwrap();
        assert_eq!(
            modified.disposition(),
            MigrationDisposition::PreserveModified
        );
        let receipt = apply_projection_migration(&modified, b"replacement\n").unwrap();
        assert!(!receipt.changed);
        assert_eq!(
            std::fs::read(workspace.path().join("modified.md")).unwrap(),
            MODIFIED
        );
    }

    #[test]
    fn path_escape_absolute_directory_and_symlink_are_rejected() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert!(
            plan_projection_migration(workspace.path(), Path::new("../outside"), None).is_err()
        );
        assert!(plan_projection_migration(workspace.path(), outside.path(), None).is_err());
        std::fs::create_dir(workspace.path().join("directory")).unwrap();
        assert!(plan_projection_migration(workspace.path(), Path::new("directory"), None).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("target"),
                workspace.path().join("link"),
            )
            .unwrap();
            assert!(plan_projection_migration(workspace.path(), Path::new("link"), None).is_err());
        }
    }

    #[test]
    #[cfg(unix)]
    fn descriptor_read_rejects_final_and_parent_symlink_swaps() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("external.md");
        std::fs::write(&external, UNOWNED).unwrap();

        let target = workspace.path().join("target.md");
        std::fs::write(&target, GENERATED).unwrap();
        let error = read_projection_file_inner(workspace.path(), Path::new("target.md"), || {
            std::fs::remove_file(&target).map_err(|error| error.to_string())?;
            std::os::unix::fs::symlink(&external, &target).map_err(|error| error.to_string())
        })
        .unwrap_err();
        assert!(
            error.contains("symlink") || error.contains("follow"),
            "{error}"
        );
        assert_eq!(std::fs::read(&external).unwrap(), UNOWNED);

        let parent = workspace.path().join("parent");
        let retained = workspace.path().join("retained-parent");
        let decoy = outside.path().join("decoy");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&decoy).unwrap();
        std::fs::write(parent.join("owned.md"), GENERATED).unwrap();
        std::fs::write(decoy.join("owned.md"), UNOWNED).unwrap();
        let error =
            read_projection_file_inner(workspace.path(), Path::new("parent/owned.md"), || {
                std::fs::rename(&parent, &retained).map_err(|error| error.to_string())?;
                std::os::unix::fs::symlink(&decoy, &parent).map_err(|error| error.to_string())
            })
            .unwrap_err();
        assert!(
            error.contains("parent") || error.contains("binding"),
            "{error}"
        );
        assert_eq!(std::fs::read(decoy.join("owned.md")).unwrap(), UNOWNED);
    }

    #[test]
    fn large_unowned_and_modified_files_are_hashed_without_retaining_rollback_bytes() {
        let large = vec![b'x'; MAX_ROLLBACK_BYTES + 1];

        let unowned_workspace = tempfile::tempdir().unwrap();
        std::fs::write(unowned_workspace.path().join("user.bin"), &large).unwrap();
        let unowned = plan_project_projection(
            unowned_workspace.path(),
            &[ProjectProjectionFile::write("user.bin", b"desired")],
        )
        .unwrap();
        assert_eq!(
            unowned.entries[0].disposition,
            ProjectProjectionDisposition::PreserveUnowned
        );
        assert!(unowned.entries[0].previous_bytes.is_none());

        let modified_workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(modified_workspace.path().join(".ags")).unwrap();
        std::fs::write(modified_workspace.path().join("owned.bin"), &large).unwrap();
        let manifest = OwnershipManifest {
            schema_version: OWNERSHIP_SCHEMA.to_string(),
            entries: BTreeMap::from([(
                "owned.bin".to_string(),
                OwnershipEntry {
                    last_applied_sha256: ags_platform::sha256(b"old generated"),
                },
            )]),
        };
        std::fs::write(
            modified_workspace.path().join(OWNERSHIP_RELATIVE_PATH),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let modified = plan_project_projection(
            modified_workspace.path(),
            &[ProjectProjectionFile::write("owned.bin", b"desired")],
        )
        .unwrap();
        assert_eq!(
            modified.entries[0].disposition,
            ProjectProjectionDisposition::PreserveModified
        );
        assert!(modified.entries[0].previous_bytes.is_none());
    }

    #[test]
    fn apply_rechecks_bytes_and_symlink_toctou() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = workspace.path().join("target.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("target.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        std::fs::write(&target, b"raced bytes\n").unwrap();
        assert!(apply_projection_migration(&plan, b"replacement\n").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"raced bytes\n");

        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("target.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();
        std::fs::remove_file(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path().join("outside.md"), &target).unwrap();
        assert!(apply_projection_migration(&plan, b"replacement\n").is_err());
        assert!(!outside.path().join("outside.md").exists());
    }

    #[test]
    fn create_requires_stable_absence_and_contained_parent() {
        let workspace = tempfile::tempdir().unwrap();
        let plan = plan_projection_migration(workspace.path(), Path::new("new.md"), None).unwrap();
        assert_eq!(plan.disposition(), MigrationDisposition::Create);
        let receipt = apply_projection_migration(&plan, b"new\n").unwrap();
        assert!(receipt.changed);
        assert_eq!(
            std::fs::read(workspace.path().join("new.md")).unwrap(),
            b"new\n"
        );
    }

    #[test]
    fn tampered_unowned_disposition_cannot_reclaim_user_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("user.md");
        std::fs::write(&target, UNOWNED).unwrap();
        let mut plan =
            plan_projection_migration(workspace.path(), Path::new("user.md"), None).unwrap();
        assert_eq!(plan.disposition(), MigrationDisposition::PreserveUnowned);
        plan.disposition = MigrationDisposition::ReclaimExactOwned;

        let error = apply_projection_migration(&plan, b"replacement\n").unwrap_err();
        assert!(error.contains("tampered"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), UNOWNED);
    }

    #[test]
    #[cfg(unix)]
    fn final_symlink_substitution_is_rejected_without_touching_outside_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        let outside_target = outside.path().join("outside.md");
        std::fs::write(&target, GENERATED).unwrap();
        std::fs::write(&outside_target, b"outside\n").unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();

        let result = apply_projection_migration_inner(&plan, b"replacement\n", || {
            std::fs::remove_file(&target).map_err(|error| error.to_string())?;
            std::os::unix::fs::symlink(&outside_target, &target).map_err(|error| error.to_string())
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&outside_target).unwrap(), b"outside\n");
        assert_eq!(std::fs::read_link(&target).unwrap(), outside_target);
    }

    #[test]
    #[cfg(unix)]
    fn final_regular_file_substitution_is_exchanged_back_unchanged() {
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("owned.md");
        let original = workspace.path().join("original.md");
        std::fs::write(&target, GENERATED).unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();

        let result = apply_projection_migration_inner(&plan, b"replacement\n", || {
            std::fs::rename(&target, &original).map_err(|error| error.to_string())?;
            std::fs::write(&target, b"new user bytes\n").map_err(|error| error.to_string())
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"new user bytes\n");
        assert_eq!(std::fs::read(&original).unwrap(), GENERATED);
    }

    #[test]
    #[cfg(unix)]
    fn final_parent_rename_and_symlink_substitution_rolls_back_outside() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let parent = workspace.path().join("projection");
        let moved_parent = outside.path().join("moved-projection");
        let decoy_parent = outside.path().join("decoy");
        std::fs::create_dir(&parent).unwrap();
        std::fs::create_dir(&decoy_parent).unwrap();
        std::fs::write(parent.join("owned.md"), GENERATED).unwrap();
        std::fs::write(decoy_parent.join("owned.md"), b"outside decoy\n").unwrap();
        let plan = plan_projection_migration(
            workspace.path(),
            Path::new("projection/owned.md"),
            Some(&ags_platform::sha256(GENERATED)),
        )
        .unwrap();

        let result = apply_projection_migration_inner(&plan, b"replacement\n", || {
            std::fs::rename(&parent, &moved_parent).map_err(|error| error.to_string())?;
            std::os::unix::fs::symlink(&decoy_parent, &parent).map_err(|error| error.to_string())
        });

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(moved_parent.join("owned.md")).unwrap(),
            GENERATED
        );
        assert_eq!(
            std::fs::read(decoy_parent.join("owned.md")).unwrap(),
            b"outside decoy\n"
        );
        assert_eq!(std::fs::read_link(&parent).unwrap(), decoy_parent);
    }
}
