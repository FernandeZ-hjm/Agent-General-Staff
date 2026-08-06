use super::model::{
    AdoptionContext, BodyRevision, CatalogReviewStatus, InstalledSkillIndex, InstalledSkillRecord,
    JournalFileState, JournalLinkState, PreparedSkillChange, ResolvedSource, RiskAcknowledgements,
    RiskFinding, SkillMutationResult, SnapshotDiscovery, SourceSpec, TransactionJournal,
    TransactionPhase, UpdatePolicy, TRANSACTION_JOURNAL_SCHEMA,
};
use super::projection::{host_index_path, host_index_paths, index_points_to};
use super::remote::{acquire_remote_candidate_with_backend, GitBackend};
use super::source::{audit_local_source, audit_local_source_with_boundary};
use super::store::{
    body_path, installed_skill_index_path, load_installed_skills, write_installed_skills,
};
use crate::{
    build_capability_snapshot_with_roots, build_capability_snapshots_with_live_roots,
    hash_skill_source, publish_capability_snapshots, snapshot_path,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

type FileBackup = (PathBuf, Option<Vec<u8>>);

pub fn transaction_lock_path(runtime_home: &Path) -> PathBuf {
    ags_platform::RuntimeLayout::new(runtime_home).maintenance_lock()
}

pub fn transaction_journal_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("transactions").join("pending.json")
}

fn acquire_transaction_lock(runtime_home: &Path) -> Result<ags_platform::MaintenanceLock, String> {
    ags_platform::MaintenanceLock::acquire(runtime_home)
}

fn record_hash(record: &InstalledSkillRecord) -> Result<String, String> {
    serde_json::to_vec(record)
        .map(|bytes| ags_platform::sha256(&bytes))
        .map_err(|error| format!("cannot serialize installed skill record: {error}"))
}

fn body_identity(
    runtime_home: &Path,
    record: &InstalledSkillRecord,
) -> Result<Option<String>, String> {
    let body = body_path(runtime_home, record);
    match fs::symlink_metadata(&body) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "symlink_refused: installed body {}",
            body.display()
        )),
        Ok(metadata) if metadata.is_dir() => hash_skill_source(&body).map(Some),
        Ok(_) => Err(format!(
            "installed body is not a directory: {}",
            body.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect installed body {}: {error}",
            body.display()
        )),
    }
}

fn ensure_plan_cas(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
) -> Result<InstalledSkillIndex, String> {
    let registry = load_installed_skills(&context.runtime_home)?;
    if registry.revision != plan.registry_revision {
        return Err(format!(
            "stale_plan_registry_revision: expected {}, current {}",
            plan.registry_revision, registry.revision
        ));
    }
    let current = registry.skills.get(&plan.skill_id);
    match (&plan.previous_record, current) {
        (Some(expected), Some(actual)) => {
            let expected_hash = record_hash(expected)?;
            if plan.previous_record_hash.as_deref() != Some(expected_hash.as_str())
                || record_hash(actual)? != expected_hash
                || expected != actual
            {
                return Err("stale_plan_previous_record".to_string());
            }
        }
        (None, None) => {
            if plan.previous_record_hash.is_some() || plan.previous_body_hash.is_some() {
                return Err("stale_plan_previous_record".to_string());
            }
        }
        _ => return Err("stale_plan_previous_record".to_string()),
    }
    let current_body_hash = current
        .map(|record| body_identity(&context.runtime_home, record))
        .transpose()?
        .flatten();
    if current_body_hash != plan.previous_body_hash {
        return Err("stale_plan_previous_body".to_string());
    }
    Ok(registry)
}

/// Restore the exact pre-transaction registry, host links, snapshots, and
/// immutable-body identity recorded in the machine-local journal. Recovery
/// is explicit because a process destructor cannot observe SIGKILL or power
/// loss; repeating this function after a successful recovery is a no-op.
pub fn recover_pending_transactions(runtime_home: &Path, home: &Path) -> Result<(), String> {
    let _lock = acquire_transaction_lock(runtime_home)?;
    recover_pending_transactions_locked(runtime_home, home)
}

fn recover_pending_transactions_locked(runtime_home: &Path, home: &Path) -> Result<(), String> {
    let journal_path = transaction_journal_path(runtime_home);
    let bytes = match fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot read transaction journal {}: {error}",
                journal_path.display()
            ))
        }
    };
    let journal: TransactionJournal = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse transaction journal: {error}"))?;
    if journal.schema_version != TRANSACTION_JOURNAL_SCHEMA {
        return Err(format!(
            "unsupported transaction journal schema: {}",
            journal.schema_version
        ));
    }
    if journal.phase == TransactionPhase::Committed {
        fs::remove_file(&journal_path).map_err(|error| {
            format!(
                "cannot clear committed transaction journal {}: {error}",
                journal_path.display()
            )
        })?;
        return Ok(());
    }
    validate_journal_paths(runtime_home, home, &journal)?;
    let mut errors = Vec::new();
    if let Err(error) = restore_optional_file(
        Path::new(&journal.registry.path),
        journal.registry.bytes.as_deref(),
    ) {
        errors.push(error);
    }
    for link in &journal.links {
        if let Err(error) = restore_journal_link(link) {
            errors.push(error);
        }
    }
    for snapshot in &journal.snapshots {
        if let Err(error) =
            restore_optional_file(Path::new(&snapshot.path), snapshot.bytes.as_deref())
        {
            errors.push(error);
        }
    }
    if let Err(error) = restore_journal_body(&journal) {
        errors.push(error);
    }
    if errors.is_empty() {
        fs::remove_file(&journal_path).map_err(|error| {
            format!(
                "cannot clear recovered transaction journal {}: {error}",
                journal_path.display()
            )
        })?;
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn validate_journal_paths(
    runtime_home: &Path,
    home: &Path,
    journal: &TransactionJournal,
) -> Result<(), String> {
    let registry = Path::new(&journal.registry.path);
    if registry != installed_skill_index_path(runtime_home)
        || !safe_journal_path(registry, runtime_home)
    {
        return Err("transaction journal registry path drift".to_string());
    }
    let body = Path::new(&journal.body_path);
    let bodies_root = super::store::bodies_root(runtime_home);
    if !safe_journal_path(body, &bodies_root)
        || body == bodies_root
        || body.parent() == Some(bodies_root.as_path())
    {
        return Err("transaction journal body path escapes runtime home".to_string());
    }
    for link in &journal.links {
        if !safe_journal_path(Path::new(&link.path), home) {
            return Err("transaction journal link path escapes host home".to_string());
        }
    }
    for snapshot in &journal.snapshots {
        if !safe_journal_path(Path::new(&snapshot.path), runtime_home) {
            return Err("transaction journal snapshot path escapes runtime home".to_string());
        }
    }
    Ok(())
}

fn safe_journal_path(path: &Path, root: &Path) -> bool {
    !path
        .components()
        .any(|component| component == Component::ParentDir)
        && path.starts_with(root)
}

fn restore_journal_link(link: &JournalLinkState) -> Result<(), String> {
    let path = Path::new(&link.path);
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            return Err(format!(
                "cannot replace host directory during recovery: {}",
                path.display()
            ))
        }
        Ok(_) => fs::remove_file(path).map_err(|error| {
            format!(
                "cannot clear link {} during recovery: {error}",
                path.display()
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect link {} during recovery: {error}",
                path.display()
            ))
        }
    }
    if let Some(target) = &link.previous_target {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "cannot create link parent {} during recovery: {error}",
                    parent.display()
                )
            })?;
        }
        create_dir_symlink(Path::new(target), path)?;
    }
    Ok(())
}

