use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const RUNTIME_SETUP_RECOVERY_SCHEMA: &str = "0.4.13-runtime-setup-recovery";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
enum SavedNode {
    Absent,
    File(Vec<u8>),
    Symlink(PathBuf),
    Directory(BTreeMap<String, SavedNode>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RecoveryPhase {
    Applying,
    Applied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeSetupRecovery {
    schema_version: String,
    transaction_id: String,
    phase: RecoveryPhase,
    entries: BTreeMap<PathBuf, SavedNode>,
    #[serde(default)]
    after_state_hashes: BTreeMap<PathBuf, String>,
}

pub struct RuntimeSetupMaintenanceBackend {
    pub runtime_home: PathBuf,
    pub prepared_change: Option<PreparedRuntimeSetup>,
}

pub fn path_state_hash(path: &Path) -> Result<String, String> {
    let state = capture_node(path)?;
    serde_json::to_vec(&state)
        .map(ags_platform::sha256)
        .map_err(|error| format!("cannot hash path state {}: {error}", path.display()))
}

fn capture_node(path: &Path) -> Result<SavedNode, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(SavedNode::Absent),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink() {
        return fs::read_link(path)
            .map(SavedNode::Symlink)
            .map_err(|error| format!("cannot read symlink {}: {error}", path.display()));
    }
    if metadata.is_file() {
        return fs::read(path)
            .map(SavedNode::File)
            .map_err(|error| format!("cannot read {}: {error}", path.display()));
    }
    if metadata.is_dir() {
        let mut children = BTreeMap::new();
        for entry in fs::read_dir(path)
            .map_err(|error| format!("cannot read directory {}: {error}", path.display()))?
        {
            let entry = entry.map_err(|error| format!("cannot read directory entry: {error}"))?;
            let name = entry.file_name().into_string().map_err(|_| {
                format!(
                    "runtime setup cannot recover non-UTF-8 entry in {}",
                    path.display()
                )
            })?;
            children.insert(name, capture_node(&entry.path())?);
        }
        return Ok(SavedNode::Directory(children));
    }
    Err(format!(
        "runtime setup rejects special file {}",
        path.display()
    ))
}

fn remove_exact(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| format!("cannot remove {}: {error}", path.display()))
}

fn restore_node(path: &Path, node: &SavedNode) -> Result<(), String> {
    remove_exact(path)?;
    match node {
        SavedNode::Absent => Ok(()),
        SavedNode::File(bytes) => ags_platform::atomic_write(path, bytes),
        SavedNode::Symlink(target) => {
            let parent = path
                .parent()
                .ok_or_else(|| format!("symlink path has no parent: {}", path.display()))?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, path)
                    .map_err(|error| format!("cannot restore symlink {}: {error}", path.display()))
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                Err("runtime setup symlink recovery is unsupported on this platform".to_string())
            }
        }
        SavedNode::Directory(children) => {
            fs::create_dir_all(path)
                .map_err(|error| format!("cannot restore directory {}: {error}", path.display()))?;
            for (name, child) in children {
                restore_node(&path.join(name), child)?;
            }
            Ok(())
        }
    }
}

fn recovery_path(runtime_home: &Path, transaction_id: &str) -> PathBuf {
    ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("recovery")
        .join(format!(
            "runtime-setup-{}.json",
            ags_platform::sha256_hex(transaction_id.as_bytes())
        ))
}

fn persist_recovery(runtime_home: &Path, recovery: &RuntimeSetupRecovery) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(recovery)
        .map_err(|error| format!("cannot serialize runtime setup recovery: {error}"))?;
    bytes.push(b'\n');
    ags_platform::atomic_write(
        &recovery_path(runtime_home, &recovery.transaction_id),
        &bytes,
    )
}

fn restore_recovery(runtime_home: &Path, recovery: &RuntimeSetupRecovery) -> Result<(), String> {
    for (path, node) in recovery.entries.iter().rev() {
        restore_node(path, node)?;
    }
    let path = recovery_path(runtime_home, &recovery.transaction_id);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
    }
}

pub fn recover_incomplete_runtime_setups(runtime_home: &Path) -> Result<(), String> {
    let _lock = ags_platform::MaintenanceLock::acquire(runtime_home)?;
    recover_incomplete_runtime_setups_locked(runtime_home)
}

