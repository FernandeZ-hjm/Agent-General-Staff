use super::model::*;
use super::store;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ServiceContext {
    pub runtime_home: PathBuf,
    pub binding_id: String,
    pub clock: ServiceClock,
    pub plan_ttl_seconds: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum ServiceClock {
    System,
    Fixed(u64),
}

impl ServiceClock {
    fn now_unix(self) -> Result<u64, String> {
        match self {
            Self::Fixed(value) => Ok(value),
            Self::System => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .map_err(|error| format!("system clock is before UNIX epoch: {error}")),
        }
    }
}

pub trait MaintenanceBackend {
    fn prepare(&self, intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String>;

    /// Execute the already sealed plan. A backend must perform subject-specific
    /// rollback before returning a failed execution after its first mutation.
    fn apply(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String>;

    fn verify(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String>;

    fn recover(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String>;
}

pub struct MaintenanceService<B> {
    context: ServiceContext,
    backend: B,
}

impl<B: MaintenanceBackend> MaintenanceService<B> {
    pub fn new(context: ServiceContext, backend: B) -> Result<Self, String> {
        if context.binding_id.trim().is_empty() {
            return Err("maintenance binding id must not be empty".to_string());
        }
        if context.plan_ttl_seconds == 0 {
            return Err("maintenance plan ttl must be greater than zero".to_string());
        }
        Ok(Self { context, backend })
    }

    pub fn plan(&self, intent: MaintenanceIntent) -> Result<MaintenancePlan, String> {
        validate_intent(&intent)?;
        let prepared = self.backend.prepare(&intent)?;
        let now_unix = self.context.clock.now_unix()?;
        let required_acknowledgements = prepared
            .risks
            .iter()
            .filter(|risk| risk.class == RiskClass::AcknowledgementRequired)
            .map(|risk| risk.id.clone())
            .collect();
        let mut plan = MaintenancePlan {
            schema_version: MAINTENANCE_PLAN_SCHEMA.to_string(),
            plan_hash: String::new(),
            binding_id: self.context.binding_id.clone(),
            created_at_unix: now_unix,
            expires_at_unix: now_unix
                .checked_add(self.context.plan_ttl_seconds)
                .ok_or_else(|| "maintenance plan expiry overflow".to_string())?,
            intent,
            current_version: prepared.current_version,
            target_version: prepared.target_version,
            source: prepared.source,
            planned_writes: prepared.planned_writes,
            risks: prepared.risks,
            verification_steps: prepared.verification_steps,
            activation: prepared.activation,
            recovery_point: prepared.recovery_point,
            required_acknowledgements,
            metadata: prepared.metadata,
            payload: prepared.payload,
        };
        plan.seal()?;
        store::persist_plan(&self.context.runtime_home, &plan)?;
        Ok(plan)
    }

    pub fn apply(
        &self,
        plan_hash: &str,
        acknowledgements: &BTreeSet<String>,
    ) -> Result<MaintenanceReceipt, String> {
        let plan = self.load_bound_plan(plan_hash)?;
        if let Some(receipt) =
            store::load_apply_receipt_optional(&self.context.runtime_home, plan_hash)?
        {
            if receipt.binding_id != self.context.binding_id || receipt.plan_hash != plan.plan_hash
            {
                return Err("maintenance apply receipt binding mismatch".to_string());
            }
            return match receipt.status {
                MaintenanceStatus::Verified => Ok(receipt),
                _ => Err(
                    "maintenance plan already ended without a verified state; create a new plan"
                        .to_string(),
                ),
            };
        }
        if plan
            .risks
            .iter()
            .any(|risk| risk.class == RiskClass::Blocking)
        {
            return Err("maintenance plan contains blocking safety findings".to_string());
        }
        if self.context.clock.now_unix()? > plan.expires_at_unix {
            return Err("maintenance plan expired; create a new plan".to_string());
        }
        let missing = plan
            .required_acknowledgements
            .difference(acknowledgements)
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "maintenance risks require acknowledgement: {}",
                missing.join(",")
            ));
        }
        let known = plan
            .risks
            .iter()
            .map(|risk| risk.id.clone())
            .collect::<BTreeSet<_>>();
        let unknown = acknowledgements
            .difference(&known)
            .cloned()
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            return Err(format!(
                "maintenance acknowledgements are not in this plan: {}",
                unknown.join(",")
            ));
        }
        let applied = self.backend.apply(&plan)?;
        if applied.status != MaintenanceStatus::Applied {
            return Err("maintenance backend did not return an applied state".to_string());
        }

        // Apply owns the product closure: activating state without proving the
        // resulting route is never a successful transaction. `verify` remains
        // available as a later health recheck, but callers cannot accidentally
        // omit the first verification or its rollback.
        let verification = self.backend.verify(&plan);
        let execution = match verification {
            Ok(verified) if verified.status == MaintenanceStatus::Verified => {
                merge_closed_execution(applied, verified)
            }
            Ok(failed) => self.recover_failed_verification(&plan, applied, failed.error.clone())?,
            Err(error) => self.recover_failed_verification(&plan, applied, Some(error))?,
        };
        self.finish(&plan, MaintenancePhase::Apply, execution)
    }

    pub fn status(&self, plan_hash: &str) -> Result<MaintenancePlan, String> {
        self.load_bound_plan(plan_hash)
    }

    pub fn verify(&self, plan_hash: &str) -> Result<MaintenanceReceipt, String> {
        let plan = self.load_bound_plan(plan_hash)?;
        let applied = store::load_apply_receipt(&self.context.runtime_home, plan_hash)?;
        if applied.binding_id != self.context.binding_id
            || applied.plan_hash != plan.plan_hash
            || applied.phase != MaintenancePhase::Apply
            || applied.status != MaintenanceStatus::Verified
        {
            return Err(
                "maintenance verification requires this binding's closed apply receipt".to_string(),
            );
        }
        let execution = self.backend.verify(&plan)?;
        self.finish(&plan, MaintenancePhase::Verify, execution)
    }

    pub fn recover(&self, plan_hash: &str) -> Result<MaintenanceReceipt, String> {
        let plan = self.load_bound_plan(plan_hash)?;
        let applied = store::load_apply_receipt(&self.context.runtime_home, plan_hash)?;
        if applied.binding_id != self.context.binding_id
            || applied.plan_hash != plan.plan_hash
            || applied.phase != MaintenancePhase::Apply
            || applied.status != MaintenanceStatus::Verified
        {
            return Err(
                "maintenance recovery requires this binding's closed apply receipt".to_string(),
            );
        }
        let execution = self.backend.recover(&plan)?;
        self.finish(&plan, MaintenancePhase::Recover, execution)
    }

    fn load_bound_plan(&self, plan_hash: &str) -> Result<MaintenancePlan, String> {
        let plan = self.load_plan(plan_hash)?;
        if plan.binding_id != self.context.binding_id {
            return Err("maintenance plan belongs to another connection".to_string());
        }
        Ok(plan)
    }

    fn recover_failed_verification(
        &self,
        plan: &MaintenancePlan,
        mut applied: MaintenanceExecution,
        verification_error: Option<String>,
    ) -> Result<MaintenanceExecution, String> {
        let error = verification_error
            .unwrap_or_else(|| "maintenance verification did not reach verified state".to_string());
        match self.backend.recover(plan) {
            Ok(recovered) if recovered.status == MaintenanceStatus::Recovered => {
                applied.status = MaintenanceStatus::FailedRecovered;
                applied
                    .verification_results
                    .extend(recovered.verification_results);
                applied
                    .activation_results
                    .extend(recovered.activation_results);
                applied.recovery_status = "recovered-after-verification-failure".to_string();
                applied.error = Some(error);
                Ok(applied)
            }
            Ok(recovered) => {
                applied.status = MaintenanceStatus::Failed;
                applied.recovery_status =
                    format!("unexpected-recovery-state:{:?}", recovered.status);
                applied.error = Some(error);
                Ok(applied)
            }
            Err(recovery_error) => {
                applied.status = MaintenanceStatus::Failed;
                applied.recovery_status = "recovery-failed".to_string();
                applied.error = Some(format!("{error}; recovery failed: {recovery_error}"));
                Ok(applied)
            }
        }
    }

    fn load_plan(&self, plan_hash: &str) -> Result<MaintenancePlan, String> {
        let plan = store::load_plan(&self.context.runtime_home, plan_hash)?;
        plan.verify_hash()?;
        if plan.plan_hash != plan_hash {
            return Err("maintenance plan identity mismatch".to_string());
        }
        Ok(plan)
    }

    fn finish(
        &self,
        plan: &MaintenancePlan,
        phase: MaintenancePhase,
        execution: MaintenanceExecution,
    ) -> Result<MaintenanceReceipt, String> {
        let completed_at_unix = self.context.clock.now_unix()?;
        let identity = serde_json::json!({
            "plan_hash": plan.plan_hash,
            "binding_id": self.context.binding_id,
            "completed_at_unix": completed_at_unix,
            "phase": phase,
            "status": execution.status,
        });
        let digest = ags_platform::sha256_hex(
            &serde_json::to_vec(&identity)
                .map_err(|error| format!("cannot seal maintenance receipt: {error}"))?,
        );
        let receipt = MaintenanceReceipt {
            schema_version: MAINTENANCE_RECEIPT_SCHEMA.to_string(),
            receipt_id: format!("mr-{}", &digest[..16]),
            plan_hash: plan.plan_hash.clone(),
            binding_id: self.context.binding_id.clone(),
            completed_at_unix,
            phase,
            status: execution.status,
            applied_writes: execution.applied_writes,
            verification_results: execution.verification_results,
            activation_results: execution.activation_results,
            recovery_status: execution.recovery_status,
            error: execution.error,
        };
        store::persist_receipt(&self.context.runtime_home, &receipt)?;
        Ok(receipt)
    }
}

fn merge_closed_execution(
    mut applied: MaintenanceExecution,
    verified: MaintenanceExecution,
) -> MaintenanceExecution {
    applied.status = MaintenanceStatus::Verified;
    applied
        .verification_results
        .extend(verified.verification_results);
    if !verified.activation_results.is_empty() {
        applied.activation_results = verified.activation_results;
    }
    applied.recovery_status = "available".to_string();
    applied.error = None;
    applied
}

fn validate_intent(intent: &MaintenanceIntent) -> Result<(), String> {
    if intent.schema_version != MAINTENANCE_INTENT_SCHEMA {
        return Err("maintenance intent schema mismatch".to_string());
    }
    if intent.request_id.trim().is_empty() || intent.target.trim().is_empty() {
        return Err("maintenance intent request_id and target are required".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FakeBackend {
        applied: Cell<u32>,
        recovered: Cell<u32>,
        fail_verification: bool,
    }

    impl MaintenanceBackend for FakeBackend {
        fn prepare(&self, _intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String> {
            Ok(PreparedMaintenance {
                current_version: Some("a".to_string()),
                target_version: Some("b".to_string()),
                source: None,
                planned_writes: vec![],
                risks: vec![RiskFinding {
                    id: "unreviewed-source".to_string(),
                    class: RiskClass::AcknowledgementRequired,
                    summary: "source was selected by the user".to_string(),
                    evidence_hash: None,
                }],
                verification_steps: vec![],
                activation: vec![],
                recovery_point: None,
                metadata: Default::default(),
                payload: None,
            })
        }

        fn apply(&self, _plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
            self.applied.set(self.applied.get() + 1);
            Ok(execution(MaintenanceStatus::Applied))
        }

        fn verify(&self, _plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
            if self.fail_verification {
                Ok(MaintenanceExecution {
                    error: Some("route verification failed".to_string()),
                    ..execution(MaintenanceStatus::Failed)
                })
            } else {
                Ok(execution(MaintenanceStatus::Verified))
            }
        }

        fn recover(&self, _plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
            self.recovered.set(self.recovered.get() + 1);
            Ok(execution(MaintenanceStatus::Recovered))
        }
    }

    fn execution(status: MaintenanceStatus) -> MaintenanceExecution {
        MaintenanceExecution {
            status,
            applied_writes: vec![],
            verification_results: vec![],
            activation_results: vec![],
            recovery_status: "not-required".to_string(),
            error: None,
        }
    }

    fn service_with_verification(
        root: &std::path::Path,
        binding: &str,
        now: u64,
        fail_verification: bool,
    ) -> MaintenanceService<FakeBackend> {
        MaintenanceService::new(
            ServiceContext {
                runtime_home: root.to_path_buf(),
                binding_id: binding.to_string(),
                clock: ServiceClock::Fixed(now),
                plan_ttl_seconds: 60,
            },
            FakeBackend {
                applied: Cell::new(0),
                recovered: Cell::new(0),
                fail_verification,
            },
        )
        .unwrap()
    }

    fn service(root: &std::path::Path, binding: &str, now: u64) -> MaintenanceService<FakeBackend> {
        service_with_verification(root, binding, now, false)
    }

    #[test]
    fn plan_is_hash_bound_and_requires_explicit_risk_acknowledgement() {
        let root = tempfile::tempdir().unwrap();
        let service = service(root.path(), "mcp-session-1", 100);
        let plan = service
            .plan(MaintenanceIntent::new(
                "request-1",
                MaintenanceSubject::Skill,
                MaintenanceOperation::Install,
                "example",
            ))
            .unwrap();
        plan.verify_hash().unwrap();

        let error = service
            .apply(&plan.plan_hash, &BTreeSet::new())
            .unwrap_err();
        assert!(error.contains("unreviewed-source"));
        assert_eq!(service.backend.applied.get(), 0);

        let receipt = service
            .apply(
                &plan.plan_hash,
                &["unreviewed-source".to_string()].into_iter().collect(),
            )
            .unwrap();
        assert_eq!(receipt.status, MaintenanceStatus::Verified);
        assert_eq!(receipt.phase, MaintenancePhase::Apply);
        assert_eq!(service.backend.applied.get(), 1);

        let repeated = service
            .apply(
                &plan.plan_hash,
                &["unreviewed-source".to_string()].into_iter().collect(),
            )
            .unwrap();
        assert_eq!(repeated.receipt_id, receipt.receipt_id);
        assert_eq!(service.backend.applied.get(), 1);

        let verified = service.verify(&plan.plan_hash).unwrap();
        assert_eq!(verified.phase, MaintenancePhase::Verify);
        assert_eq!(verified.status, MaintenanceStatus::Verified);
        let recovered = service.recover(&plan.plan_hash).unwrap();
        assert_eq!(recovered.phase, MaintenancePhase::Recover);
        assert_eq!(recovered.status, MaintenanceStatus::Recovered);
    }

    #[test]
    fn plan_is_connection_bound_and_expires_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let creator = service(root.path(), "mcp-session-1", 100);
        let plan = creator
            .plan(MaintenanceIntent::new(
                "request-1",
                MaintenanceSubject::Skill,
                MaintenanceOperation::Update,
                "example",
            ))
            .unwrap();
        let acknowledgements = ["unreviewed-source".to_string()].into_iter().collect();

        let other = service(root.path(), "mcp-session-2", 100);
        assert!(other
            .status(&plan.plan_hash)
            .unwrap_err()
            .contains("another connection"));
        assert!(other
            .apply(&plan.plan_hash, &acknowledgements)
            .unwrap_err()
            .contains("another connection"));

        let expired = service(root.path(), "mcp-session-1", 161);
        assert!(expired
            .apply(&plan.plan_hash, &acknowledgements)
            .unwrap_err()
            .contains("expired"));
    }

    #[test]
    fn verify_requires_a_closed_apply_receipt() {
        let root = tempfile::tempdir().unwrap();
        let service = service(root.path(), "mcp-session-1", 100);
        let plan = service
            .plan(MaintenanceIntent::new(
                "request-verify-before-apply",
                MaintenanceSubject::Skill,
                MaintenanceOperation::Install,
                "example",
            ))
            .unwrap();

        assert!(service
            .verify(&plan.plan_hash)
            .unwrap_err()
            .contains("no successful apply receipt"));
    }

    #[test]
    fn apply_verification_failure_recovers_before_returning() {
        let root = tempfile::tempdir().unwrap();
        let service = service_with_verification(root.path(), "mcp-session-1", 100, true);
        let plan = service
            .plan(MaintenanceIntent::new(
                "request-failed-route",
                MaintenanceSubject::Skill,
                MaintenanceOperation::Install,
                "example",
            ))
            .unwrap();
        let acknowledgements = ["unreviewed-source".to_string()].into_iter().collect();

        let receipt = service.apply(&plan.plan_hash, &acknowledgements).unwrap();
        assert_eq!(receipt.status, MaintenanceStatus::FailedRecovered);
        assert_eq!(
            receipt.recovery_status,
            "recovered-after-verification-failure"
        );
        assert_eq!(service.backend.applied.get(), 1);
        assert_eq!(service.backend.recovered.get(), 1);
        assert!(service.verify(&plan.plan_hash).is_err());
    }
}