fn restore_journal_body(journal: &TransactionJournal) -> Result<(), String> {
    let body = Path::new(&journal.body_path);
    let metadata = match fs::symlink_metadata(body) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot inspect body {} during recovery: {error}",
                body.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "special_file_refused: transaction body {}",
            body.display()
        ));
    }
    let actual = hash_skill_source(body)?;
    if journal.body_preexisting {
        if journal.previous_body_hash.as_deref() != Some(actual.as_str()) {
            return Err(format!(
                "preexisting immutable body changed during recovery: {}",
                body.display()
            ));
        }
    } else {
        if journal.expected_body_hash.as_deref() != Some(actual.as_str()) {
            return Err(format!(
                "new immutable body changed during recovery: {}",
                body.display()
            ));
        }
        fs::remove_dir_all(body)
            .map_err(|error| format!("cannot remove new body {}: {error}", body.display()))?;
    }
    Ok(())
}

fn persist_journal(runtime_home: &Path, journal: &TransactionJournal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| format!("cannot serialize transaction journal: {error}"))?;
    ags_platform::atomic_write(
        &transaction_journal_path(runtime_home),
        &[bytes, b"\n".to_vec()].concat(),
    )
}

fn advance_journal(
    runtime_home: &Path,
    journal: &mut TransactionJournal,
    phase: TransactionPhase,
) -> Result<(), String> {
    journal.phase = phase;
    persist_journal(runtime_home, journal)
}

struct JournalCapture {
    body_hash: Option<String>,
    previous_body_hash: Option<String>,
    body_preexisting: bool,
    registry: Option<Vec<u8>>,
    links: Vec<JournalLinkState>,
    snapshots: Vec<JournalFileState>,
}

fn new_transaction_journal(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    body: &Path,
    capture: JournalCapture,
) -> TransactionJournal {
    TransactionJournal {
        schema_version: TRANSACTION_JOURNAL_SCHEMA.to_string(),
        transaction_id: transaction_id.to_string(),
        operation: plan.operation.clone(),
        phase: TransactionPhase::Prepared,
        body_path: body.to_string_lossy().into_owned(),
        expected_body_hash: capture.body_hash,
        previous_body_hash: capture.previous_body_hash,
        body_preexisting: capture.body_preexisting,
        registry: JournalFileState {
            path: installed_skill_index_path(&context.runtime_home)
                .to_string_lossy()
                .into_owned(),
            bytes: capture.registry,
        },
        links: capture.links,
        snapshots: capture.snapshots,
    }
}

pub fn plan_removal(
    context: &AdoptionContext,
    skill_id: &str,
) -> Result<PreparedSkillChange, String> {
    let registry = load_installed_skills(&context.runtime_home)?;
    let record = registry
        .skills
        .get(skill_id)
        .ok_or_else(|| format!("skill is not installed: {skill_id}"))?;
    let body = body_path(&context.runtime_home, record);
    let mut indexes = record
        .target_hosts
        .iter()
        .flat_map(|host| host_index_paths(&context.host_home, host, skill_id))
        .collect::<Vec<_>>();
    indexes.sort();
    indexes.dedup();
    if indexes.is_empty() {
        return Err("installed Skill has no supported target Host".to_string());
    }
    for index in &indexes {
        if index.exists() && !index_points_to(index, &body) {
            return Err(format!(
                "host index no longer points at the adopted body: {}",
                index.display()
            ));
        }
    }
    let plan = PreparedSkillChange {
        operation: "remove".to_string(),
        skill_id: skill_id.to_string(),
        source: record.source.clone(),
        source_hash: record.source_hash.clone(),
        license_path: record.license_path.clone(),
        license_hash: record.license_hash.clone(),
        routing_metadata_path: record.routing_metadata_path.clone(),
        routing_metadata_hash: record.routing_metadata_hash.clone(),
        body_path: body.to_string_lossy().into_owned(),
        installed_skill_index_path: installed_skill_index_path(&context.runtime_home)
            .to_string_lossy()
            .into_owned(),
        target_hosts: record.target_hosts.clone(),
        host_indexes: indexes
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        retired_host_indexes: Vec::new(),
        planned_writes: vec![
            "unlink exact adopted host indexes".to_string(),
            "remove installed Skill index entry".to_string(),
            "refresh selected host capability snapshots".to_string(),
        ],
        warnings: vec!["immutable body is retained for recoverable rollback".to_string()],
        source_spec: record.source_spec.clone(),
        resolved_source: record.resolved_source.clone(),
        body_hash: record.source_hash.clone(),
        candidate_identity: record
            .resolved_source
            .as_ref()
            .map(|source| source.candidate_identity.clone())
            .unwrap_or_default(),
        update_policy: record.update_policy,
        catalog_review: record.catalog_review,
        risk_findings: record.risk_findings.clone(),
        candidate_path: None,
        previous_body_revision: Some(record.body_revision.clone()),
        rollback_revision: None,
        registry_revision: registry.revision,
        previous_record: Some(record.clone()),
        previous_record_hash: Some(record_hash(record)?),
        previous_body_hash: body_identity(&context.runtime_home, record)?,
    };
    Ok(plan)
}

pub fn apply_removal(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    if plan.operation != "remove" {
        return Err(format!(
            "prepared Skill change cannot remove: operation={} target={}",
            plan.operation, plan.skill_id
        ));
    }
    apply_removal_transaction(context, plan, transaction_id)
}

/// Plan an install from a typed source identity. Local and remote sources use
/// the same persisted transaction path; no third-party body enters the AGS
/// repository.
pub fn plan_install(
    context: &AdoptionContext,
    source: &SourceSpec,
    routing_metadata: Option<&Path>,
    requested_hosts: &[String],
    update_policy: UpdatePolicy,
) -> Result<PreparedSkillChange, String> {
    plan_install_with_backend(
        context,
        source,
        routing_metadata,
        requested_hosts,
        update_policy,
        &super::remote::SystemGitBackend,
    )
}

pub fn plan_install_with_backend(
    context: &AdoptionContext,
    source: &SourceSpec,
    routing_metadata: Option<&Path>,
    requested_hosts: &[String],
    update_policy: UpdatePolicy,
    backend: &dyn GitBackend,
) -> Result<PreparedSkillChange, String> {
    let mut target_hosts = normalize_hosts(requested_hosts)?;
    let existing_registry = load_installed_skills(&context.runtime_home)?;
    let (mut record, candidate_path, candidate_identity) = match source {
        SourceSpec::Local { path } => {
            let audited =
                audit_local_source(Path::new(path), target_hosts.clone(), routing_metadata)?;
            let mut record = audited.record;
            record.source_spec = SourceSpec::Local {
                path: record.source.clone(),
            };
            record.risk_findings = audited.risk_findings;
            let candidate_path = record.source.clone();
            (record, candidate_path, String::new())
        }
        SourceSpec::GitHub { .. } | SourceSpec::Git { .. } => {
            let candidate = acquire_remote_candidate_with_backend(context, source, backend)?;
            let candidate_identity = candidate.resolved_source.candidate_identity.clone();
            (
                candidate.record,
                candidate.skill_dir.to_string_lossy().into_owned(),
                candidate_identity,
            )
        }
    };
    bind_catalog_review(&context.authority_root, &mut record)?;
    if record.catalog_review == CatalogReviewStatus::Unreviewed {
        add_catalog_review_risk(&mut record.risk_findings);
    }
    reject_official_collision(&context.authority_root, &record.skill_id)?;
    let previous = existing_registry.skills.get(&record.skill_id).cloned();
    if let Some(existing) = &previous {
        target_hosts.extend(existing.target_hosts.iter().cloned());
        target_hosts.sort();
        target_hosts.dedup();
    }
    record.target_hosts = target_hosts.clone();
    record.update_policy = update_policy;
    record.installed_at = unix_time();
    if let Some(existing) = &previous {
        record.body_revisions = existing.body_revisions.clone();
    }
    let plan = build_plan(
        context,
        "install",
        &record,
        target_hosts,
        PlanBinding {
            candidate_identity,
            candidate_path: Some(candidate_path),
            previous_body_revision: previous.as_ref().map(|record| record.body_revision.clone()),
            rollback_revision: None,
            allow_legacy_authority_indexes: false,
        },
    )?;
    Ok(plan)
}

