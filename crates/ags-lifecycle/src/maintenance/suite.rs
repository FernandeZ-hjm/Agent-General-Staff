use super::*;
use crate::suite_skill_projection::{
    apply_required_suite_skill_projection, recover_required_suite_skill_projection,
    verify_required_suite_skill_projection_with_runtime, PreparedSuiteSkillProjection,
    SuiteSkillProjectionPolicy,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const SNAPSHOT_RECOVERY_SCHEMA: &str = "0.5.0-suite-snapshot-recovery";

/// Required suite Skill projection is a runtime maintenance subject. Setup and
/// update use this same backend instead of owning separate link/snapshot
/// transactions.
pub struct SuiteSkillMaintenanceBackend {
    pub source_root: PathBuf,
    pub runtime_home: PathBuf,
    pub host_home: PathBuf,
    pub preflight_target: PathBuf,
    pub policy: SuiteSkillProjectionPolicy,
    /// Setup may already have rendered this exact change for user review. In
    /// that case the same value is sealed into MaintenancePlan instead of
    /// rescanning and creating a second, potentially different preview.
    pub prepared_change: Option<PreparedSuiteSkillProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", content = "bytes", rename_all = "kebab-case")]
enum PreviousSnapshot {
    Absent,
    Present(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRecoveryRecord {
    schema_version: String,
    transaction_id: String,
    snapshots: BTreeMap<String, PreviousSnapshot>,
}

impl SuiteSkillMaintenanceBackend {
    fn affected_hosts(change: &PreparedSuiteSkillProjection) -> Vec<String> {
        let mut hosts = change.hosts.clone();
        hosts.extend(change.deactivated_hosts.iter().cloned());
        hosts.sort();
        hosts.dedup();
        hosts
    }

    fn change<'a>(
        &self,
        plan: &'a MaintenancePlan,
    ) -> Result<&'a PreparedSuiteSkillProjection, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::SuiteSkills(change)) => Ok(change),
            _ => Err("maintenance plan has no typed suite Skill change".to_string()),
        }
    }

    fn snapshot_recovery_path(&self, transaction_id: &str) -> PathBuf {
        let identity = ags_platform::sha256(transaction_id.as_bytes());
        ags_platform::RuntimeLayout::new(&self.runtime_home)
            .maintenance()
            .join("recovery")
            .join(format!("suite-snapshots-{identity}.json"))
    }

    fn capture_snapshots(
        &self,
        transaction_id: &str,
        hosts: &[String],
    ) -> Result<SnapshotRecoveryRecord, String> {
        let mut snapshots = BTreeMap::new();
        for host in hosts {
            let path = ags_capability_governance::snapshot_path(&self.runtime_home, host);
            let previous = match fs::read(&path) {
                Ok(bytes) => PreviousSnapshot::Present(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    PreviousSnapshot::Absent
                }
                Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
            };
            snapshots.insert(host.to_string(), previous);
        }
        let record = SnapshotRecoveryRecord {
            schema_version: SNAPSHOT_RECOVERY_SCHEMA.to_string(),
            transaction_id: transaction_id.to_string(),
            snapshots,
        };
        let mut bytes = serde_json::to_vec_pretty(&record)
            .map_err(|error| format!("cannot serialize snapshot recovery: {error}"))?;
        bytes.push(b'\n');
        ags_platform::atomic_write(&self.snapshot_recovery_path(transaction_id), &bytes)
            .map_err(|error| format!("cannot persist snapshot recovery: {error}"))?;
        Ok(record)
    }

    fn load_snapshot_recovery(
        &self,
        transaction_id: &str,
        hosts: &[String],
    ) -> Result<SnapshotRecoveryRecord, String> {
        let path = self.snapshot_recovery_path(transaction_id);
        let bytes = fs::read(&path).map_err(|error| {
            format!("cannot read snapshot recovery {}: {error}", path.display())
        })?;
        let record: SnapshotRecoveryRecord = serde_json::from_slice(&bytes).map_err(|error| {
            format!("cannot parse snapshot recovery {}: {error}", path.display())
        })?;
        let observed_hosts = record
            .snapshots
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let required_hosts = hosts
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if record.schema_version != SNAPSHOT_RECOVERY_SCHEMA
            || record.transaction_id != transaction_id
            || observed_hosts != required_hosts
        {
            return Err("suite snapshot recovery identity mismatch".to_string());
        }
        Ok(record)
    }

    fn restore_snapshots(&self, record: &SnapshotRecoveryRecord) -> Result<(), String> {
        let mut errors = Vec::new();
        for (host, previous) in &record.snapshots {
            let path = ags_capability_governance::snapshot_path(&self.runtime_home, host);
            let result = match previous {
                PreviousSnapshot::Present(bytes) => ags_platform::atomic_write(&path, bytes),
                PreviousSnapshot::Absent => match fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                },
            };
            if let Err(error) = result {
                errors.push(format!("cannot restore {}: {error}", path.display()));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn remove_snapshot_recovery(&self, transaction_id: &str) -> Result<(), String> {
        let path = self.snapshot_recovery_path(transaction_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot remove {}: {error}", path.display())),
        }
    }

    pub(super) fn compile_and_publish_suite_snapshots(
        &self,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<BTreeMap<String, String>, String> {
        let snapshots = ags_capability_governance::build_capability_snapshots_with_live_roots(
            &change.authority_root,
            &change.hosts,
            &self.runtime_home,
            &self.host_home,
        )
        .map_err(|error| format!("capability snapshot build failed: {error:?}"))?;
        for (host, snapshot) in &snapshots {
            let tables = snapshot
                .validate_integrity(host)
                .map_err(|error| format!("invalid `{host}` candidate snapshot: {error:?}"))?;
            verify_required_skills(snapshot, &tables, &change.required_skills, host)?;
        }
        let hashes =
            ags_capability_governance::publish_capability_snapshots(&self.runtime_home, snapshots)?;
        for host in &change.deactivated_hosts {
            let path = ags_capability_governance::snapshot_path(&self.runtime_home, host);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot retire {}: {error}", path.display())),
            }
        }
        Ok(hashes)
    }

    pub(super) fn apply_change(
        &self,
        plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<MaintenanceExecution, String> {
        self.apply_change_internal(plan, change, true)
    }

    pub(super) fn apply_change_deferred(
        &self,
        plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<MaintenanceExecution, String> {
        self.apply_change_internal(plan, change, false)
    }

    fn apply_change_internal(
        &self,
        plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
        refresh_snapshot_state: bool,
    ) -> Result<MaintenanceExecution, String> {
        let affected_hosts = Self::affected_hosts(change);
        let snapshots = self.capture_snapshots(&plan.plan_hash, &affected_hosts)?;
        let projection = match apply_required_suite_skill_projection(
            &self.runtime_home,
            change,
            &plan.plan_hash,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                self.restore_snapshots(&snapshots)?;
                self.remove_snapshot_recovery(&plan.plan_hash)?;
                return Err(error);
            }
        };
        let hashes = match if refresh_snapshot_state {
            self.compile_and_publish_suite_snapshots(change)
        } else {
            Ok(BTreeMap::new())
        } {
            Ok(hashes) => hashes,
            Err(error) => {
                let snapshot_recovery = self.restore_snapshots(&snapshots);
                let projection_recovery = projection.recover();
                let snapshot_cleanup = snapshot_recovery
                    .as_ref()
                    .map(|_| self.remove_snapshot_recovery(&plan.plan_hash))
                    .unwrap_or_else(|_| Ok(()));
                return Err(format!(
                    "{error}; snapshot recovery={}; snapshot cleanup={}; projection recovery={}",
                    snapshot_recovery
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|error| error),
                    snapshot_cleanup
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|error| error),
                    projection_recovery
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|error| error),
                ));
            }
        };
        Ok(MaintenanceExecution {
            status: MaintenanceStatus::Applied,
            applied_writes: plan.planned_writes.clone(),
            verification_results: vec![VerificationResult {
                id: "suite-skill-projection".to_string(),
                passed: true,
                evidence: plan.plan_hash.clone(),
            }],
            activation_results: hashes
                .into_iter()
                .map(|(host, hash)| ActivationResult {
                    host,
                    activated: true,
                    repreflight_passed: false,
                    route_verified: false,
                    evidence: hash,
                })
                .collect(),
            recovery_status: "available".to_string(),
            error: None,
        })
    }

    pub(super) fn verify_change(
        &self,
        _plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<MaintenanceExecution, String> {
        verify_required_suite_skill_projection_with_runtime(change, Some(&self.runtime_home))?;
        let mut results = Vec::new();
        let mut activations = Vec::new();
        let mut all_passed = true;
        for host in &change.hosts {
            let (snapshot, _) =
                ags_capability_governance::load_static_snapshot(&self.runtime_home, host).map_err(
                    |error| format!("cannot load `{host}` capability snapshot: {error:?}"),
                )?;
            let tables = snapshot
                .validate_integrity(host)
                .map_err(|error| format!("invalid `{host}` capability snapshot: {error:?}"))?;
            let routes_passed =
                verify_required_skills(&snapshot, &tables, &change.required_skills, host).is_ok();
            let agent = ags_workspace_facts::AgentType::from_str(host)
                .map_err(|error| format!("cannot map suite Host `{host}`: {error}"))?;
            let preflight =
                ags_workspace_facts::run_session_preflight(&self.preflight_target, &agent);
            let passed = routes_passed && preflight.exit_code == 0;
            all_passed &= passed;
            results.push(VerificationResult {
                id: format!("suite-skill-route-{host}"),
                passed,
                evidence: format!(
                    "snapshot={} routes={} preflight={}",
                    snapshot.snapshot_hash, routes_passed, preflight.exit_code
                ),
            });
            activations.push(ActivationResult {
                host: host.clone(),
                activated: true,
                repreflight_passed: preflight.exit_code == 0,
                route_verified: routes_passed,
                evidence: snapshot.snapshot_hash,
            });
        }
        Ok(MaintenanceExecution {
            status: if all_passed {
                MaintenanceStatus::Verified
            } else {
                MaintenanceStatus::Failed
            },
            applied_writes: Vec::new(),
            verification_results: results,
            activation_results: activations,
            recovery_status: "not-required".to_string(),
            error: (!all_passed)
                .then(|| "required suite Skills did not verify on every Host".to_string()),
        })
    }

    pub(super) fn recover_change(
        &self,
        plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<MaintenanceExecution, String> {
        let affected_hosts = Self::affected_hosts(change);
        let snapshots = self.load_snapshot_recovery(&plan.plan_hash, &affected_hosts)?;
        recover_required_suite_skill_projection(&self.runtime_home, change, &plan.plan_hash)?;
        self.restore_snapshots(&snapshots)?;
        self.remove_snapshot_recovery(&plan.plan_hash)?;
        Ok(MaintenanceExecution {
            status: MaintenanceStatus::Recovered,
            applied_writes: Vec::new(),
            verification_results: vec![VerificationResult {
                id: "suite-skill-recovery".to_string(),
                passed: true,
                evidence: plan.plan_hash.clone(),
            }],
            activation_results: Vec::new(),
            recovery_status: "recovered".to_string(),
            error: None,
        })
    }

    pub(super) fn recover_incomplete_change(
        &self,
        plan: &MaintenancePlan,
        change: &PreparedSuiteSkillProjection,
    ) -> Result<(), String> {
        let snapshot_path = self.snapshot_recovery_path(&plan.plan_hash);
        let projection_path = crate::suite_skill_projection::suite_skill_recovery_path(
            &self.runtime_home,
            &plan.plan_hash,
        );
        if projection_path.is_file() {
            recover_required_suite_skill_projection(&self.runtime_home, change, &plan.plan_hash)?;
        }
        if snapshot_path.is_file() {
            let affected_hosts = Self::affected_hosts(change);
            let snapshots = self.load_snapshot_recovery(&plan.plan_hash, &affected_hosts)?;
            self.restore_snapshots(&snapshots)?;
            self.remove_snapshot_recovery(&plan.plan_hash)?;
        }
        Ok(())
    }
}

fn verify_required_skills(
    snapshot: &ags_capability_governance::HostCapabilitySnapshot,
    tables: &ags_capability_governance::ActiveCapabilityTables,
    required_skills: &[String],
    host: &str,
) -> Result<(), String> {
    use ags_capability_governance::{GovernanceState, SkillRoutingSurface, SkillSourceKind};

    for skill_id in required_skills {
        let card = snapshot
            .catalog
            .iter()
            .find(|card| card.skill_id == *skill_id)
            .ok_or_else(|| {
                format!("required Skill `{skill_id}` is absent from `{host}` candidate catalog")
            })?;
        if card.source_kind != SkillSourceKind::Suite {
            return Err(format!(
                "required Skill `{skill_id}` on `{host}` is not sourced from the suite authority"
            ));
        }
        match card.routing_surface {
            SkillRoutingSurface::SkillTarget => {
                ags_capability_governance::resolve_skill(
                    skill_id,
                    None,
                    &snapshot.snapshot_hash,
                    &tables.skills,
                )
                .map_err(|error| {
                    format!(
                        "required SkillTarget `{skill_id}` does not resolve on `{host}` candidate snapshot: {error:?}"
                    )
                })?;
            }
            SkillRoutingSurface::HostCommand => {
                if !card.availability.is_ready()
                    || card
                        .routing_hint
                        .as_deref()
                        .is_none_or(|hint| hint.trim().is_empty())
                {
                    return Err(format!(
                        "required HostCommand `{skill_id}` is not ready on `{host}`: {:?}",
                        card.reason_codes
                    ));
                }
                if tables
                    .skills
                    .active_skills()
                    .iter()
                    .any(|skill| skill.skill_id == *skill_id)
                {
                    return Err(format!(
                        "required HostCommand `{skill_id}` leaked into `{host}` SkillTarget table"
                    ));
                }
            }
            SkillRoutingSurface::NotRoutable => {
                let invalid_reason = card.reason_codes.iter().find(|reason| {
                    matches!(
                        reason.as_str(),
                        "canonical_missing"
                            | "host_not_visible"
                            | "health_degraded"
                            | "auth_required"
                            | "metadata_incomplete"
                    )
                });
                if card.governance != GovernanceState::ManagedInactive
                    || invalid_reason.is_some()
                    || tables
                        .skills
                        .active_skills()
                        .iter()
                        .any(|skill| skill.skill_id == *skill_id)
                {
                    return Err(format!(
                        "required non-routable Skill `{skill_id}` has an invalid `{host}` projection: {:?}",
                        card.reason_codes
                    ));
                }
            }
        }
    }
    Ok(())
}

impl MaintenanceBackend for SuiteSkillMaintenanceBackend {
    fn prepare(&self, intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String> {
        if intent.subject != MaintenanceSubject::Runtime
            || !matches!(
                intent.operation,
                MaintenanceOperation::Install
                    | MaintenanceOperation::Update
                    | MaintenanceOperation::Repair
            )
            || intent.target != "suite-skills"
        {
            return Err(
                "suite Skill backend accepts runtime install/update/repair for target=suite-skills"
                    .to_string(),
            );
        }
        let change = match &self.prepared_change {
            Some(change) => change.clone(),
            None => crate::suite_skill_projection::plan_required_suite_skill_projection(
                &self.source_root,
                &self.runtime_home,
                &self.host_home,
                &self.policy,
            )?,
        };
        let risks = change
            .blocking_findings
            .iter()
            .enumerate()
            .map(|(index, finding)| RiskFinding {
                id: format!("suite-skill-blocking-{index}"),
                class: RiskClass::Blocking,
                summary: finding.clone(),
                evidence_hash: Some(ags_platform::sha256(finding.as_bytes())),
            })
            .collect();
        let mut planned_writes = change
            .operations
            .iter()
            .map(|operation| PlannedWrite {
                operation: format!("{:?}", operation.kind).to_ascii_lowercase(),
                path: operation.link_path.to_string_lossy().into_owned(),
                before_hash: None,
                after_hash: operation
                    .desired_target
                    .as_ref()
                    .map(|target| ags_platform::sha256(target.to_string_lossy().as_bytes())),
            })
            .collect::<Vec<_>>();
        planned_writes.extend(change.deactivated_hosts.iter().map(|host| {
            let path = ags_capability_governance::snapshot_path(&self.runtime_home, host);
            PlannedWrite {
                operation: "remove_deactivated_snapshot".to_string(),
                path: path.to_string_lossy().into_owned(),
                before_hash: ags_platform::sha256_file(&path).ok(),
                after_hash: None,
            }
        }));
        let activation = change
            .hosts
            .iter()
            .map(|host| ActivationRequirement {
                host: host.clone(),
                requires_restart: true,
                requires_repreflight: true,
                expected_snapshot_hash: None,
                exact_route_target: None,
            })
            .collect();
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "required_skill_count".to_string(),
            change.required_skills.len().to_string(),
        );
        metadata.insert(
            "authority_root".to_string(),
            change.authority_root.to_string_lossy().into_owned(),
        );
        Ok(PreparedMaintenance {
            current_version: None,
            target_version: None,
            source: None,
            planned_writes,
            risks,
            verification_steps: vec![VerificationStep {
                id: "suite-skills-selected-host-route".to_string(),
                description: "verify every required Skill resolves from the sealed snapshot on every selected Host".to_string(),
            }],
            activation,
            recovery_point: None,
            metadata,
            payload: Some(MaintenancePayload::SuiteSkills(change)),
        })
    }

    fn apply(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.change(plan)?;
        self.apply_change(plan, change)
    }

    fn verify(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.change(plan)?;
        self.verify_change(plan, change)
    }

    fn recover(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        let change = self.change(plan)?;
        self.recover_change(plan, change)
    }
}