/// Resume the exact persisted setup transaction for an explicit rollback.
/// The plan supplies its original binding and authority; callers cannot
/// substitute a different runtime, source root, or prepared payload.
pub fn recover_runtime_setup_plan(
    runtime_home: &Path,
    plan_hash: &str,
) -> Result<MaintenanceReceipt, String> {
    let plan = super::store::load_plan(runtime_home, plan_hash)?;
    plan.verify_hash()?;
    if plan.plan_hash != plan_hash {
        return Err("runtime setup recovery plan identity mismatch".to_string());
    }
    let change = match plan.payload.as_ref() {
        Some(MaintenancePayload::RuntimeSetup(change)) => change,
        _ => return Err("maintenance plan is not a runtime setup transaction".to_string()),
    };
    if ags_platform::normalize_path(&change.runtime_home)
        != ags_platform::normalize_path(runtime_home)
    {
        return Err("runtime setup recovery target differs from the sealed plan".to_string());
    }
    let service = MaintenanceService::new(
        ServiceContext {
            runtime_home: runtime_home.to_path_buf(),
            binding_id: plan.binding_id.clone(),
            clock: ServiceClock::System,
            plan_ttl_seconds: 60 * 60,
        },
        RuntimeSetupMaintenanceBackend {
            runtime_home: runtime_home.to_path_buf(),
            prepared_change: None,
        },
    )?;
    service.recover(plan_hash)
}

fn recover_incomplete_runtime_setups_locked(runtime_home: &Path) -> Result<(), String> {
    let directory = ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("recovery");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", directory.display())),
    };
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("runtime-setup-") || !name.ends_with(".json") {
            continue;
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let recovery: RuntimeSetupRecovery = serde_json::from_slice(&bytes)
            .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
        if recovery.schema_version != RUNTIME_SETUP_RECOVERY_SCHEMA {
            return Err(format!(
                "unsupported runtime setup recovery schema in {}",
                path.display()
            ));
        }
        if recovery.phase == RecoveryPhase::Applying {
            let plan = super::store::load_plan(runtime_home, &recovery.transaction_id)?;
            let change = match plan.payload.as_ref() {
                Some(MaintenancePayload::RuntimeSetup(change)) => change,
                _ => return Err("runtime setup recovery plan payload mismatch".to_string()),
            };
            rollback_legacy_migrations(
                &legacy_adoption_context(change),
                change.legacy_skill_migrations.iter().rev(),
                &plan.plan_hash,
            )?;
            SuiteSkillMaintenanceBackend {
                source_root: change.source_root.clone(),
                runtime_home: change.runtime_home.clone(),
                host_home: change.host_home.clone(),
                policy: crate::suite_skill_projection::SuiteSkillProjectionPolicy {
                    required_authority_root: Some(change.suite_skills.authority_root.clone()),
                    target_hosts: change.suite_skills.hosts.clone(),
                },
                prepared_change: None,
            }
            .recover_incomplete_change(&plan, &change.suite_skills)?;
            restore_recovery(runtime_home, &recovery)?;
        }
    }
    Ok(())
}

fn prune_superseded_recovery(runtime_home: &Path, current: &str) -> Result<(), String> {
    let directory = ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("recovery");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", directory.display())),
    };
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("runtime-setup-") || !name.ends_with(".json") {
            continue;
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let recovery: RuntimeSetupRecovery = serde_json::from_slice(&bytes)
            .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
        if recovery.transaction_id == current || recovery.phase != RecoveryPhase::Applied {
            continue;
        }
        let identity = ags_platform::sha256(recovery.transaction_id.as_bytes());
        for obsolete in [
            path,
            directory.join(format!("suite-snapshots-{identity}.json")),
            directory.join(format!("suite-skills-{identity}.json")),
        ] {
            match fs::remove_file(&obsolete) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot remove {}: {error}", obsolete.display())),
            }
        }
    }
    Ok(())
}

impl RuntimeSetupMaintenanceBackend {
    fn change<'a>(&self, plan: &'a MaintenancePlan) -> Result<&'a PreparedRuntimeSetup, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::RuntimeSetup(change)) => Ok(change),
            _ => Err("maintenance plan has no typed runtime setup change".to_string()),
        }
    }

    fn suite_backend(&self, change: &PreparedRuntimeSetup) -> SuiteSkillMaintenanceBackend {
        SuiteSkillMaintenanceBackend {
            source_root: change.source_root.clone(),
            runtime_home: change.runtime_home.clone(),
            host_home: change.host_home.clone(),
            policy: crate::suite_skill_projection::SuiteSkillProjectionPolicy {
                required_authority_root: Some(change.suite_skills.authority_root.clone()),
                target_hosts: change.suite_skills.hosts.clone(),
            },
            prepared_change: None,
        }
    }
}