/// Seal a one-way migration of a Skill that an older suite projected directly
/// from its authority checkout. The existing body is copied into the same
/// isolated candidate store used by remote acquisition, then bound to the
/// reviewed catalog identity only when its exact body hash matches. Normal
/// readers never infer installation from the old symlink layout.
pub fn plan_legacy_catalog_migration(
    context: &AdoptionContext,
    source_dir: &Path,
    skill_id: &str,
    requested_hosts: &[String],
) -> Result<PreparedSkillChange, String> {
    let target_hosts = normalize_hosts(requested_hosts)?;
    let registry = load_installed_skills(&context.runtime_home)?;
    let manifest = crate::third_party_manifest::read_third_party_manifest(&context.authority_root)?;
    let catalog = manifest
        .capabilities
        .iter()
        .find(|capability| {
            capability.kind == crate::third_party_manifest::CapabilityKind::Skill
                && capability.id == skill_id
        })
        .ok_or_else(|| format!("retired suite Skill `{skill_id}` is absent from the catalog"))?;
    let repository = catalog
        .source
        .repository
        .clone()
        .ok_or_else(|| format!("catalog Skill `{skill_id}` has no repository"))?;
    let revision = catalog
        .source
        .revision
        .clone()
        .ok_or_else(|| format!("catalog Skill `{skill_id}` has no pinned revision"))?;
    let source_spec = SourceSpec::github(
        repository.clone(),
        Some(revision.clone()),
        catalog.source.subdir.clone(),
    )
    .with_tracking_ref(catalog.source.tracking_ref.clone());
    let source_boundary = source_dir.canonicalize().map_err(|error| {
        format!(
            "cannot resolve legacy suite Skill {}: {error}",
            source_dir.display()
        )
    })?;
    // The migration preserves exactly the already-visible Skill body. A
    // checkout-level AGS license is not evidence of the third-party body's
    // license, so audit is intentionally bounded to this Skill directory.
    let audited = audit_local_source_with_boundary(
        &source_boundary,
        target_hosts.clone(),
        None,
        Some(&source_boundary),
    )?;
    if audited.record.skill_id != skill_id {
        return Err(format!(
            "legacy suite Skill id mismatch: expected `{skill_id}`, observed `{}`",
            audited.record.skill_id
        ));
    }
    let expected_hash = catalog
        .source
        .integrity
        .as_deref()
        .ok_or_else(|| format!("catalog Skill `{skill_id}` has no reviewed body hash"))?;
    if expected_hash != audited.record.source_hash {
        return Err(format!(
            "catalog_integrity_mismatch: `{skill_id}` expected {expected_hash}, observed {}",
            audited.record.source_hash
        ));
    }
    if let Some(existing) = registry.skills.get(skill_id) {
        let existing_body = body_path(&context.runtime_home, existing);
        if !existing_body.is_dir() || hash_skill_source(&existing_body)? != existing.source_hash {
            return Err(format!(
                "installed Skill `{skill_id}` body is missing or changed before migration"
            ));
        }
        if existing.source_hash != audited.record.source_hash {
            let mut preserved = existing.clone();
            preserved.target_hosts.extend(target_hosts);
            preserved.target_hosts.sort();
            preserved.target_hosts.dedup();
            return build_plan(
                context,
                "reactivate",
                &preserved,
                preserved.target_hosts.clone(),
                PlanBinding {
                    candidate_identity: preserved
                        .resolved_source
                        .as_ref()
                        .map(|source| source.candidate_identity.clone())
                        .unwrap_or_default(),
                    candidate_path: Some(existing_body.to_string_lossy().into_owned()),
                    previous_body_revision: Some(existing.body_revision.clone()),
                    rollback_revision: None,
                    allow_legacy_authority_indexes: true,
                },
            );
        }
    }

    let candidate_identity = ags_platform::sha256(
        &serde_json::to_vec(&(
            "legacy-suite-migration",
            &source_spec,
            &audited.record.source_hash,
        ))
        .map_err(|error| format!("cannot serialize migration candidate identity: {error}"))?,
    );
    let candidate_root = context
        .runtime_home
        .join("candidates")
        .join(candidate_identity.trim_start_matches("sha256:"));
    let checkout_root = candidate_root.join("checkout");
    let subdir = catalog.source.subdir.clone().unwrap_or_default();
    let candidate_path = if subdir.is_empty() {
        checkout_root.clone()
    } else {
        checkout_root.join(&subdir)
    };
    materialize_migration_candidate(
        &audited.source_dir,
        &candidate_root,
        &checkout_root,
        &candidate_path,
        &audited.record.source_hash,
    )?;

    let resolved = ResolvedSource {
        source_spec: source_spec.clone(),
        resolved_commit: revision,
        body_hash: audited.record.source_hash.clone(),
        candidate_identity: candidate_identity.clone(),
        subdir,
    };
    let mut record = audited.record;
    record.source = source_spec
        .repository_url()
        .unwrap_or(&repository)
        .to_string();
    record.source_spec = source_spec;
    record.resolved_source = Some(resolved);
    record.update_policy = UpdatePolicy::Notify;
    record.installed_at = unix_time();
    bind_catalog_review(&context.authority_root, &mut record)?;
    if record.catalog_review != CatalogReviewStatus::Reviewed {
        return Err(format!(
            "legacy suite Skill `{skill_id}` did not bind to its reviewed catalog identity"
        ));
    }
    record.body_revisions = vec![BodyRevision::from_record(&record)];
    build_plan(
        context,
        "install",
        &record,
        target_hosts,
        PlanBinding {
            candidate_identity,
            candidate_path: Some(candidate_path.to_string_lossy().into_owned()),
            previous_body_revision: None,
            rollback_revision: None,
            allow_legacy_authority_indexes: true,
        },
    )
}

