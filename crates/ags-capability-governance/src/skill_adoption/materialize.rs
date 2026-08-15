#![allow(clippy::unnecessary_cast, clippy::useless_conversion)] // stat field widths differ per platform
use super::model::{
    AdoptionContext, MaterializedBodyNode, MaterializedSkillChange, PreparedSkillChange,
    ReadInputSeal, RiskAcknowledgements,
};
#[cfg(unix)]
use super::model::{
    InstalledSkillIndex, InstalledSkillRecord, MaterializedBodyDisposition, MaterializedBodyTree,
    MaterializedDirectory, MaterializedRegularFile, MaterializedSnapshot, MaterializedSymlink,
    ReadInputIdentity, ReadInputKind,
};
#[cfg(unix)]
use super::store::{body_path, installed_skill_index_path, observe_installed_skills};
#[cfg(unix)]
use super::transaction::{
    ensure_plan_cas_against, ensure_risks_acknowledged, validate_candidate_path,
};
#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::Read;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use crate::shared_skill_source::{
    observe_bounded_regular_file, observe_skill_source, ObservedKind, SourcePolicy, MAX_FILE_BYTES,
};

#[cfg(unix)]
const MAX_CANDIDATE_SIDE_INPUT_BYTES: u64 = 64 * 1024;

#[cfg(unix)]
use rustix::fs::{AtFlags, FileType, Mode, OFlags, Stat};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(all(test, unix))]
use std::os::unix::fs::PermissionsExt;

#[cfg(all(test, unix))]
thread_local! {
    static AFTER_FILE_PREIMAGE_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_REGISTRY_LOGICAL_CAS_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_ABSENT_PARENT_OBSERVATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone)]
pub(super) struct SkillSourceSnapshot {
    #[cfg(unix)]
    pub root_mode: u32,
    pub nodes: Vec<MaterializedBodyNode>,
    pub seals: Vec<ReadInputSeal>,
    #[cfg(unix)]
    pub manifest_hash: String,
    pub source_hash: String,
}

pub(super) fn seal_candidate_tree(root: &Path) -> Result<Vec<ReadInputSeal>, String> {
    snapshot_skill_source(root).map(|tree| tree.seals)
}

pub(super) fn snapshot_skill_source(root: &Path) -> Result<SkillSourceSnapshot, String> {
    scan_candidate_tree(root, true, ModePolicy::Desired)
}

#[cfg(unix)]
pub(super) fn snapshot_skill_source_at(
    root: &crate::shared_skill_source::DescriptorRoot,
    relative_path: &Path,
) -> Result<SkillSourceSnapshot, String> {
    let observed = crate::shared_skill_source::observe_skill_source_at(
        root,
        relative_path,
        SourcePolicy::Strict,
    )?;
    snapshot_from_observation(
        observed,
        root.path().join(relative_path),
        ModePolicy::Desired,
    )
}

#[cfg(unix)]
fn snapshot_installed_body(root: &Path) -> Result<SkillSourceSnapshot, String> {
    scan_candidate_tree(root, true, ModePolicy::Observed)
}

#[derive(Clone, Copy)]
enum ModePolicy {
    Desired,
    #[cfg(unix)]
    Observed,
}

/// Compute the complete post-state of a Skill change without mutating any
/// registry, body, host index or snapshot path. The returned bytes are the
/// sole mutation input consumed by the outer control-plane transaction.
pub fn materialize_skill_change(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    acknowledgements: &RiskAcknowledgements,
) -> Result<MaterializedSkillChange, String> {
    #[cfg(not(unix))]
    {
        let _ = (context, plan, acknowledgements);
        return Err("descriptor_semantics_unavailable_for_skill_materialization".to_string());
    }

    #[cfg(unix)]
    materialize_skill_change_unix(context, plan, acknowledgements)
}