impl MaintenanceBackend for RuntimeSetupMaintenanceBackend {
    fn prepare(&self, intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String> {
        if intent.subject != MaintenanceSubject::Runtime
            || !matches!(
                intent.operation,
                MaintenanceOperation::Install | MaintenanceOperation::Repair
            )
            || intent.target != "setup"
        {
            return Err(
                "runtime setup backend accepts runtime install/repair for target=setup".to_string(),
            );
        }
        recover_incomplete_runtime_setups(&self.runtime_home)?;
        let change = self
            .prepared_change
            .clone()
            .ok_or_else(|| "runtime setup requires a prepared change".to_string())?;
        if ags_platform::normalize_path(&change.runtime_home)
            != ags_platform::normalize_path(&self.runtime_home)
        {
            return Err("runtime setup target differs from maintenance runtime".to_string());
        }
        let mut risks = change
            .suite_skills
            .blocking_findings
            .iter()
            .enumerate()
            .map(|(index, finding)| RiskFinding {
                id: format!("runtime-setup-blocking-{index}"),
                class: RiskClass::Blocking,
                summary: finding.clone(),
                evidence_hash: Some(ags_platform::sha256(finding.as_bytes())),
            })
            .collect::<Vec<_>>();
        for migration in &change.legacy_skill_migrations {
            if migration.operation == "reactivate" {
                continue;
            }
            for finding in &migration.risk_findings {
                risks.push(RiskFinding {
                    id: legacy_migration_risk_id(&migration.skill_id, finding),
                    class: if finding.acknowledgement_required {
                        RiskClass::AcknowledgementRequired
                    } else {
                        RiskClass::Advisory
                    },
                    summary: format!(
                        "migrate retired suite Skill `{}`: {}",
                        migration.skill_id, finding.detail
                    ),
                    evidence_hash: Some(ags_platform::sha256(
                        format!("{}:{:?}", finding.code, finding.path).as_bytes(),
                    )),
                });
            }
        }
        for file in &change.files {
            let current = path_state_hash(&file.path)?;
            let expected = change
                .file_before_state_hashes
                .get(&file.path)
                .ok_or_else(|| format!("missing planned state for {}", file.path.display()))?;
            if &current != expected {
                return Err(format!(
                    "runtime setup input drift at {}",
                    file.path.display()
                ));
            }
            let can_write_without_force = match capture_node(&file.path)? {
                SavedNode::Absent => true,
                SavedNode::File(bytes) => bytes == file.content.as_bytes(),
                _ => false,
            };
            if !change.force && !can_write_without_force {
                risks.push(RiskFinding {
                    id: format!(
                        "runtime-file-replace-{}",
                        ags_platform::sha256_hex(file.path.to_string_lossy().as_bytes())
                    ),
                    class: RiskClass::Blocking,
                    summary: format!(
                        "{} exists with different content; rerun with explicit force",
                        file.path.display()
                    ),
                    evidence_hash: Some(current),
                });
            }
        }
        for path in &change.cleanup_paths {
            let current = path_state_hash(path)?;
            if change.cleanup_before_state_hashes.get(path) != Some(&current) {
                return Err(format!("runtime cleanup input drift at {}", path.display()));
            }
        }
        let mut planned_writes = change
            .files
            .iter()
            .map(|file| PlannedWrite {
                operation: "write".to_string(),
                path: file.path.to_string_lossy().into_owned(),
                before_hash: change.file_before_state_hashes.get(&file.path).cloned(),
                after_hash: Some(ags_platform::sha256(file.content.as_bytes())),
            })
            .collect::<Vec<_>>();
        planned_writes.extend(change.cleanup_paths.iter().map(|path| PlannedWrite {
            operation: "remove-retired".to_string(),
            path: path.to_string_lossy().into_owned(),
            before_hash: change.cleanup_before_state_hashes.get(path).cloned(),
            after_hash: Some(ags_platform::sha256(b"absent")),
        }));
        planned_writes.extend(change.suite_skills.operations.iter().map(|operation| {
            PlannedWrite {
                operation: format!("{:?}", operation.kind).to_ascii_lowercase(),
                path: operation.link_path.to_string_lossy().into_owned(),
                before_hash: None,
                after_hash: operation
                    .desired_target
                    .as_ref()
                    .map(|target| ags_platform::sha256(target.to_string_lossy().as_bytes())),
            }
        }));
        for migration in &change.legacy_skill_migrations {
            if migration.operation != "reactivate" {
                planned_writes.push(PlannedWrite {
                    operation: "install-immutable-skill-body".to_string(),
                    path: migration.body_path.clone(),
                    before_hash: migration.previous_body_hash.clone(),
                    after_hash: Some(migration.source_hash.clone()),
                });
            }
            planned_writes.push(PlannedWrite {
                operation: "update-installed-skill-index".to_string(),
                path: migration.installed_skill_index_path.clone(),
                before_hash: None,
                after_hash: Some(migration.source_hash.clone()),
            });
            planned_writes.extend(migration.host_indexes.iter().map(|path| PlannedWrite {
                operation: "activate-migrated-skill".to_string(),
                path: path.clone(),
                before_hash: None,
                after_hash: Some(migration.source_hash.clone()),
            }));
        }
        Ok(PreparedMaintenance {
            current_version: None,
            target_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            source: None,
            planned_writes,
            risks,
            verification_steps: vec![VerificationStep {
                id: "runtime-setup-closure".to_string(),
                description:
                    "verify runtime files, retired entries, every selected Host snapshot and exact route"
                        .to_string(),
            }],
            activation: change
                .suite_skills
                .hosts
                .iter()
                .map(|host| ActivationRequirement {
                    host: host.clone(),
                    requires_restart: true,
                    requires_repreflight: true,
                    expected_snapshot_hash: None,
                    exact_route_target: None,
                })
                .collect(),
            recovery_point: None,
            metadata: BTreeMap::from([
                (
                    "runtime_file_count".to_string(),
                    change.files.len().to_string(),
                ),
                (
                    "legacy_skill_migration_count".to_string(),
                    change.legacy_skill_migrations.len().to_string(),
                ),
            ]),
            payload: Some(MaintenancePayload::RuntimeSetup(change)),
        })
    }

    fn apply(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let _lock = ags_platform::MaintenanceLock::acquire(&self.runtime_home)?;
        recover_incomplete_runtime_setups_locked(&self.runtime_home)?;
        let change = self.change(plan)?;
        let mut paths = change
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        paths.extend(change.cleanup_paths.iter().cloned());
        let mut recovery = RuntimeSetupRecovery {
            schema_version: RUNTIME_SETUP_RECOVERY_SCHEMA.to_string(),
            transaction_id: plan.plan_hash.clone(),
            phase: RecoveryPhase::Applying,
            entries: BTreeMap::new(),
            after_state_hashes: BTreeMap::new(),
        };
        for path in paths {
            let current = path_state_hash(&path)?;
            let expected = change
                .file_before_state_hashes
                .get(&path)
                .or_else(|| change.cleanup_before_state_hashes.get(&path))
                .ok_or_else(|| format!("missing sealed path state for {}", path.display()))?;
            if &current != expected {
                return Err(format!("runtime setup stale plan at {}", path.display()));
            }
            recovery.entries.insert(path.clone(), capture_node(&path)?);
        }
        persist_recovery(&self.runtime_home, &recovery)?;
        let mutation = (|| -> Result<MaintenanceExecution, String> {
            for file in &change.files {
                ags_platform::atomic_write(&file.path, file.content.as_bytes())?;
                #[cfg(unix)]
                if let Some(mode) = file.mode {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&file.path, fs::Permissions::from_mode(mode)).map_err(
                        |error| format!("cannot chmod {}: {error}", file.path.display()),
                    )?;
                }
            }
            for path in &change.cleanup_paths {
                let finding = crate::setup::plan::cleanup_install_entry(path, change.force);
                if finding.status == crate::setup::SetupCheckStatus::Fail {
                    return Err(finding.detail.unwrap_or(finding.message));
                }
            }
            let mut execution = self
                .suite_backend(change)
                .apply_change_deferred(plan, &change.suite_skills)?;
            let context = legacy_adoption_context(change);
            let mut applied = Vec::new();
            for migration in &change.legacy_skill_migrations {
                let acknowledgements = migration
                    .risk_findings
                    .iter()
                    .filter(|finding| finding.acknowledgement_required)
                    .map(|finding| finding.acknowledgement_id())
                    .collect();
                let transaction_id = format!("{}:legacy:{}", plan.plan_hash, migration.skill_id);
                let applied_change = if migration.operation == "reactivate" {
                    ags_capability_governance::skill_adoption::apply_reactivation_in_maintenance_transaction(
                        &context,
                        migration,
                        &transaction_id,
                    )
                } else {
                    ags_capability_governance::skill_adoption::apply_install_in_maintenance_transaction(
                        &context,
                        migration,
                        &transaction_id,
                        &acknowledgements,
                    )
                };
                match applied_change {
                    Ok(_result) => {
                        applied.push(migration);
                    }
                    Err(error) => {
                        let rollback = rollback_legacy_migrations(
                            &context,
                            applied.into_iter().rev(),
                            &plan.plan_hash,
                        );
                        let suite = self
                            .suite_backend(change)
                            .recover_change(plan, &change.suite_skills);
                        return Err(format!(
                            "{error}; legacy migration recovery={}; suite recovery={}",
                            rollback
                                .map(|_| "ok".to_string())
                                .unwrap_or_else(|error| error),
                            suite
                                .map(|_| "ok".to_string())
                                .unwrap_or_else(|error| error),
                        ));
                    }
                }
            }
            let hashes = match self
                .suite_backend(change)
                .compile_and_publish_suite_snapshots(&change.suite_skills)
            {
                Ok(hashes) => hashes,
                Err(error) => {
                    let rollback = rollback_legacy_migrations(
                        &context,
                        applied.into_iter().rev(),
                        &plan.plan_hash,
                    );
                    let suite = self
                        .suite_backend(change)
                        .recover_change(plan, &change.suite_skills);
                    return Err(format!(
                        "{error}; legacy migration recovery={}; suite recovery={}",
                        rollback
                            .map(|_| "ok".to_string())
                            .unwrap_or_else(|error| error),
                        suite
                            .map(|_| "ok".to_string())
                            .unwrap_or_else(|error| error),
                    ));
                }
            };
            execution
                .activation_results
                .extend(hashes.into_iter().map(|(host, hash)| ActivationResult {
                    host,
                    activated: true,
                    repreflight_passed: false,
                    route_verified: false,
                    evidence: hash,
                }));
            Ok(execution)
        })();
        match mutation {
            Ok(execution) => {
                recovery.phase = RecoveryPhase::Applied;
                let after_state_hashes = recovery
                    .entries
                    .keys()
                    .map(|path| path_state_hash(path).map(|hash| (path.clone(), hash)))
                    .collect::<Result<BTreeMap<_, _>, _>>();
                match after_state_hashes.and_then(|hashes| {
                    recovery.after_state_hashes = hashes;
                    persist_recovery(&self.runtime_home, &recovery)
                }) {
                    Ok(()) => Ok(execution),
                    Err(error) => {
                        let context = legacy_adoption_context(change);
                        let legacy = rollback_legacy_migrations(
                            &context,
                            change.legacy_skill_migrations.iter().rev(),
                            &plan.plan_hash,
                        );
                        let suite = self
                            .suite_backend(change)
                            .recover_change(plan, &change.suite_skills);
                        let runtime = restore_recovery(&self.runtime_home, &recovery);
                        Err(format!(
                            "{error}; legacy recovery={}; suite recovery={}; runtime recovery={}",
                            legacy
                                .map(|_| "ok".to_string())
                                .unwrap_or_else(|error| error),
                            suite
                                .map(|_| "ok".to_string())
                                .unwrap_or_else(|error| error),
                            runtime
                                .map(|_| "ok".to_string())
                                .unwrap_or_else(|error| error),
                        ))
                    }
                }
            }
            Err(error) => {
                let restore = restore_recovery(&self.runtime_home, &recovery);
                Err(format!(
                    "{error}; runtime recovery={}",
                    restore
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|error| error)
                ))
            }
        }
    }

    fn verify(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.change(plan)?;
        for file in &change.files {
            if fs::read(&file.path).ok().as_deref() != Some(file.content.as_bytes()) {
                return Err(format!(
                    "runtime file verification failed: {}",
                    file.path.display()
                ));
            }
        }
        for path in &change.cleanup_paths {
            if fs::symlink_metadata(path).is_ok() {
                return Err(format!("retired runtime entry remains: {}", path.display()));
            }
        }
        let mut execution = self
            .suite_backend(change)
            .verify_change(plan, &change.suite_skills)?;
        if !change.legacy_skill_migrations.is_empty() {
            let ids = change
                .legacy_skill_migrations
                .iter()
                .map(|migration| migration.skill_id.clone())
                .collect::<Vec<_>>();
            let routes = ags_capability_governance::skill_adoption::verify_adoption_routes_batch(
                &change.runtime_home,
                &change.host_home,
                &ids,
            )?;
            for (skill_id, route) in routes {
                let passed = route.verified_on_all_targets();
                if !passed {
                    execution.status = MaintenanceStatus::Failed;
                    execution.error = Some(format!(
                        "migrated Skill `{skill_id}` did not verify on every target Host"
                    ));
                }
                execution.verification_results.push(VerificationResult {
                    id: format!("legacy-skill-route-{skill_id}"),
                    passed,
                    evidence: ags_platform::sha256(
                        &serde_json::to_vec(&route)
                            .map_err(|error| format!("cannot serialize route evidence: {error}"))?,
                    ),
                });
            }
        }
        if execution.status == MaintenanceStatus::Verified {
            let _lock = ags_platform::MaintenanceLock::acquire(&self.runtime_home)?;
            prune_superseded_recovery(&self.runtime_home, &plan.plan_hash)?;
        }
        Ok(execution)
    }

    fn recover(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let _lock = ags_platform::MaintenanceLock::acquire(&self.runtime_home)?;
        let change = self.change(plan)?;
        let path = recovery_path(&self.runtime_home, &plan.plan_hash);
        let bytes =
            fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let recovery: RuntimeSetupRecovery = serde_json::from_slice(&bytes)
            .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
        if recovery.schema_version != RUNTIME_SETUP_RECOVERY_SCHEMA
            || recovery.transaction_id != plan.plan_hash
            || recovery.phase != RecoveryPhase::Applied
        {
            return Err("runtime setup recovery identity mismatch".to_string());
        }
        for (path, expected) in &recovery.after_state_hashes {
            if path_state_hash(path)? != *expected {
                return Err(format!(
                    "runtime setup recovery refused because {} changed after apply",
                    path.display()
                ));
            }
        }
        let legacy = rollback_legacy_migrations(
            &legacy_adoption_context(change),
            change.legacy_skill_migrations.iter().rev(),
            &plan.plan_hash,
        );
        let suite = self
            .suite_backend(change)
            .recover_change(plan, &change.suite_skills);
        let runtime = restore_recovery(&self.runtime_home, &recovery);
        match (legacy, suite, runtime) {
            (Ok(()), Ok(mut execution), Ok(())) => {
                execution.verification_results.push(VerificationResult {
                    id: "runtime-files-recovery".to_string(),
                    passed: true,
                    evidence: plan.plan_hash.clone(),
                });
                Ok(execution)
            }
            (legacy, suite, runtime) => Err(format!(
                "legacy recovery={}; suite recovery={}; runtime recovery={}",
                legacy
                    .map(|_| "ok".to_string())
                    .unwrap_or_else(|error| error),
                suite
                    .map(|_| "ok".to_string())
                    .unwrap_or_else(|error| error),
                runtime
                    .map(|_| "ok".to_string())
                    .unwrap_or_else(|error| error),
            )),
        }
    }
}