fn materialize_migration_candidate(
    source: &Path,
    candidate_root: &Path,
    checkout_root: &Path,
    candidate_path: &Path,
    expected_hash: &str,
) -> Result<(), String> {
    if candidate_path.is_dir() {
        let observed = hash_skill_source(candidate_path)?;
        return if observed == expected_hash {
            Ok(())
        } else {
            Err("cached migration candidate hash mismatch".to_string())
        };
    }
    if fs::symlink_metadata(candidate_root).is_ok() {
        return Err(format!(
            "migration candidate is incomplete or unsafe: {}",
            candidate_root.display()
        ));
    }
    let candidates_root = candidate_root
        .parent()
        .ok_or_else(|| "migration candidate has no store parent".to_string())?;
    fs::create_dir_all(candidates_root)
        .map_err(|error| format!("cannot create {}: {error}", candidates_root.display()))?;
    let stage = candidates_root.join(format!(
        ".migration-stage-{}-{}",
        std::process::id(),
        candidate_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("candidate")
    ));
    if fs::symlink_metadata(&stage).is_ok() {
        return Err(format!(
            "migration candidate stage already exists: {}",
            stage.display()
        ));
    }
    let relative = candidate_path
        .strip_prefix(checkout_root)
        .map_err(|_| "migration candidate path escapes checkout".to_string())?;
    let staged_path = stage.join("checkout").join(relative);
    let result = (|| {
        ags_platform::copy_regular_tree(source, &staged_path)?;
        if hash_skill_source(&staged_path)? != expected_hash {
            return Err("source_drift_during_migration_candidate_copy".to_string());
        }
        fs::rename(&stage, candidate_root).map_err(|error| {
            format!(
                "cannot publish migration candidate {}: {error}",
                candidate_root.display()
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

pub fn apply_install(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    acknowledgements: &RiskAcknowledgements,
) -> Result<SkillMutationResult, String> {
    apply_prepared_install(context, plan, transaction_id, acknowledgements, false)
}

/// Apply within a composite maintenance transaction that already owns the
/// process-wide MaintenanceLock. This is the sole non-locking entrypoint; it
/// still runs WAL recovery and every CAS/hash/risk check.
pub fn apply_install_in_maintenance_transaction(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    acknowledgements: &RiskAcknowledgements,
) -> Result<SkillMutationResult, String> {
    apply_prepared_install(context, plan, transaction_id, acknowledgements, true)
}

/// Repair Host activation for an already-installed record without replacing
/// its immutable body or inventing upstream provenance. Composite setup owns
/// the single snapshot refresh after all repairs complete.
pub fn apply_reactivation_in_maintenance_transaction(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    if plan.operation != "reactivate" {
        return Err(format!(
            "plan operation cannot reactivate: {}",
            plan.operation
        ));
    }
    let current = load_installed_skills(&context.runtime_home)?
        .skills
        .get(&plan.skill_id)
        .cloned()
        .ok_or_else(|| format!("skill is not installed: {}", plan.skill_id))?;
    let mut target = current.clone();
    target.target_hosts = plan.target_hosts.clone();
    let body = body_path(&context.runtime_home, &target);
    if !body.is_dir() || hash_skill_source(&body)? != target.source_hash {
        return Err("reactivation_body_hash_drift_after_plan".to_string());
    }
    apply_record_transaction_under_lock(context, plan, transaction_id, &target, &body, false, false)
}

pub fn apply_update(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    acknowledgements: &RiskAcknowledgements,
) -> Result<SkillMutationResult, String> {
    apply_prepared_install(context, plan, transaction_id, acknowledgements, false)
}

fn apply_prepared_install(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    acknowledgements: &RiskAcknowledgements,
    maintenance_lock_held: bool,
) -> Result<SkillMutationResult, String> {
    if plan.operation != "install" && plan.operation != "update" {
        return Err(format!(
            "plan operation cannot install/update: {}",
            plan.operation
        ));
    }
    ensure_risks_acknowledged(plan, acknowledgements)?;
    let candidate_path = plan
        .candidate_path
        .as_deref()
        .ok_or_else(|| "install plan has no candidate path".to_string())?;
    let candidate_path = Path::new(candidate_path);
    if let Some(resolved) = &plan.resolved_source {
        validate_candidate_path(context, candidate_path, &resolved.candidate_identity)?;
    }
    let repository_root = candidate_path
        .ancestors()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some("checkout"));
    let audited = audit_local_source_with_boundary(
        candidate_path,
        plan.target_hosts.clone(),
        plan.routing_metadata_path.as_deref().map(Path::new),
        repository_root,
    )?;
    if audited.record.source_hash != plan.source_hash
        || audited.record.license_hash != plan.license_hash
        || audited.record.routing_metadata_hash != plan.routing_metadata_hash
        || audited.record.skill_id != plan.skill_id
        || audited.record.source_hash != plan.body_hash
    {
        return Err("candidate_hash_or_source_drift_after_plan".to_string());
    }
    if let Some(resolved) = &plan.resolved_source {
        if resolved.body_hash != audited.record.source_hash {
            return Err("candidate_body_hash_drift_after_plan".to_string());
        }
    }
    let mut candidate_risks = audited.risk_findings.clone();
    if plan.catalog_review == CatalogReviewStatus::Unreviewed {
        add_catalog_review_risk(&mut candidate_risks);
    }
    if normalize_risk_findings(candidate_risks)
        != normalize_risk_findings(plan.risk_findings.clone())
    {
        return Err("candidate_risk_drift_after_plan".to_string());
    }
    let mut record = audited.record;
    record.source = plan.source.clone();
    record.source_spec = plan.source_spec.clone();
    record.resolved_source = plan.resolved_source.clone();
    record.update_policy = plan.update_policy;
    // Risk acknowledgement is an apply-time fact.  It must not mutate the
    // independent catalog-review truth carried by the plan.
    record.catalog_review = plan.catalog_review;
    record.risk_findings = plan.risk_findings.clone();
    record.target_hosts = plan.target_hosts.clone();
    record.body_revisions = merge_revision_history(
        load_installed_skills(&context.runtime_home)?
            .skills
            .get(&record.skill_id)
            .map(|existing| existing.body_revisions.clone())
            .unwrap_or_default(),
        &record,
    );
    if maintenance_lock_held {
        apply_record_transaction_under_lock(
            context,
            plan,
            transaction_id,
            &record,
            &audited.source_dir,
            true,
            false,
        )
    } else {
        apply_record_transaction(
            context,
            plan,
            transaction_id,
            &record,
            &audited.source_dir,
            true,
        )
    }
}

pub fn plan_update(
    context: &AdoptionContext,
    skill_id: &str,
) -> Result<PreparedSkillChange, String> {
    plan_update_with_backend(context, skill_id, &super::remote::SystemGitBackend)
}

pub fn plan_update_with_backend(
    context: &AdoptionContext,
    skill_id: &str,
    backend: &dyn GitBackend,
) -> Result<PreparedSkillChange, String> {
    let registry = load_installed_skills(&context.runtime_home)?;
    let existing = registry
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| format!("skill is not installed: {skill_id}"))?;
    if existing.update_policy == UpdatePolicy::Pinned {
        return Err("pinned_update_has_no_candidate".to_string());
    }
    if !existing.source_spec.is_upstream_bound() || existing.resolved_source.is_none() {
        return Err("local_source_has_no_upstream_update_candidate".to_string());
    }
    let candidate_source = existing
        .source_spec
        .tracking_candidate()
        .ok_or_else(|| "local_source_has_no_upstream_update_candidate".to_string())?;
    let candidate = acquire_remote_candidate_with_backend(context, &candidate_source, backend)?;
    let current_source = existing.resolved_source.as_ref().expect("checked above");
    if candidate.resolved_source.resolved_commit == current_source.resolved_commit
        && candidate.record.source_hash == existing.source_hash
    {
        return Err("no_update_available".to_string());
    }
    let mut record = candidate.record;
    record.target_hosts = existing.target_hosts.clone();
    record.update_policy = existing.update_policy;
    record.catalog_review = CatalogReviewStatus::Unreviewed;
    bind_catalog_review(&context.authority_root, &mut record)?;
    if record.catalog_review == CatalogReviewStatus::Unreviewed {
        add_catalog_review_risk(&mut record.risk_findings);
    }
    record.routing_metadata_path = existing.routing_metadata_path.clone();
    record.routing_metadata_hash = existing.routing_metadata_hash.clone();
    record.body_revisions = existing.body_revisions.clone();
    let plan = build_plan(
        context,
        "update",
        &record,
        existing.target_hosts.clone(),
        PlanBinding {
            candidate_identity: candidate.resolved_source.candidate_identity.clone(),
            candidate_path: Some(candidate.skill_dir.to_string_lossy().into_owned()),
            previous_body_revision: Some(existing.body_revision),
            rollback_revision: None,
            allow_legacy_authority_indexes: false,
        },
    )?;
    Ok(plan)
}

pub fn plan_rollback(
    context: &AdoptionContext,
    skill_id: &str,
    revision: &str,
) -> Result<PreparedSkillChange, String> {
    let registry = load_installed_skills(&context.runtime_home)?;
    let existing = registry
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| format!("skill is not installed: {skill_id}"))?;
    let body_revision = existing
        .body_revisions
        .iter()
        .find(|candidate| candidate.revision == revision)
        .cloned()
        .ok_or_else(|| format!("immutable body revision does not exist: {revision}"))?;
    let target = body_revision
        .metadata
        .restore_record(existing.body_revisions.clone());
    let body = body_path(&context.runtime_home, &target);
    if !body.is_dir() || hash_skill_source(&body)? != body_revision.source_hash {
        return Err("rollback target body is missing or changed".to_string());
    }
    if existing.body_revision == revision {
        return Err("rollback target is already installed".to_string());
    }
    let plan = build_plan(
        context,
        "rollback",
        &target,
        target.target_hosts.clone(),
        PlanBinding {
            candidate_identity: target
                .resolved_source
                .as_ref()
                .map(|source| source.candidate_identity.clone())
                .unwrap_or_default(),
            candidate_path: Some(body.to_string_lossy().into_owned()),
            previous_body_revision: Some(existing.body_revision),
            rollback_revision: Some(revision.to_string()),
            allow_legacy_authority_indexes: false,
        },
    )?;
    Ok(plan)
}

pub fn apply_rollback(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    if plan.operation != "rollback" {
        return Err(format!(
            "plan operation cannot rollback: {}",
            plan.operation
        ));
    }
    let revision = plan
        .rollback_revision
        .as_deref()
        .ok_or_else(|| "rollback plan has no immutable target revision".to_string())?;
    let registry = load_installed_skills(&context.runtime_home)?;
    let current = registry
        .skills
        .get(&plan.skill_id)
        .cloned()
        .ok_or_else(|| format!("skill is not installed: {}", plan.skill_id))?;
    if plan.previous_body_revision.as_deref() != Some(current.body_revision.as_str()) {
        return Err("rollback_source_drift_after_plan".to_string());
    }
    let target_revision = current
        .body_revisions
        .iter()
        .find(|candidate| candidate.revision == revision)
        .ok_or_else(|| "rollback target revision is no longer retained".to_string())?;
    let target = target_revision
        .metadata
        .restore_record(current.body_revisions.clone());
    let body = body_path(&context.runtime_home, &target);
    if body.to_string_lossy() != plan.body_path
        || !body.is_dir()
        || hash_skill_source(&body)? != target_revision.source_hash
    {
        return Err("rollback_target_hash_or_path_drift".to_string());
    }
    apply_record_transaction(context, plan, transaction_id, &target, &body, false)
}

/// Reverse one successfully applied persisted plan. Recovery is compare-and-
/// swap bound to that plan's post-state, so it can never overwrite a later
/// install, update, rollback or removal of the same Skill.
pub fn recover_applied_change(
    context: &AdoptionContext,
    original: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    let _lock = acquire_transaction_lock(&context.runtime_home)?;
    recover_applied_change_under_lock(context, original, transaction_id, true)
}

/// Recover one Skill change while a composite transaction retains the global
/// maintenance lock. Identity/CAS checks are identical to standalone recover.
pub fn recover_applied_change_in_maintenance_transaction(
    context: &AdoptionContext,
    original: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    recover_applied_change_under_lock(context, original, transaction_id, false)
}

fn recover_applied_change_under_lock(
    context: &AdoptionContext,
    original: &PreparedSkillChange,
    transaction_id: &str,
    refresh_snapshot_state: bool,
) -> Result<SkillMutationResult, String> {
    recover_pending_transactions_locked(&context.runtime_home, &context.host_home)?;
    let registry = load_installed_skills(&context.runtime_home)?;
    let current = registry.skills.get(&original.skill_id).cloned();

    if original.operation == "remove" {
        if current.is_some() {
            return Err("recovery_refused: removal post-state is no longer current".to_string());
        }
    } else {
        let current = current.as_ref().ok_or_else(|| {
            "recovery_refused: applied Skill post-state is no longer installed".to_string()
        })?;
        if !record_matches_plan_target(current, original) {
            return Err("recovery_refused: a later Skill state replaced this plan".to_string());
        }
    }

    if let Some(mut previous) = original.previous_record.clone() {
        if let Some(current) = current.as_ref() {
            previous.body_revisions = current.body_revisions.clone();
        }
        let body = body_path(&context.runtime_home, &previous);
        if !body.is_dir() || hash_skill_source(&body)? != previous.source_hash {
            return Err("recovery_refused: previous immutable body is unavailable".to_string());
        }
        let recovery = build_plan(
            context,
            "recover",
            &previous,
            previous.target_hosts.clone(),
            PlanBinding {
                candidate_identity: previous
                    .resolved_source
                    .as_ref()
                    .map(|source| source.candidate_identity.clone())
                    .unwrap_or_default(),
                candidate_path: Some(body.to_string_lossy().into_owned()),
                previous_body_revision: current.as_ref().map(|record| record.body_revision.clone()),
                rollback_revision: Some(previous.body_revision.clone()),
                allow_legacy_authority_indexes: false,
            },
        )?;
        apply_record_transaction_under_lock(
            context,
            &recovery,
            transaction_id,
            &previous,
            &body,
            false,
            refresh_snapshot_state,
        )
    } else {
        let current = current.ok_or_else(|| {
            "recovery_refused: install plan has no current post-state".to_string()
        })?;
        let removal = plan_removal(context, &current.skill_id)?;
        apply_removal_under_lock(context, &removal, transaction_id, refresh_snapshot_state)
    }
}

fn record_matches_plan_target(record: &InstalledSkillRecord, plan: &PreparedSkillChange) -> bool {
    record.skill_id == plan.skill_id
        && record.source_hash == plan.source_hash
        && record.license_hash == plan.license_hash
        && record.routing_metadata_hash == plan.routing_metadata_hash
        && record.target_hosts == plan.target_hosts
        && record.source_spec == plan.source_spec
        && record.resolved_source == plan.resolved_source
        && record.update_policy == plan.update_policy
        && record.catalog_review == plan.catalog_review
        && normalize_risk_findings(record.risk_findings.clone())
            == normalize_risk_findings(plan.risk_findings.clone())
}

fn apply_record_transaction(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    record: &InstalledSkillRecord,
    source_dir: &Path,
    install_body_content: bool,
) -> Result<SkillMutationResult, String> {
    let _lock = acquire_transaction_lock(&context.runtime_home)?;
    apply_record_transaction_under_lock(
        context,
        plan,
        transaction_id,
        record,
        source_dir,
        install_body_content,
        true,
    )
}

fn apply_record_transaction_under_lock(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    record: &InstalledSkillRecord,
    source_dir: &Path,
    install_body_content: bool,
    refresh_snapshot_state: bool,
) -> Result<SkillMutationResult, String> {
    recover_pending_transactions_locked(&context.runtime_home, &context.host_home)?;
    let registry = ensure_plan_cas(context, plan)?;
    apply_transaction_locked(
        context,
        plan,
        transaction_id,
        TransactionWrite {
            registry,
            record: Some(record),
            source_dir: Some(source_dir),
            install_body_content,
            refresh_snapshot_state,
        },
    )
}

fn apply_removal_transaction(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
) -> Result<SkillMutationResult, String> {
    let _lock = acquire_transaction_lock(&context.runtime_home)?;
    apply_removal_under_lock(context, plan, transaction_id, true)
}

fn apply_removal_under_lock(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    refresh_snapshot_state: bool,
) -> Result<SkillMutationResult, String> {
    recover_pending_transactions_locked(&context.runtime_home, &context.host_home)?;
    let registry = ensure_plan_cas(context, plan)?;
    apply_transaction_locked(
        context,
        plan,
        transaction_id,
        TransactionWrite {
            registry,
            record: None,
            source_dir: None,
            install_body_content: false,
            refresh_snapshot_state,
        },
    )
}

struct TransactionWrite<'a> {
    registry: InstalledSkillIndex,
    record: Option<&'a InstalledSkillRecord>,
    source_dir: Option<&'a Path>,
    install_body_content: bool,
    refresh_snapshot_state: bool,
}

fn apply_transaction_locked(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    transaction_id: &str,
    write: TransactionWrite<'_>,
) -> Result<SkillMutationResult, String> {
    let TransactionWrite {
        mut registry,
        record,
        source_dir,
        install_body_content,
        refresh_snapshot_state,
    } = write;
    let body = record
        .map(|record| body_path(&context.runtime_home, record))
        .unwrap_or_else(|| PathBuf::from(&plan.body_path));
    if body.to_string_lossy() != plan.body_path {
        return Err("plan_body_path_drift".to_string());
    }
    let registry_file = installed_skill_index_path(&context.runtime_home);
    let registry_backup = read_optional(&registry_file)?;
    let mut transaction_indexes = plan.host_indexes.clone();
    transaction_indexes.extend(plan.retired_host_indexes.iter().cloned());
    transaction_indexes.sort();
    transaction_indexes.dedup();
    let link_backups = capture_links(&transaction_indexes)?;
    let snapshot_backups = if refresh_snapshot_state {
        capture_snapshots(&context.runtime_home, &plan.target_hosts)?
    } else {
        Vec::new()
    };
    let body_preexisting = fs::symlink_metadata(&body).is_ok();
    let previous_body_hash = if body_preexisting {
        let metadata = fs::symlink_metadata(&body).map_err(|error| {
            format!(
                "cannot inspect transaction body {}: {error}",
                body.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "special_file_refused: transaction body {}",
                body.display()
            ));
        }
        Some(hash_skill_source(&body)?)
    } else {
        None
    };
    let expected_body_hash = record.map(|record| record.source_hash.as_str());
    let mut journal = new_transaction_journal(
        context,
        plan,
        transaction_id,
        &body,
        JournalCapture {
            body_hash: expected_body_hash.map(str::to_string),
            previous_body_hash,
            body_preexisting,
            registry: registry_backup.clone(),
            links: link_backups
                .iter()
                .map(|(path, target)| JournalLinkState {
                    path: path.to_string_lossy().into_owned(),
                    previous_target: target
                        .as_ref()
                        .map(|target| target.to_string_lossy().into_owned()),
                })
                .collect(),
            snapshots: snapshot_backups
                .iter()
                .map(|(path, bytes)| JournalFileState {
                    path: path.to_string_lossy().into_owned(),
                    bytes: bytes.clone(),
                })
                .collect(),
        },
    );
    persist_journal(&context.runtime_home, &journal)?;

    let applied = (|| -> Result<BTreeMap<String, String>, String> {
        if let Some(record) = record {
            if install_body_content {
                let source_dir =
                    source_dir.ok_or_else(|| "install transaction has no source".to_string())?;
                install_body(source_dir, &body, &record.source_hash)?;
            } else if !body.is_dir() || hash_skill_source(&body)? != record.source_hash {
                return Err("immutable rollback body is unavailable".to_string());
            }
            advance_journal(
                &context.runtime_home,
                &mut journal,
                TransactionPhase::BodyInstalled,
            )?;
            for index in &plan.host_indexes {
                replace_symlink(Path::new(index), &body)?;
            }
            for index in &plan.retired_host_indexes {
                remove_host_index(Path::new(index))?;
            }
            registry
                .skills
                .insert(record.skill_id.clone(), record.clone());
        } else {
            for index in &plan.host_indexes {
                remove_host_index(Path::new(index))?;
            }
            registry.skills.remove(&plan.skill_id);
        }
        advance_journal(
            &context.runtime_home,
            &mut journal,
            TransactionPhase::LinksApplied,
        )?;
        registry.revision = registry.revision.saturating_add(1);
        write_installed_skills(&context.runtime_home, &registry)?;
        advance_journal(
            &context.runtime_home,
            &mut journal,
            TransactionPhase::RegistryApplied,
        )?;
        let snapshots = if refresh_snapshot_state {
            refresh_snapshots(context, &plan.target_hosts)?
        } else {
            BTreeMap::new()
        };
        advance_journal(
            &context.runtime_home,
            &mut journal,
            TransactionPhase::SnapshotsApplied,
        )?;
        Ok(snapshots)
    })();

    match applied {
        Ok(snapshots) => {
            advance_journal(
                &context.runtime_home,
                &mut journal,
                TransactionPhase::Committed,
            )?;
            fs::remove_file(transaction_journal_path(&context.runtime_home))
                .map_err(|error| format!("cannot clear committed transaction journal: {error}"))?;
            Ok(SkillMutationResult {
                operation: plan.operation.clone(),
                transaction_id: transaction_id.to_string(),
                skill_id: plan.skill_id.clone(),
                registry_revision: registry.revision,
                body_path: body.to_string_lossy().into_owned(),
                host_indexes: plan.host_indexes.clone(),
                snapshot_hashes: snapshots,
                requires_repreflight: true,
            })
        }
        Err(error) => {
            let recovery =
                recover_pending_transactions_locked(&context.runtime_home, &context.host_home);
            Err(match recovery {
                Ok(()) => error,
                Err(recovery_error) => format!("{error}; recovery failed: {recovery_error}"),
            })
        }
    }
}

struct PlanBinding {
    candidate_identity: String,
    candidate_path: Option<String>,
    previous_body_revision: Option<String>,
    rollback_revision: Option<String>,
    allow_legacy_authority_indexes: bool,
}

fn index_points_to_legacy_authority(
    context: &AdoptionContext,
    record: &InstalledSkillRecord,
    index: &Path,
) -> bool {
    let expected = context
        .authority_root
        .join("global-skills")
        .join(&record.skill_id);
    index_points_to(index, &expected)
}

fn build_plan(
    context: &AdoptionContext,
    operation: &str,
    record: &InstalledSkillRecord,
    target_hosts: Vec<String>,
    binding: PlanBinding,
) -> Result<PreparedSkillChange, String> {
    let mut indexes = target_hosts
        .iter()
        .map(|host| {
            host_index_path(&context.host_home, host, &record.skill_id)
                .ok_or_else(|| format!("unsupported skill host: {host}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    indexes.sort();
    indexes.dedup();
    let registry = load_installed_skills(&context.runtime_home)?;
    let previous_record = registry.skills.get(&record.skill_id).cloned();
    let previous_body = previous_record
        .as_ref()
        .map(|record| body_path(&context.runtime_home, record));
    for index in &indexes {
        if let Ok(metadata) = fs::symlink_metadata(index) {
            if !metadata.file_type().is_symlink() {
                return Err(format!(
                    "host index conflict is not a symlink and will not be replaced: {}",
                    index.display()
                ));
            }
            let previous_owned = previous_body
                .as_deref()
                .is_some_and(|body| index_points_to(index, body));
            let legacy_owned = binding.allow_legacy_authority_indexes
                && index_points_to_legacy_authority(context, record, index);
            if !previous_owned && !legacy_owned {
                return Err(format!(
                    "host index conflict is not owned by this installed Skill: {}",
                    index.display()
                ));
            }
        }
    }
    let canonical = indexes
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut retired_indexes = target_hosts
        .iter()
        .flat_map(|host| host_index_paths(&context.host_home, host, &record.skill_id))
        .filter(|path| !canonical.contains(path))
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect::<Vec<_>>();
    retired_indexes.sort();
    retired_indexes.dedup();
    for index in &retired_indexes {
        let metadata = fs::symlink_metadata(index).map_err(|error| {
            format!(
                "cannot inspect legacy host index {}: {error}",
                index.display()
            )
        })?;
        let previous_owned = previous_body
            .as_deref()
            .is_some_and(|body| index_points_to(index, body));
        let legacy_owned = binding.allow_legacy_authority_indexes
            && index_points_to_legacy_authority(context, record, index);
        if !metadata.file_type().is_symlink() || (!previous_owned && !legacy_owned) {
            return Err(format!(
                "duplicate host index is not owned by this installed Skill: {}",
                index.display()
            ));
        }
    }
    let plan = PreparedSkillChange {
        operation: operation.to_string(),
        skill_id: record.skill_id.clone(),
        source: record.source.clone(),
        source_hash: record.source_hash.clone(),
        license_path: record.license_path.clone(),
        license_hash: record.license_hash.clone(),
        routing_metadata_path: record.routing_metadata_path.clone(),
        routing_metadata_hash: record.routing_metadata_hash.clone(),
        body_path: body_path(&context.runtime_home, record)
            .to_string_lossy()
            .into_owned(),
        installed_skill_index_path: installed_skill_index_path(&context.runtime_home)
            .to_string_lossy()
            .into_owned(),
        target_hosts,
        host_indexes: indexes
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        retired_host_indexes: retired_indexes
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        planned_writes: vec![
            format!(
                "install immutable body: {}",
                body_path(&context.runtime_home, record).display()
            ),
            "replace exact host thin indexes".to_string(),
            "retire duplicate indexes from alternate Host roots".to_string(),
            format!(
                "update installed Skill index: {}",
                installed_skill_index_path(&context.runtime_home).display()
            ),
            "refresh selected host capability snapshots".to_string(),
        ],
        warnings: record
            .risk_findings
            .iter()
            .map(|finding| format!("{}: {}", finding.code, finding.detail))
            .collect(),
        source_spec: record.source_spec.clone(),
        resolved_source: record.resolved_source.clone(),
        body_hash: record.source_hash.clone(),
        candidate_identity: binding.candidate_identity,
        update_policy: record.update_policy,
        catalog_review: record.catalog_review,
        risk_findings: record.risk_findings.clone(),
        candidate_path: binding.candidate_path,
        previous_body_revision: binding.previous_body_revision,
        rollback_revision: binding.rollback_revision,
        registry_revision: registry.revision,
        previous_record_hash: previous_record.as_ref().map(record_hash).transpose()?,
        previous_body_hash: previous_record
            .as_ref()
            .map(|existing| body_identity(&context.runtime_home, existing))
            .transpose()?
            .flatten(),
        previous_record,
    };
    Ok(plan)
}

fn validate_candidate_path(
    context: &AdoptionContext,
    candidate_path: &Path,
    candidate_identity: &str,
) -> Result<(), String> {
    let candidates_root = context
        .runtime_home
        .join("candidates")
        .canonicalize()
        .map_err(|error| {
            format!(
                "cannot resolve candidate store {}: {error}",
                candidates_root_display(context)
            )
        })?;
    let candidate = candidate_path.canonicalize().map_err(|error| {
        format!(
            "cannot resolve candidate path {}: {error}",
            candidate_path.display()
        )
    })?;
    if fs::symlink_metadata(candidate_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("symlink_refused: candidate path".to_string());
    }
    if !candidate.starts_with(&candidates_root) {
        return Err("candidate_path_escapes_runtime_home".to_string());
    }
    let expected = candidate_identity.trim_start_matches("sha256:");
    if !candidate
        .ancestors()
        .any(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some(expected))
    {
        return Err("candidate_identity_path_mismatch".to_string());
    }
    Ok(())
}

fn candidates_root_display(context: &AdoptionContext) -> String {
    context
        .runtime_home
        .join("candidates")
        .display()
        .to_string()
}

fn ensure_risks_acknowledged(
    plan: &PreparedSkillChange,
    acknowledgements: &RiskAcknowledgements,
) -> Result<(), String> {
    let known = plan
        .risk_findings
        .iter()
        .map(RiskFinding::acknowledgement_id)
        .collect::<RiskAcknowledgements>();
    let unknown = acknowledgements
        .difference(&known)
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(format!("acknowledgement_unknown: {}", unknown.join(",")));
    }
    let missing = plan
        .risk_findings
        .iter()
        .filter(|finding| finding.acknowledgement_required)
        .map(RiskFinding::acknowledgement_id)
        .filter(|identifier| !acknowledgements.contains(identifier))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!("acknowledgement_required: {}", missing.join(",")));
    }
    Ok(())
}

fn add_catalog_review_risk(risk_findings: &mut Vec<RiskFinding>) {
    if risk_findings
        .iter()
        .any(|finding| finding.code == "catalog_unreviewed")
    {
        return;
    }
    risk_findings.push(RiskFinding::acknowledgement(
        "catalog_unreviewed",
        None,
        "third-party source has not completed catalog review",
    ));
    risk_findings
        .sort_by(|left, right| left.code.cmp(&right.code).then(left.path.cmp(&right.path)));
}

fn bind_catalog_review(
    authority_root: &Path,
    record: &mut InstalledSkillRecord,
) -> Result<(), String> {
    let SourceSpec::GitHub { url, subdir, .. } = &record.source_spec else {
        return Ok(());
    };
    let observed_revision = record
        .resolved_source
        .as_ref()
        .map(|source| source.resolved_commit.as_str())
        .or_else(|| record.source_spec.requested_ref());
    let Some(observed_revision) = observed_revision else {
        return Ok(());
    };
    let manifest = crate::third_party_manifest::read_third_party_manifest(authority_root)?;
    let mut matches = manifest.capabilities.iter().filter(|capability| {
        capability.kind == crate::third_party_manifest::CapabilityKind::Skill
            && capability.source.repository.as_deref() == Some(url.as_str())
            && capability.source.revision.as_deref() == Some(observed_revision)
            && capability.source.subdir.as_deref() == subdir.as_deref()
    });
    let Some(catalog) = matches.next() else {
        return Ok(());
    };
    if matches.next().is_some() {
        return Err("catalog source identity is duplicated".to_string());
    }
    if catalog.id != record.skill_id {
        return Err(format!(
            "catalog_skill_id_mismatch: source is reviewed for `{}` but body declares `{}`",
            catalog.id, record.skill_id
        ));
    }
    let expected = catalog
        .source
        .integrity
        .as_deref()
        .ok_or_else(|| format!("catalog entry `{}` has no reviewed body hash", catalog.id))?;
    if expected != record.source_hash {
        return Err(format!(
            "catalog_integrity_mismatch: `{}` expected {expected}, observed {}",
            catalog.id, record.source_hash
        ));
    }
    record.catalog_review = CatalogReviewStatus::Reviewed;
    record
        .risk_findings
        .retain(|finding| finding.code != "catalog_unreviewed");
    Ok(())
}

fn normalize_risk_findings(mut risk_findings: Vec<RiskFinding>) -> Vec<RiskFinding> {
    risk_findings
        .sort_by(|left, right| left.code.cmp(&right.code).then(left.path.cmp(&right.path)));
    risk_findings.dedup();
    risk_findings
}

fn merge_revision_history(
    mut history: Vec<BodyRevision>,
    record: &InstalledSkillRecord,
) -> Vec<BodyRevision> {
    if !history
        .iter()
        .any(|revision| revision.revision == record.body_revision)
    {
        history.push(BodyRevision::from_record(record));
    } else if let Some(existing) = history
        .iter_mut()
        .find(|revision| revision.revision == record.body_revision)
    {
        *existing = BodyRevision::from_record(record);
    }
    history.sort_by(|left, right| left.revision.cmp(&right.revision));
    history.dedup_by(|left, right| left.revision == right.revision);
    history
}

fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn normalize_hosts(requested: &[String]) -> Result<Vec<String>, String> {
    let supported = ags_host_integration::supported_skill_hosts()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    let mut hosts = if requested.iter().any(|host| host == "all") {
        supported.iter().cloned().collect::<Vec<_>>()
    } else {
        requested.to_vec()
    };
    hosts.sort();
    hosts.dedup();
    if let Some(unsupported) = hosts.iter().find(|host| !supported.contains(*host)) {
        return Err(format!("unsupported skill host: {unsupported}"));
    }
    Ok(hosts)
}

fn reject_official_collision(authority_root: &Path, skill_id: &str) -> Result<(), String> {
    let path = authority_root.join("manifests/skills-registry.yaml");
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "cannot read official skill registry {}: {error}",
            path.display()
        )
    })?;
    let document: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("cannot parse official skill registry: {error}"))?;
    let collision = document["skills"]
        .as_sequence()
        .into_iter()
        .flatten()
        .any(|skill| skill["name"].as_str() == Some(skill_id));
    if collision {
        Err(format!(
            "third-party adoption cannot shadow suite Skill id: {skill_id}"
        ))
    } else {
        Ok(())
    }
}