#[cfg(unix)]
fn materialize_skill_change_unix(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    acknowledgements: &RiskAcknowledgements,
) -> Result<MaterializedSkillChange, String> {
    let registry_path = installed_skill_index_path(&context.runtime_home);
    let mut structural_read_inputs = Vec::new();
    let mut all_parent_directories = BTreeSet::new();
    collect_parent_observation(
        &context.runtime_home,
        &registry_path,
        &mut all_parent_directories,
        &mut structural_read_inputs,
    )?;
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    if plan.registry_read_input.as_ref() != Some(&observed_registry.seal)
        || plan.registry_semantic_hash != observed_registry.semantic_hash
        || ags_platform::sha256(&observed_registry.canonical_bytes)
            != observed_registry.semantic_hash
    {
        return Err("stale_plan_registry_revision: registry seal drift".to_string());
    }
    let mut registry_artifact = MaterializedRegularFile {
        path: registry_path.to_string_lossy().into_owned(),
        pre_bytes: observed_registry.raw_bytes.clone(),
        post_bytes: Vec::new(),
        pre_mode: observed_registry
            .raw_bytes
            .as_ref()
            .map(|_| observed_registry.seal.mode),
        post_mode: 0o600,
    };
    let mut registry = ensure_plan_cas_against(context, plan, observed_registry.value)?;
    #[cfg(test)]
    AFTER_REGISTRY_LOGICAL_CAS_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });

    let (body, post_target) = match plan.operation.as_str() {
        "install" | "update" => {
            ensure_risks_acknowledged(plan, acknowledgements)?;
            let candidate_path = plan
                .candidate_path
                .as_deref()
                .ok_or_else(|| "install plan has no candidate path".to_string())?;
            let candidate_path = Path::new(candidate_path);
            if let Some(resolved) = &plan.resolved_source {
                validate_candidate_path(context, candidate_path, &resolved.candidate_identity)?;
            }
            // Typed tree validation precedes plan-seal comparison so unsafe
            // nodes and exceeded budgets retain their precise error class.
            let candidate = snapshot_skill_source(candidate_path)?;
            revalidate_candidate_read_inputs(&candidate.seals, &plan.candidate_read_inputs)?;
            if candidate.source_hash != plan.source_hash
                || candidate.source_hash != plan.body_hash
                || plan
                    .target_record
                    .as_ref()
                    .map(|record| record.source_hash.as_str())
                    != Some(candidate.source_hash.as_str())
            {
                return Err("candidate_source_hash_drift_after_plan".to_string());
            }
            let record = plan
                .target_record
                .clone()
                .ok_or_else(|| "skill plan has no sealed target record".to_string())?;
            if record.skill_id != plan.skill_id
                || record.source_hash != plan.source_hash
                || record.license_hash != plan.license_hash
                || record.routing_metadata_hash != plan.routing_metadata_hash
                || record.target_hosts != plan.target_hosts
                || record.source_spec != plan.source_spec
                || record.resolved_source != plan.resolved_source
                || record.update_policy != plan.update_policy
                || record.catalog_review != plan.catalog_review
                || record.risk_findings != plan.risk_findings
            {
                return Err("skill_plan_target_record_drift".to_string());
            }
            let destination = body_path(&context.runtime_home, &record);
            if destination.to_string_lossy() != plan.body_path {
                return Err("plan_body_path_drift".to_string());
            }
            registry.skills.insert(record.skill_id.clone(), record);
            let disposition = match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(format!(
                        "special_file_refused: immutable body {}",
                        destination.display()
                    ));
                }
                Ok(_) => {
                    let existing = snapshot_installed_body(&destination)?;
                    if existing.root_mode != 0o755 || existing.nodes != candidate.nodes {
                        return Err(format!(
                            "immutable body mode or content drift: {}",
                            destination.display()
                        ));
                    }
                    MaterializedBodyDisposition::AlreadyExact {
                        root: destination.to_string_lossy().into_owned(),
                        manifest_hash: candidate.manifest_hash,
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let (body_parent_directories, parent_seal) =
                        absent_parent_directories(&context.runtime_home, &destination)?;
                    for directory in &body_parent_directories {
                        all_parent_directories.insert((directory.path.clone(), directory.mode));
                    }
                    structural_read_inputs.push(parent_seal);
                    MaterializedBodyDisposition::CreateExact(MaterializedBodyTree {
                        root: destination.to_string_lossy().into_owned(),
                        root_mode: 0o755,
                        parent_directories: body_parent_directories,
                        nodes: candidate.nodes,
                        manifest_hash: candidate.manifest_hash,
                    })
                }
                Err(error) => {
                    return Err(format!(
                        "cannot inspect immutable body {}: {error}",
                        destination.display()
                    ));
                }
            };
            (disposition, Some(path_bytes(&destination)))
        }
        "remove" => {
            if plan.previous_record.is_none() {
                return Err("removal plan has no previous installed record".to_string());
            }
            registry.skills.remove(&plan.skill_id);
            (
                MaterializedBodyDisposition::UnchangedRetained {
                    root: plan.body_path.clone(),
                    manifest_hash: plan.body_hash.clone(),
                },
                None,
            )
        }
        operation => return Err(format!("unsupported_skill_materialization: {operation}")),
    };

    registry.revision = registry.revision.saturating_add(1);
    let registry_bytes = canonical_registry_bytes(&registry)?;
    registry_artifact.post_bytes = registry_bytes;

    let mut links = Vec::new();
    for path in &plan.host_indexes {
        if post_target.is_some() {
            collect_parent_observation(
                &context.host_home,
                Path::new(path),
                &mut all_parent_directories,
                &mut structural_read_inputs,
            )?;
        }
        links.push(materialized_link(Path::new(path), post_target.clone())?);
    }
    for path in &plan.retired_host_indexes {
        links.push(materialized_link(Path::new(path), None)?);
    }
    links.sort_by(|left, right| left.path.cmp(&right.path));
    for link in &links {
        if plan.expected_link_targets.get(&link.path) != Some(&link.previous_target) {
            return Err(format!("host_link_target_drift_after_plan: {}", link.path));
        }
    }

    let candidate_snapshots = pure_skill_snapshot_overlay(context, plan, &registry)?;
    let mut snapshots = Vec::new();
    for (host, snapshot) in candidate_snapshots {
        snapshot
            .validate_integrity(&host)
            .map_err(|error| format!("invalid `{host}` candidate snapshot: {error:?}"))?;
        let snapshot_hash = snapshot.snapshot_hash.clone();
        let mut bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|error| format!("cannot serialize `{host}` snapshot: {error}"))?;
        bytes.push(b'\n');
        let snapshot_path = crate::snapshot_path(&context.runtime_home, &host);
        collect_parent_observation(
            &context.runtime_home,
            &snapshot_path,
            &mut all_parent_directories,
            &mut structural_read_inputs,
        )?;
        let file = materialized_file(&snapshot_path, bytes, 0o600)?;
        snapshots.push(MaterializedSnapshot {
            host,
            snapshot_hash,
            file,
        });
    }

    let parent_directories = all_parent_directories
        .into_iter()
        .map(|(path, mode)| MaterializedDirectory { path, mode })
        .collect::<Vec<_>>();

    let mut read_inputs = plan.candidate_read_inputs.clone();
    read_inputs.extend(structural_read_inputs);
    read_inputs.push(
        plan.registry_read_input
            .clone()
            .ok_or_else(|| "skill plan has no installed registry read-input seal".to_string())?,
    );
    read_inputs.sort_by(|left, right| {
        left.root
            .cmp(&right.root)
            .then(left.relative_path.cmp(&right.relative_path))
    });
    read_inputs.dedup();
    let materialization_hash = ags_platform::sha256(
        serde_json::to_vec(&(
            &plan.operation,
            &plan.skill_id,
            registry.revision,
            &registry_artifact,
            &parent_directories,
            &body,
            &links,
            &snapshots,
            &read_inputs,
        ))
        .map_err(|error| format!("cannot serialize Skill materialization: {error}"))?,
    );
    let final_registry = observe_installed_skills(&context.runtime_home)?;
    if final_registry.seal != observed_registry_seal(plan)? {
        return Err(
            "stale_plan_registry_revision: registry changed during materialization".to_string(),
        );
    }
    Ok(MaterializedSkillChange {
        operation: plan.operation.clone(),
        skill_id: plan.skill_id.clone(),
        registry_revision: registry.revision,
        registry: registry_artifact,
        parent_directories,
        body,
        links,
        snapshots,
        read_inputs,
        materialization_hash,
    })
}

