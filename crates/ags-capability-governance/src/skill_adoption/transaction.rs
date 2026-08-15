use super::model::{
    AdoptionContext, BodyRevision, CatalogReviewStatus, InstalledSkillIndex, InstalledSkillRecord,
    PreparedSkillChange, PreparedSkillChangeContract, ReadInputSeal, ResolvedSource,
    RiskAcknowledgements, RiskFinding, SourceSpec, UpdatePolicy,
};
use super::projection::{host_index_path, host_index_paths, index_points_to};
use super::remote::{acquire_remote_candidate_with_backend, GitBackend};
use super::source::{audit_local_source, audit_local_source_with_boundary};
use super::store::{
    body_path, installed_skill_index_path, observe_installed_skills, ObservedInstalledSkillIndex,
};
use crate::hash_skill_source;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn ensure_plan_cas_against(
    context: &AdoptionContext,
    plan: &PreparedSkillChange,
    registry: InstalledSkillIndex,
) -> Result<InstalledSkillIndex, String> {
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

pub fn plan_removal(
    context: &AdoptionContext,
    skill_id: &str,
) -> Result<PreparedSkillChange, String> {
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    let registry = &observed_registry.value;
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
        contract_schema: PreparedSkillChangeContract::V2,
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
        registry_read_input: Some(observed_registry.seal.clone()),
        registry_semantic_hash: observed_registry.semantic_hash.clone(),
        previous_record: Some(record.clone()),
        previous_record_hash: Some(record_hash(record)?),
        previous_body_hash: body_identity(&context.runtime_home, record)?,
        target_record: None,
        candidate_read_inputs: Vec::new(),
        expected_link_targets: capture_expected_link_targets(&indexes)?,
    };
    Ok(plan)
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
    let mut target_hosts = normalize_hosts(context, requested_hosts)?;
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    let existing_registry = &observed_registry.value;
    let (mut record, candidate_path, candidate_identity, candidate_read_inputs) = match source {
        SourceSpec::Local { path } => {
            let audited =
                audit_local_source(Path::new(path), target_hosts.clone(), routing_metadata)?;
            let mut record = audited.record;
            record.source_spec = SourceSpec::Local {
                path: record.source.clone(),
            };
            record.risk_findings = audited.risk_findings;
            let candidate_path = record.source.clone();
            (record, candidate_path, String::new(), audited.read_inputs)
        }
        SourceSpec::GitHub { .. } | SourceSpec::Git { .. } => {
            let candidate = acquire_remote_candidate_with_backend(context, source, backend)?;
            let candidate_identity = candidate.resolved_source.candidate_identity.clone();
            let mut audited = audit_local_source_with_boundary(
                &candidate.skill_dir,
                target_hosts.clone(),
                routing_metadata,
                Some(&candidate.checkout_root),
            )?;
            for risk in candidate.record.risk_findings.iter().cloned() {
                if !audited.record.risk_findings.contains(&risk) {
                    audited.record.risk_findings.push(risk);
                }
            }
            audited.risk_findings = audited.record.risk_findings.clone();
            audited.record.source = candidate.record.source;
            audited.record.source_spec = candidate.record.source_spec;
            audited.record.resolved_source = Some(candidate.resolved_source.clone());
            (
                audited.record,
                candidate.skill_dir.to_string_lossy().into_owned(),
                candidate_identity,
                audited.read_inputs,
            )
        }
    };
    bind_catalog_review(&context.authority_root, &mut record)?;
    if record.catalog_review == CatalogReviewStatus::Unreviewed {
        add_catalog_review_risk(&mut record.risk_findings);
    }
    reject_official_collision(&context.authority_root, &record)?;
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
    let allow_legacy_authority_indexes = matches!(source, SourceSpec::Local { .. })
        && record.catalog_review == CatalogReviewStatus::Reviewed;
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
            allow_legacy_authority_indexes,
            candidate_read_inputs: Some(candidate_read_inputs),
        },
        &observed_registry,
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
    let target_hosts = normalize_hosts(context, requested_hosts)?;
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    let registry = &observed_registry.value;
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
                    candidate_read_inputs: Some(audited.read_inputs),
                },
                &observed_registry,
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
            candidate_read_inputs: Some(audited.read_inputs),
        },
        &observed_registry,
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
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    let registry = &observed_registry.value;
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
    let mut audited = audit_local_source_with_boundary(
        &candidate.skill_dir,
        existing.target_hosts.clone(),
        existing.routing_metadata_path.as_deref().map(Path::new),
        Some(&candidate.checkout_root),
    )?;
    audited
        .record
        .risk_findings
        .extend(candidate.record.risk_findings.iter().cloned());
    let mut unique_risks = Vec::new();
    for risk in audited.record.risk_findings.drain(..) {
        if !unique_risks.contains(&risk) {
            unique_risks.push(risk);
        }
    }
    audited.record.risk_findings = unique_risks;
    audited.risk_findings = audited.record.risk_findings.clone();
    let mut record = audited.record;
    record.source = candidate.record.source;
    record.source_spec = candidate.record.source_spec;
    record.resolved_source = Some(candidate.resolved_source.clone());
    record.target_hosts = existing.target_hosts.clone();
    record.update_policy = existing.update_policy;
    record.catalog_review = CatalogReviewStatus::Unreviewed;
    bind_catalog_review(&context.authority_root, &mut record)?;
    if record.catalog_review == CatalogReviewStatus::Unreviewed {
        add_catalog_review_risk(&mut record.risk_findings);
    }
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
            candidate_read_inputs: Some(audited.read_inputs),
        },
        &observed_registry,
    )?;
    Ok(plan)
}