fn install_body(source: &Path, body: &Path, expected_hash: &str) -> Result<bool, String> {
    if body.exists() {
        let actual = hash_skill_source(body)?;
        if actual == expected_hash {
            return Ok(false);
        }
        return Err(format!(
            "immutable body path contains different content: {}",
            body.display()
        ));
    }
    let parent = body
        .parent()
        .ok_or_else(|| "immutable body path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create body parent {}: {error}", parent.display()))?;
    let stage = parent.join(format!(
        ".stage-{}-{}",
        std::process::id(),
        body.file_name().unwrap().to_string_lossy()
    ));
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(|error| {
            format!("cannot clear stale body stage {}: {error}", stage.display())
        })?;
    }
    ags_platform::copy_regular_tree(source, &stage)?;
    let staged_hash = hash_skill_source(&stage)?;
    if staged_hash != expected_hash {
        let _ = fs::remove_dir_all(&stage);
        return Err("source_drift_during_copy".to_string());
    }
    fs::rename(&stage, body)
        .map_err(|error| format!("cannot publish immutable body {}: {error}", body.display()))?;
    Ok(true)
}

fn refresh_snapshots(
    context: &AdoptionContext,
    hosts: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let snapshots = match context.snapshot_discovery {
        SnapshotDiscovery::Live => build_capability_snapshots_with_live_roots(
            &context.authority_root,
            hosts,
            &context.runtime_home,
            &context.host_home,
        ),
        SnapshotDiscovery::Offline => hosts
            .iter()
            .map(|host| {
                build_capability_snapshot_with_roots(
                    &context.authority_root,
                    host,
                    &context.runtime_home,
                    &context.host_home,
                )
                .map(|snapshot| (host.clone(), snapshot))
            })
            .collect(),
    }
    .map_err(|error| format!("capability snapshot build failed: {error:?}"))?;
    publish_capability_snapshots(&context.runtime_home, snapshots)
}