fn legacy_adoption_context(
    change: &PreparedRuntimeSetup,
) -> ags_capability_governance::skill_adoption::AdoptionContext {
    ags_capability_governance::skill_adoption::AdoptionContext {
        authority_root: change.source_root.clone(),
        runtime_home: change.runtime_home.clone(),
        host_home: change.host_home.clone(),
        snapshot_discovery: ags_capability_governance::skill_adoption::SnapshotDiscovery::Live,
    }
}

fn legacy_migration_risk_id(
    skill_id: &str,
    finding: &ags_capability_governance::skill_adoption::RiskFinding,
) -> String {
    format!("legacy-skill-{}-{}", skill_id, finding.acknowledgement_id())
}

fn rollback_legacy_migrations<'a>(
    context: &ags_capability_governance::skill_adoption::AdoptionContext,
    migrations: impl IntoIterator<
        Item = &'a ags_capability_governance::skill_adoption::PreparedSkillChange,
    >,
    transaction_id: &str,
) -> Result<(), String> {
    for migration in migrations {
        let index = ags_capability_governance::skill_adoption::load_installed_skills(
            &context.runtime_home,
        )?;
        match index.skills.get(&migration.skill_id) {
            None => continue,
            Some(record)
                if record.source_hash == migration.source_hash
                    && record.body_revision
                        == migration.source_hash.trim_start_matches("sha256:") => {}
            Some(_) => {
                return Err(format!(
                    "recovery_refused: migrated Skill `{}` changed after apply",
                    migration.skill_id
                ))
            }
        }
        ags_capability_governance::skill_adoption::recover_applied_change_in_maintenance_transaction(
            context,
            migration,
            &format!("{transaction_id}:recover-legacy:{}", migration.skill_id),
        )?;
    }
    Ok(())
}