pub fn plan_rollback(
    context: &AdoptionContext,
    skill_id: &str,
    revision: &str,
) -> Result<PreparedSkillChange, String> {
    let observed_registry = observe_installed_skills(&context.runtime_home)?;
    let registry = &observed_registry.value;
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
            candidate_read_inputs: None,
        },
        &observed_registry,
    )?;
    Ok(plan)
}

struct PlanBinding {
    candidate_identity: String,
    candidate_path: Option<String>,
    previous_body_revision: Option<String>,
    rollback_revision: Option<String>,
    allow_legacy_authority_indexes: bool,
    candidate_read_inputs: Option<Vec<ReadInputSeal>>,
}

const LEGACY_SUPERPOWERS_ENTRYPOINTS: &[&str] = &[
    "brainstorming",
    "dispatching-parallel-agents",
    "executing-plans",
    "finishing-a-development-branch",
    "receiving-code-review",
    "requesting-code-review",
    "subagent-driven-development",
    "systematic-debugging",
    "test-driven-development",
    "using-git-worktrees",
    "using-superpowers",
    "verification-before-completion",
    "writing-plans",
    "writing-skills",
];

fn build_plan(
    context: &AdoptionContext,
    operation: &str,
    record: &InstalledSkillRecord,
    target_hosts: Vec<String>,
    binding: PlanBinding,
    observed_registry: &ObservedInstalledSkillIndex,
) -> Result<PreparedSkillChange, String> {
    let candidate_read_inputs = match binding.candidate_read_inputs {
        Some(seals) => seals,
        None => binding
            .candidate_path
            .as_deref()
            .map(Path::new)
            .filter(|path| path.is_dir())
            .map(super::materialize::seal_candidate_tree)
            .transpose()?
            .unwrap_or_default(),
    };
    let mut indexes = target_hosts
        .iter()
        .filter_map(|host| host_index_path(&context.host_home, host, &record.skill_id))
        .collect::<Vec<_>>();
    indexes.sort();
    indexes.dedup();
    let registry = &observed_registry.value;
    let previous_record = registry.skills.get(&record.skill_id).cloned();
    let mut target_record = record.clone();
    // Preserve the canonical record emitted by the legacy apply-time audit.
    // Planning timestamps are not part of the installed post-state today.
    target_record.installed_at = 0;
    target_record.body_revisions = merge_revision_history(
        previous_record
            .as_ref()
            .map(|existing| existing.body_revisions.clone())
            .unwrap_or_default(),
        &target_record,
    );
    let previous_body = previous_record
        .as_ref()
        .map(|record| body_path(&context.runtime_home, record));
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
    if binding.allow_legacy_authority_indexes && record.skill_id == "superpowers" {
        retired_indexes.extend(target_hosts.iter().flat_map(|host| {
            LEGACY_SUPERPOWERS_ENTRYPOINTS
                .iter()
                .flat_map(|entrypoint| host_index_paths(&context.host_home, host, entrypoint))
                .filter(|path| fs::symlink_metadata(path).is_ok())
        }));
    }
    retired_indexes.sort();
    retired_indexes.dedup();
    let mut all_indexes = indexes.clone();
    all_indexes.extend(retired_indexes.iter().cloned());
    let expected_link_targets = capture_expected_link_targets(&all_indexes)?;
    for index in &all_indexes {
        let key = index.to_string_lossy();
        let Some(target) = expected_link_targets
            .get(key.as_ref())
            .and_then(|target| target.as_deref())
        else {
            continue;
        };
        let previous_owned = previous_body
            .as_deref()
            .is_some_and(|body| observed_link_points_to(index, target, body));
        let legacy_root = context
            .authority_root
            .join("global-skills")
            .join(&record.skill_id);
        let legacy_owned = binding.allow_legacy_authority_indexes
            && (observed_link_points_to(index, target, &legacy_root)
                || (record.skill_id == "superpowers"
                    && LEGACY_SUPERPOWERS_ENTRYPOINTS.iter().any(|entrypoint| {
                        observed_link_points_to(
                            index,
                            target,
                            &context
                                .authority_root
                                .join("global-skills/superpowers/playbooks")
                                .join(entrypoint),
                        )
                    })));
        if !previous_owned && !legacy_owned {
            return Err(format!(
                "host index conflict is not owned by this installed Skill: {}",
                index.display()
            ));
        }
    }
    let plan = PreparedSkillChange {
        contract_schema: PreparedSkillChangeContract::V2,
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
        registry_read_input: Some(observed_registry.seal.clone()),
        registry_semantic_hash: observed_registry.semantic_hash.clone(),
        previous_record_hash: previous_record.as_ref().map(record_hash).transpose()?,
        previous_body_hash: previous_record
            .as_ref()
            .map(|existing| body_identity(&context.runtime_home, existing))
            .transpose()?
            .flatten(),
        target_record: Some(target_record),
        previous_record,
        candidate_read_inputs,
        expected_link_targets,
    };
    Ok(plan)
}