#[cfg(unix)]
pub(super) fn pure_skill_snapshot_overlay(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    registry: &InstalledSkillIndex,
) -> Result<Vec<(String, crate::HostCapabilitySnapshot)>, String> {
    let current_index_hash = &plan.registry_semantic_hash;
    let next_index_hash = ags_platform::sha256(
        serde_json::to_vec(registry)
            .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?,
    );
    let mut snapshots = Vec::new();
    for host in &plan.target_hosts {
        let (mut snapshot, _) = crate::load_static_snapshot(&context.runtime_home, host).map_err(
            |error| match error {
                crate::SnapshotLoadError::CanonicalArtifact(
                    crate::CanonicalArtifactReadError::Unavailable(detail),
                ) => format!(
                    "snapshot_required: canonical base snapshot is missing for host `{host}`: {detail}"
                ),
                crate::SnapshotLoadError::CanonicalArtifact(
                    crate::CanonicalArtifactReadError::Refused(detail),
                ) => format!(
                    "skill_snapshot_refused: `{host}` canonical base snapshot: {detail}"
                ),
                error => format!("skill_snapshot_stale: `{host}` base: {error:?}"),
            },
        )?;
        if snapshot.installed_skill_index_hash != *current_index_hash {
            return Err(format!(
                "skill_snapshot_stale: `{host}` installed index seal does not match canonical state"
            ));
        }
        let registration = crate::load_canonical_host_registration(&context.runtime_home, host)
            .map_err(|error| format!("skill_snapshot_stale: `{host}` registration: {error:?}"))?;
        if registration.host_id.as_str() != host
            || registration.surface != snapshot.surface
            || registration.registration_hash != snapshot.host_registration_hash
        {
            return Err(format!(
                "skill_snapshot_stale: `{host}` registration seal drift"
            ));
        }

        snapshot.catalog.retain(|card| {
            card.skill_id != plan.skill_id || card.source_kind != crate::SkillSourceKind::External
        });
        snapshot
            .active_skills
            .retain(|skill| skill.skill_id != plan.skill_id);
        if plan.operation != "remove" {
            let record = registry.skills.get(&plan.skill_id).ok_or_else(|| {
                format!(
                    "skill_snapshot_stale: `{}` missing candidate record",
                    plan.skill_id
                )
            })?;
            if record.target_hosts.iter().any(|target| target == host) {
                let (card, active) = pure_installed_skill_row(record);
                snapshot.catalog.push(card);
                if let Some(active) = active {
                    snapshot.active_skills.push(active);
                }
            }
        }
        snapshot.installed_skill_index_hash = next_index_hash.clone();
        snapshot.reseal();
        snapshot
            .validate_integrity(host)
            .map_err(|error| format!("invalid `{host}` candidate snapshot: {error:?}"))?;
        snapshots.push((host.clone(), snapshot));
    }
    Ok(snapshots)
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(super) fn pure_skill_snapshot_overlay(
    _context: &AdoptionContext,
    _plan: &PreparedSkillChange,
    _registry: &super::model::InstalledSkillIndex,
) -> Result<Vec<(String, crate::HostCapabilitySnapshot)>, String> {
    Err("descriptor_semantics_unavailable_for_skill_snapshot_overlay".to_string())
}

#[cfg(unix)]
fn pure_installed_skill_row(
    record: &InstalledSkillRecord,
) -> (crate::SkillCard, Option<crate::ActiveSkill>) {
    let reasons = if record.requires_auth {
        vec!["auth_required".to_string()]
    } else {
        Vec::new()
    };
    let availability = if reasons.is_empty() {
        crate::AvailabilityState::Ready
    } else {
        crate::AvailabilityState::Unavailable {
            reason_codes: reasons.clone(),
        }
    };
    let card = crate::SkillCard {
        skill_id: record.skill_id.clone(),
        display_name: record.skill_id.clone(),
        summary: record.summary.clone(),
        intent_tags: record.intent_tags.clone(),
        positive_examples: record.positive_examples.clone(),
        negative_examples: record.negative_examples.clone(),
        entrypoints: record.entrypoints.clone(),
        routing_surface: crate::SkillRoutingSurface::SkillTarget,
        routing_hint: Some(record.invoke_hint.clone()),
        source_kind: crate::SkillSourceKind::External,
        governance: crate::GovernanceState::Active,
        availability: availability.clone(),
        reason_codes: reasons,
        requires_auth: record.requires_auth,
        auth_state: if record.requires_auth {
            crate::AuthState::Unknown
        } else {
            crate::AuthState::NotRequired
        },
        version: record.version.clone(),
        source_hash: record.source_hash.clone(),
    };
    let active = availability.is_ready().then(|| crate::ActiveSkill {
        skill_id: record.skill_id.clone(),
        invoke_hint: record.invoke_hint.clone(),
        allowed_entrypoints: record.entrypoints.clone(),
        intent_tags: record.intent_tags.clone(),
        source_hash: record.source_hash.clone(),
        body_ref: crate::SkillBodyRef::new(
            &record.skill_id,
            record.body_revision.clone(),
            record.source_hash.clone(),
        ),
    });
    (card, active)
}

#[cfg(unix)]
fn observed_registry_seal(plan: &PreparedSkillChange) -> Result<ReadInputSeal, String> {
    plan.registry_read_input
        .clone()
        .ok_or_else(|| "skill plan has no installed registry read-input seal".to_string())
}

#[cfg(unix)]
fn revalidate_candidate_read_inputs(
    tree_seals: &[ReadInputSeal],
    planned_seals: &[ReadInputSeal],
) -> Result<(), String> {
    if planned_seals.is_empty()
        || tree_seals
            .iter()
            .any(|tree_seal| !planned_seals.contains(tree_seal))
    {
        return Err("candidate_read_input_drift_after_plan".to_string());
    }
    for expected in planned_seals
        .iter()
        .filter(|seal| !tree_seals.contains(seal))
    {
        if expected.kind != ReadInputKind::RegularFile
            || expected.identity.is_none()
            || expected.relative_path.is_empty()
            || Path::new(&expected.relative_path).components().count() != 1
            || Path::new(&expected.relative_path).is_absolute()
        {
            return Err("candidate_side_input_seal_invalid".to_string());
        }
        let path = Path::new(&expected.root).join(&expected.relative_path);
        let observed = observe_bounded_regular_file(
            &path,
            MAX_CANDIDATE_SIDE_INPUT_BYTES,
            "candidate side input",
        )?;
        let actual = ReadInputSeal {
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
        if &actual != expected {
            return Err("candidate_side_input_drift_after_plan".to_string());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn collect_parent_observation(
    authorized_root: &Path,
    target: &Path,
    directories: &mut BTreeSet<(String, u32)>,
    read_inputs: &mut Vec<ReadInputSeal>,
) -> Result<(), String> {
    let (observed_directories, seal) = absent_parent_directories(authorized_root, target)?;
    directories.extend(
        observed_directories
            .into_iter()
            .map(|directory| (directory.path, directory.mode)),
    );
    read_inputs.push(seal);
    Ok(())
}

#[cfg(unix)]
fn absent_parent_directories(
    authorized_root: &Path,
    target: &Path,
) -> Result<(Vec<MaterializedDirectory>, ReadInputSeal), String> {
    let target_parent = target
        .parent()
        .ok_or_else(|| format!("materialized target has no parent: {}", target.display()))?;
    if !target_parent.starts_with(authorized_root) {
        return Err(format!(
            "materialized parent escapes authorized root {}: {}",
            authorized_root.display(),
            target_parent.display()
        ));
    }

    // The authorized root itself may be one of the directories this plan
    // creates. Hold its nearest existing ancestor, then inspect every suffix
    // component with `statat/openat` relative to held descriptors.
    let mut anchor = authorized_root;
    let anchor_fd = loop {
        match rustix::fs::open(
            anchor,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => break fd,
            Err(rustix::io::Errno::NOENT) => {
                anchor = anchor.parent().ok_or_else(|| {
                    format!(
                        "authorized root has no existing ancestor: {}",
                        authorized_root.display()
                    )
                })?;
            }
            Err(error) => {
                return Err(format!(
                    "cannot open authorized parent anchor {}: {error}",
                    anchor.display()
                ));
            }
        }
    };
    let anchor_before = rustix::fs::fstat(&anchor_fd)
        .map_err(|error| format!("cannot stat authorized parent anchor: {error}"))?;
    if FileType::from_raw_mode(anchor_before.st_mode) != FileType::Directory {
        return Err(format!(
            "special_file_refused: parent anchor {}",
            anchor.display()
        ));
    }
    let relative = target_parent.strip_prefix(anchor).map_err(|_| {
        format!(
            "materialized parent {} is outside held anchor {}",
            target_parent.display(),
            anchor.display()
        )
    })?;
    let components = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err("materialized parent suffix is not lexical-normal".to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut current_fd = anchor_fd;
    let mut current_path = anchor.to_path_buf();
    let mut held_before = anchor_before;
    let mut first_absent: Option<std::ffi::OsString> = None;
    let mut absent = Vec::new();
    for (index, component) in components.iter().enumerate() {
        let next_path = current_path.join(component);
        let parent_before = rustix::fs::fstat(&current_fd)
            .map_err(|error| format!("cannot stat held materialized parent: {error}"))?;
        match rustix::fs::statat(&current_fd, component, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(named_before) => {
                if FileType::from_raw_mode(named_before.st_mode) != FileType::Directory {
                    return Err(format!(
                        "special_file_refused: materialized parent {}",
                        next_path.display()
                    ));
                }
                let child_fd = rustix::fs::openat(
                    &current_fd,
                    component,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| {
                    format!(
                        "cannot open materialized parent {}: {error}",
                        next_path.display()
                    )
                })?;
                let opened = rustix::fs::fstat(&child_fd).map_err(|error| {
                    format!(
                        "cannot stat materialized parent {}: {error}",
                        next_path.display()
                    )
                })?;
                if stat_binding(&named_before) != stat_binding(&opened) {
                    return Err("materialized_parent_identity_drift".to_string());
                }
                let named_after =
                    rustix::fs::statat(&current_fd, component, AtFlags::SYMLINK_NOFOLLOW).map_err(
                        |error| {
                            format!(
                                "cannot revalidate materialized parent {}: {error}",
                                next_path.display()
                            )
                        },
                    )?;
                let parent_after = rustix::fs::fstat(&current_fd).map_err(|error| {
                    format!("cannot revalidate held materialized parent: {error}")
                })?;
                if stat_binding(&opened) != stat_binding(&named_after)
                    || stat_identity(&parent_before) != stat_identity(&parent_after)
                {
                    return Err("materialized_parent_identity_drift".to_string());
                }
                current_fd = child_fd;
                current_path = next_path;
                held_before = opened;
            }
            Err(rustix::io::Errno::NOENT) => {
                #[cfg(test)]
                AFTER_ABSENT_PARENT_OBSERVATION_HOOK.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().take() {
                        hook();
                    }
                });
                match rustix::fs::statat(&current_fd, component, AtFlags::SYMLINK_NOFOLLOW) {
                    Err(rustix::io::Errno::NOENT) => {}
                    _ => return Err("materialized_parent_appeared_during_observation".to_string()),
                }
                first_absent = Some(component.clone());
                let mut absent_path = current_path.clone();
                for suffix in &components[index..] {
                    absent_path.push(suffix);
                    absent.push(MaterializedDirectory {
                        path: absent_path.to_string_lossy().into_owned(),
                        mode: 0o700,
                    });
                }
                break;
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect materialized parent {}: {error}",
                    next_path.display()
                ));
            }
        }
    }

    let existing = rustix::fs::fstat(&current_fd)
        .map_err(|error| format!("cannot revalidate held materialized parent: {error}"))?;
    if stat_identity(&held_before) != stat_identity(&existing) {
        return Err("materialized_parent_identity_drift".to_string());
    }
    if let Some(first_absent) = first_absent {
        match rustix::fs::statat(&current_fd, first_absent, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {}
            _ => return Err("materialized_parent_appeared_during_observation".to_string()),
        }
    }
    let seal = seal_for_stat(&current_path, "", &existing, &[])?;
    Ok((absent, seal))
}

#[cfg(unix)]
fn canonical_registry_bytes(registry: &InstalledSkillIndex) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(registry)
        .map_err(|error| format!("cannot serialize installed Skill index: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(unix)]
fn materialized_file(
    path: &Path,
    post_bytes: Vec<u8>,
    post_mode: u32,
) -> Result<MaterializedRegularFile, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("materialized file has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("materialized file has no name: {}", path.display()))?;
    let parent_fd = match rustix::fs::open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => {
            return Ok(MaterializedRegularFile {
                path: path.to_string_lossy().into_owned(),
                pre_bytes: None,
                pre_mode: None,
                post_bytes,
                post_mode,
            });
        }
        Err(error) => {
            return Err(format!(
                "cannot open materialized file parent {}: {error}",
                parent.display()
            ));
        }
    };
    let parent_before = rustix::fs::fstat(&parent_fd)
        .map_err(|error| format!("cannot stat materialized file parent: {error}"))?;
    match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(named_before)
            if FileType::from_raw_mode(named_before.st_mode) != FileType::RegularFile =>
        {
            Err(format!(
                "special_file_refused: materialized file {}",
                path.display()
            ))
        }
        Ok(named_before) => {
            #[cfg(test)]
            AFTER_FILE_PREIMAGE_STAT_HOOK.with(|hook| {
                if let Some(hook) = hook.borrow_mut().take() {
                    hook();
                }
            });
            let fd = rustix::fs::openat(
                &parent_fd,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                format!(
                    "materialized file preimage drift or symlink {}: {error}",
                    path.display()
                )
            })?;
            let stat = rustix::fs::fstat(&fd)
                .map_err(|error| format!("cannot stat preimage fd: {error}"))?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
                || stat_binding(&named_before) != stat_binding(&stat)
            {
                return Err(format!(
                    "materialized file preimage drift: {}",
                    path.display()
                ));
            }
            let file = fs::File::from(fd);
            let mut limited = file.take(MAX_FILE_BYTES + 1);
            let mut pre = Vec::new();
            limited
                .read_to_end(&mut pre)
                .map_err(|error| format!("cannot read preimage fd: {error}"))?;
            if pre.len() as u64 > MAX_FILE_BYTES {
                return Err(format!(
                    "materialized file preimage exceeds {MAX_FILE_BYTES} bytes: {}",
                    path.display()
                ));
            }
            let file = limited.into_inner();
            let after = rustix::fs::fstat(&file)
                .map_err(|error| format!("cannot revalidate preimage fd: {error}"))?;
            if stat_binding(&stat) != stat_binding(&after) {
                return Err(format!(
                    "materialized file preimage drift: {}",
                    path.display()
                ));
            }
            let named_after = rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate preimage path: {error}"))?;
            let parent_after = rustix::fs::fstat(&parent_fd)
                .map_err(|error| format!("cannot revalidate materialized file parent: {error}"))?;
            if stat_binding(&named_after) != stat_binding(&stat)
                || stat_identity(&parent_before) != stat_identity(&parent_after)
            {
                return Err(format!(
                    "materialized file preimage drift: {}",
                    path.display()
                ));
            }
            Ok(MaterializedRegularFile {
                path: path.to_string_lossy().into_owned(),
                pre_bytes: Some(pre),
                pre_mode: Some(stat_mode(&stat)),
                post_bytes,
                post_mode,
            })
        }
        Err(rustix::io::Errno::NOENT) => {
            match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => {}
                _ => {
                    return Err(format!(
                        "materialized file appeared during observation: {}",
                        path.display()
                    ));
                }
            }
            let parent_after = rustix::fs::fstat(&parent_fd)
                .map_err(|error| format!("cannot revalidate materialized file parent: {error}"))?;
            if stat_identity(&parent_before) != stat_identity(&parent_after) {
                return Err(format!(
                    "materialized file parent drift: {}",
                    path.display()
                ));
            }
            Ok(MaterializedRegularFile {
                path: path.to_string_lossy().into_owned(),
                pre_bytes: None,
                pre_mode: None,
                post_bytes,
                post_mode,
            })
        }
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

#[cfg(unix)]
fn materialized_link(
    path: &Path,
    post_target: Option<Vec<u8>>,
) -> Result<MaterializedSymlink, String> {
    let previous_target = crate::shared_skill_source::observe_link_target(path)?;
    Ok(MaterializedSymlink {
        path: path.to_string_lossy().into_owned(),
        previous_target,
        post_target,
    })
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

fn scan_candidate_tree(
    root: &Path,
    _enforce_budgets: bool,
    mode_policy: ModePolicy,
) -> Result<SkillSourceSnapshot, String> {
    #[cfg(not(unix))]
    {
        let _ = (root, _enforce_budgets, mode_policy);
        return Err("descriptor_semantics_unavailable_for_skill_materialization".to_string());
    }

    #[cfg(unix)]
    {
        let observed = observe_skill_source(root, SourcePolicy::Strict)?;
        snapshot_from_observation(observed, root.to_path_buf(), mode_policy)
    }
}

#[cfg(unix)]
fn snapshot_from_observation(
    observed: crate::shared_skill_source::SourceObservation,
    root: PathBuf,
    mode_policy: ModePolicy,
) -> Result<SkillSourceSnapshot, String> {
    if observed.root_kind != ObservedKind::Directory {
        return Err("symlink_refused: candidate root".to_string());
    }
    let mut nodes = observed
        .nodes
        .iter()
        .map(|node| match node.kind {
            ObservedKind::Directory => Ok(MaterializedBodyNode::Directory {
                relative_path: node.relative_path.clone(),
                mode: match mode_policy {
                    ModePolicy::Desired => 0o755,
                    ModePolicy::Observed => node.mode,
                },
            }),
            ObservedKind::RegularFile => Ok(MaterializedBodyNode::RegularFile {
                relative_path: node.relative_path.clone(),
                bytes: node.bytes.clone(),
                mode: match mode_policy {
                    ModePolicy::Desired if node.mode & 0o111 != 0 => 0o700,
                    ModePolicy::Desired => 0o600,
                    ModePolicy::Observed => node.mode,
                },
            }),
            ObservedKind::Symlink => Err(format!("symlink_refused: {}", node.relative_path)),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut seals = vec![ReadInputSeal {
        root: root.to_string_lossy().into_owned(),
        relative_path: String::new(),
        kind: ReadInputKind::Directory,
        mode: observed.root_mode,
        identity: Some(ReadInputIdentity {
            device: observed.root_device,
            inode: observed.root_inode,
        }),
        digest: ags_platform::sha256([]),
    }];
    seals.extend(observed.nodes.iter().map(|node| ReadInputSeal {
        root: root.to_string_lossy().into_owned(),
        relative_path: node.relative_path.clone(),
        kind: match node.kind {
            ObservedKind::Directory => ReadInputKind::Directory,
            ObservedKind::RegularFile => ReadInputKind::RegularFile,
            ObservedKind::Symlink => ReadInputKind::Symlink,
        },
        mode: node.mode,
        identity: Some(ReadInputIdentity {
            device: node.device,
            inode: node.inode,
        }),
        digest: ags_platform::sha256(&node.bytes),
    }));
    nodes.sort_by(|left, right| body_node_path(left).cmp(body_node_path(right)));
    seals.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let manifest_hash = ags_platform::sha256(
        serde_json::to_vec(&nodes)
            .map_err(|error| format!("cannot serialize body manifest: {error}"))?,
    );
    Ok(SkillSourceSnapshot {
        root_mode: match mode_policy {
            ModePolicy::Desired => 0o755,
            ModePolicy::Observed => observed.root_mode,
        },
        nodes,
        seals,
        manifest_hash,
        source_hash: observed.source_hash,
    })
}

#[cfg(unix)]
fn seal_for_stat(
    root: &Path,
    relative_path: &str,
    stat: &Stat,
    bytes: &[u8],
) -> Result<ReadInputSeal, String> {
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let kind = if file_type == FileType::Directory {
        ReadInputKind::Directory
    } else if file_type == FileType::RegularFile {
        ReadInputKind::RegularFile
    } else if file_type == FileType::Symlink {
        ReadInputKind::Symlink
    } else {
        return Err(format!("special_file_refused: {relative_path}"));
    };
    Ok(ReadInputSeal {
        root: root.to_string_lossy().into_owned(),
        relative_path: relative_path.to_string(),
        kind,
        mode: stat_mode(stat),
        identity: Some(ReadInputIdentity {
            device: stat.st_dev as u64,
            inode: stat.st_ino,
        }),
        digest: ags_platform::sha256(bytes),
    })
}

#[cfg(unix)]
fn stat_binding(stat: &Stat) -> (u64, u64, u32, i128, i128, i128, i128, i128) {
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
fn stat_mode(stat: &Stat) -> u32 {
    u32::from(stat.st_mode) & 0o7777
}

#[cfg(unix)]
fn stat_identity(stat: &Stat) -> (u64, u64, u32) {
    (stat.st_dev as u64, stat.st_ino, stat.st_mode as u32)
}

#[cfg(unix)]
fn body_node_path(node: &MaterializedBodyNode) -> &str {
    match node {
        MaterializedBodyNode::Directory { relative_path, .. }
        | MaterializedBodyNode::RegularFile { relative_path, .. } => relative_path,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn adoption_context(temp: &tempfile::TempDir) -> AdoptionContext {
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

    fn local_source(temp: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
        let repository = temp.path().join("repository");
        let source = repository.join("skill");
        fs::create_dir_all(repository.join(".git")).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(repository.join("LICENSE"), b"MIT fixture license\n").unwrap();
        fs::write(
            source.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Registry CAS fixture.\n---\n"),
        )
        .unwrap();
        source
    }

    fn seed_base_snapshot(context: &AdoptionContext, host: &str) {
        let registration = ags_host_integration::HostRegistration::new(
            ags_host_integration::HostId::new(host).unwrap(),
            ags_host_integration::AgentSurface::Hybrid,
            ags_host_integration::platform_spec(host).map(|spec| spec.id.to_string()),
        );
        let registration_path = context
            .runtime_home
            .join("hosts")
            .join(host)
            .join("registration.json");
        fs::create_dir_all(registration_path.parent().unwrap()).unwrap();
        fs::write(
            registration_path,
            serde_json::to_vec_pretty(&registration).unwrap(),
        )
        .unwrap();
        let snapshot = crate::build_capability_snapshot_with_roots(
            &context.authority_root,
            host,
            &context.runtime_home,
            &context.host_home,
        )
        .unwrap();
        crate::publish_capability_snapshots(
            &context.runtime_home,
            vec![(host.to_string(), snapshot)],
        )
        .unwrap();
    }

    #[test]
    fn registry_parse_cas_and_preimage_must_come_from_one_held_fd() {
        let temp = tempfile::TempDir::new().unwrap();
        let context = adoption_context(&temp);
        seed_base_snapshot(&context, "codex");
        let source = local_source(&temp, "registry-cas-team");
        let plan = super::super::transaction::plan_install(
            &context,
            &super::super::model::SourceSpec::Local {
                path: source.to_string_lossy().into_owned(),
            },
            None,
            &["codex".to_string()],
            super::super::model::UpdatePolicy::Notify,
        )
        .unwrap();
        let registry_path = installed_skill_index_path(&context.runtime_home);
        AFTER_REGISTRY_LOGICAL_CAS_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                let drift = InstalledSkillIndex {
                    revision: 41,
                    ..InstalledSkillIndex::default()
                };
                fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
                let mut bytes = serde_json::to_vec_pretty(&drift).unwrap();
                bytes.push(b'\n');
                fs::write(registry_path, bytes).unwrap();
            }));
        });
        let acknowledgements = plan
            .risk_findings
            .iter()
            .map(|finding| finding.code.clone())
            .collect::<RiskAcknowledgements>();

        let error = materialize_skill_change(&context, &plan, &acknowledgements)
            .expect_err("registry changed after logical CAS must fail closed");
        assert!(error.contains("stale_plan_registry_revision"), "{error}");
    }

    #[test]
    fn file_preimage_stat_to_symlink_substitution_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("registry.json");
        let alternate = temp.path().join("alternate.json");
        fs::write(&target, b"old-registry").unwrap();
        fs::write(&alternate, b"new-registry").unwrap();
        let target_for_hook = target.clone();
        AFTER_FILE_PREIMAGE_STAT_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(&target_for_hook).unwrap();
                symlink("alternate.json", &target_for_hook).unwrap();
            }));
        });
        let error = materialized_file(&target, b"post".to_vec(), 0o600).unwrap_err();
        assert!(
            error.contains("drift") || error.contains("symlink"),
            "{error}"
        );
    }

    #[test]
    fn growing_regular_file_is_rejected_by_read_budget_not_only_stat_drift() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("SKILL.md"), b"small").unwrap();
        let growing = temp.path().join("SKILL.md");
        crate::shared_skill_source::set_after_named_stat_hook(Box::new(move || {
            fs::write(growing, vec![0_u8; MAX_FILE_BYTES as usize + 1]).unwrap();
        }));
        let error = match scan_candidate_tree(temp.path(), true, ModePolicy::Desired) {
            Ok(_) => panic!("growing file unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(error.contains(&MAX_FILE_BYTES.to_string()), "{error}");
    }

    #[test]
    fn materialize_link_rejects_replacement_after_named_stat() {
        let temp = tempfile::TempDir::new().unwrap();
        let old_target = temp.path().join("old");
        let new_target = temp.path().join("new");
        let link = temp.path().join("link");
        fs::create_dir_all(&old_target).unwrap();
        fs::create_dir_all(&new_target).unwrap();
        symlink(&old_target, &link).unwrap();
        let link_for_hook = link.clone();
        crate::shared_skill_source::set_after_link_named_stat_hook(Box::new(move || {
            fs::remove_file(&link_for_hook).unwrap();
            symlink(&new_target, &link_for_hook).unwrap();
        }));
        assert!(
            materialized_link(&link, None).is_err(),
            "materialize accepted a symlink replacement after its named stat"
        );
    }

    #[test]
    fn absent_parent_suffix_rejects_symlink_appearance_during_observation() {
        let temp = tempfile::TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let appearing = temp.path().join("a/b");
        let appearing_for_hook = appearing.clone();
        fs::create_dir_all(temp.path().join("a")).unwrap();
        AFTER_ABSENT_PARENT_OBSERVATION_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                symlink(&outside, &appearing_for_hook).unwrap();
            }));
        });
        assert!(
            absent_parent_directories(temp.path(), &appearing.join("body")).is_err(),
            "absent suffix accepted a symlink that appeared during observation"
        );
    }

    #[test]
    fn root_path_replacement_after_open_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("source");
        let moved = temp.path().join("source-old");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("SKILL.md"), b"same").unwrap();
        let root_for_hook = root.clone();
        crate::shared_skill_source::set_after_root_final_fstat_hook(Box::new(move || {
            fs::rename(&root_for_hook, &moved).unwrap();
            fs::create_dir_all(&root_for_hook).unwrap();
            fs::write(root_for_hook.join("SKILL.md"), b"same").unwrap();
        }));
        assert!(
            snapshot_skill_source(&root).is_err(),
            "root pathname replacement was not bound to the held root fd"
        );
    }

    #[test]
    fn child_directory_replacement_between_named_stat_and_open_is_rejected() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("source");
        let child = root.join("child");
        let moved = root.join("child-old");
        fs::create_dir_all(&child).unwrap();
        fs::write(child.join("leaf"), b"same").unwrap();
        let child_for_hook = child.clone();
        crate::shared_skill_source::set_after_named_stat_hook(Box::new(move || {
            fs::rename(&child_for_hook, &moved).unwrap();
            fs::create_dir_all(&child_for_hook).unwrap();
            fs::write(child_for_hook.join("leaf"), b"same").unwrap();
        }));
        assert!(
            snapshot_skill_source(&root).is_err(),
            "child replacement was accepted as the named-before directory"
        );
    }

    #[test]
    fn file_preimage_mode_must_come_from_the_opened_fd() {
        let temp = tempfile::TempDir::new().unwrap();
        let target = temp.path().join("snapshot.json");
        fs::write(&target, b"snapshot").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let target_for_hook = target.clone();
        AFTER_FILE_PREIMAGE_STAT_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::set_permissions(&target_for_hook, fs::Permissions::from_mode(0o644)).unwrap();
            }));
        });
        assert!(
            materialized_file(&target, b"post".to_vec(), 0o600).is_err(),
            "pre_mode was taken from the pathname stat instead of the opened fd"
        );
    }

    #[test]
    fn scanner_contract_is_streaming_and_name_bounded() {
        let source = include_str!("materialize.rs");
        let forbidden_collect = [".collect::<Result", "<Vec<_>, _>>()"].concat();
        assert!(!source.contains(&forbidden_collect));
        assert!(source.contains("MAX_NAME_BYTES"));
    }
}