fn capture_links(paths: &[String]) -> Result<Vec<(PathBuf, Option<PathBuf>)>, String> {
    paths
        .iter()
        .map(|path| {
            let path = PathBuf::from(path);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => fs::read_link(&path)
                    .map(Some)
                    .map(|target| (path.clone(), target))
                    .map_err(|error| format!("cannot read link {}: {error}", path.display())),
                Ok(_) => Err(format!(
                    "host index conflict is not a symlink: {}",
                    path.display()
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
                Err(error) => Err(format!(
                    "cannot inspect host index {}: {error}",
                    path.display()
                )),
            }
        })
        .collect()
}

fn capture_snapshots(runtime_home: &Path, hosts: &[String]) -> Result<Vec<FileBackup>, String> {
    hosts
        .iter()
        .map(|host| {
            let path = snapshot_path(runtime_home, host);
            read_optional(&path).map(|bytes| (path, bytes))
        })
        .collect()
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn replace_symlink(index: &Path, target: &Path) -> Result<(), String> {
    let parent = index
        .parent()
        .ok_or_else(|| "host index has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "cannot create host skill root {}: {error}",
            parent.display()
        )
    })?;
    let stage = parent.join(format!(
        ".ags-adopt-{}-{}.tmp",
        std::process::id(),
        index.file_name().unwrap().to_string_lossy()
    ));
    if fs::symlink_metadata(&stage).is_ok() {
        fs::remove_file(&stage)
            .map_err(|error| format!("cannot clear staged link {}: {error}", stage.display()))?;
    }
    create_dir_symlink(target, &stage)?;
    if fs::symlink_metadata(index).is_ok() {
        fs::remove_file(index)
            .map_err(|error| format!("cannot replace host index {}: {error}", index.display()))?;
    }
    fs::rename(&stage, index)
        .map_err(|error| format!("cannot publish host index {}: {error}", index.display()))
}

fn remove_host_index(index: &Path) -> Result<(), String> {
    match fs::symlink_metadata(index) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(index)
                .map_err(|error| format!("cannot unlink {}: {error}", index.display()))
        }
        Ok(_) => Err(format!(
            "host index conflict is not removable link: {}",
            index.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot inspect host index {}: {error}",
            index.display()
        )),
    }
}

fn create_dir_symlink(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .map_err(|error| format!("cannot create link {}: {error}", link.display()))
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
            .map_err(|error| format!("cannot create link {}: {error}", link.display()))
    }
}

fn restore_optional_file(path: &Path, bytes: Option<&[u8]>) -> Result<(), String> {
    if let Some(bytes) = bytes {
        ags_platform::atomic_write(path, bytes)
    } else if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("cannot remove {} during rollback: {error}", path.display()))
    } else {
        Ok(())
    }
}