fn capture_expected_link_targets(
    indexes: &[PathBuf],
) -> Result<BTreeMap<String, Option<Vec<u8>>>, String> {
    indexes
        .iter()
        .map(|path| {
            let target = observe_link_target(path)?;
            Ok((path.to_string_lossy().into_owned(), target))
        })
        .collect()
}

#[cfg(unix)]
fn observed_link_points_to(index: &Path, target: &[u8], expected: &Path) -> bool {
    use std::os::unix::ffi::OsStringExt;

    let target = PathBuf::from(std::ffi::OsString::from_vec(target.to_vec()));
    let target = if target.is_absolute() {
        target
    } else {
        index
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };
    match (target.canonicalize(), expected.canonicalize()) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    }
}

#[cfg(not(unix))]
fn observed_link_points_to(_index: &Path, _target: &[u8], _expected: &Path) -> bool {
    false
}

#[cfg(unix)]
fn observe_link_target(path: &Path) -> Result<Option<Vec<u8>>, String> {
    crate::shared_skill_source::observe_link_target(path)
}

#[cfg(not(unix))]
fn observe_link_target(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let _ = path;
    Err("descriptor_semantics_unavailable_for_skill_materialization".to_string())
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn validate_candidate_path(
    context: &AdoptionContext,
    candidate_path: &Path,
    candidate_identity: &str,
) -> Result<(), String> {
    let candidates_root = context
        .candidate_home
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

#[cfg_attr(not(unix), allow(dead_code))]
fn candidates_root_display(context: &AdoptionContext) -> String {
    context
        .candidate_home
        .join("candidates")
        .display()
        .to_string()
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn ensure_risks_acknowledged(
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

pub(super) fn add_catalog_review_risk(risk_findings: &mut Vec<RiskFinding>) {
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
    if let SourceSpec::Local { path } = &record.source_spec {
        let resolution = crate::third_party_manifest::resolve_third_party_manifest(authority_root)?;
        let source = std::fs::canonicalize(path)
            .map_err(|error| format!("cannot resolve bundled Skill source {path}: {error}"))?;
        let authority = std::fs::canonicalize(authority_root).map_err(|error| {
            format!(
                "cannot resolve capability authority {}: {error}",
                authority_root.display()
            )
        })?;
        let mut matches = resolution
            .manifest
            .capabilities
            .iter()
            .filter(|capability| {
                let Some(bundled_path) = capability.source.bundled_path.as_deref() else {
                    return false;
                };
                capability.kind == crate::third_party_manifest::CapabilityKind::Skill
                    && capability.source.manager == "bundled"
                    && authority.join(bundled_path) == source
            });
        let Some(catalog) = matches.next() else {
            return Ok(());
        };
        if matches.next().is_some() {
            return Err("catalog bundled source identity is duplicated".to_string());
        }
        let expected_skill_id = catalog
            .compatibility_parent
            .as_deref()
            .unwrap_or(&catalog.id);
        if expected_skill_id != record.skill_id {
            return Err(format!(
                "catalog_skill_id_mismatch: `{}` distributes compatibility parent `{expected_skill_id}` but body declares `{}`",
                catalog.id, record.skill_id
            ));
        }
        bind_catalog_integrity(catalog, record)?;
        return Ok(());
    }
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
    let manifest =
        crate::third_party_manifest::resolve_third_party_manifest(authority_root)?.manifest;
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
    bind_catalog_integrity(catalog, record)?;
    Ok(())
}

fn bind_catalog_integrity(
    catalog: &crate::third_party_manifest::ThirdPartyCapability,
    record: &mut InstalledSkillRecord,
) -> Result<(), String> {
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

pub(super) fn merge_revision_history(
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

fn normalize_hosts(context: &AdoptionContext, requested: &[String]) -> Result<Vec<String>, String> {
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
    for host in &hosts {
        if supported.contains(host) {
            continue;
        }
        let registration = crate::load_canonical_host_registration(&context.runtime_home, host)
            .map_err(|_| format!("unsupported skill host: {host}"))?;
        if registration.host_id.as_str() != host {
            return Err(format!("unsupported skill host: {host}"));
        }
    }
    Ok(hosts)
}

fn reject_official_collision(
    authority_root: &Path,
    record: &InstalledSkillRecord,
) -> Result<(), String> {
    let skill_id = &record.skill_id;
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
        .any(|skill| skill["name"].as_str() == Some(skill_id.as_str()));
    if collision {
        return Err(format!(
            "third-party adoption cannot shadow suite Skill id: {skill_id}"
        ));
    }
    let manifest = crate::third_party_manifest::resolve_third_party_manifest(authority_root)?;
    let reserved =
        manifest.manifest.capabilities.iter().any(|capability| {
            capability.compatibility_parent.as_deref() == Some(skill_id.as_str())
        });
    if reserved && record.catalog_review != CatalogReviewStatus::Reviewed {
        return Err(format!(
            "third-party adoption cannot shadow catalog compatibility parent: {skill_id}; install its reviewed distribution id instead"
        ));
    }
    Ok(())
}
