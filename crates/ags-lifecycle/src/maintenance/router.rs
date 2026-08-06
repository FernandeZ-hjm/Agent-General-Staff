use super::*;

/// Closed dispatch over the maintenance subjects implemented by the Rust
/// kernel. CLI and MCP construct the same router; transport adapters never
/// decide how a sealed payload is executed.
pub struct MaintenanceBackendRouter {
    pub skill: SkillMaintenanceBackend,
    pub suite_skills: SuiteSkillMaintenanceBackend,
}

impl MaintenanceBackend for MaintenanceBackendRouter {
    fn prepare(&self, intent: &MaintenanceIntent) -> Result<PreparedMaintenance, String> {
        match (intent.subject, intent.target.as_str()) {
            (MaintenanceSubject::Skill, _) => self.skill.prepare(intent),
            (MaintenanceSubject::Runtime, "suite-skills") => self.suite_skills.prepare(intent),
            _ => Err(format!(
                "no Rust maintenance backend for subject={:?} target={}",
                intent.subject, intent.target
            )),
        }
    }

    fn apply(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::Skill(_)) => self.skill.apply(plan),
            Some(MaintenancePayload::SuiteSkills(_)) => self.suite_skills.apply(plan),
            _ => Err("maintenance payload is not supported by this adapter".to_string()),
        }
    }

    fn verify(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::Skill(_)) => self.skill.verify(plan),
            Some(MaintenancePayload::SuiteSkills(_)) => self.suite_skills.verify(plan),
            _ => Err("maintenance payload is not supported by this adapter".to_string()),
        }
    }

    fn recover(&self, plan: &MaintenancePlan) -> Result<MaintenanceExecution, String> {
        match plan.payload.as_ref() {
            Some(MaintenancePayload::Skill(_)) => self.skill.recover(plan),
            Some(MaintenancePayload::SuiteSkills(_)) => self.suite_skills.recover(plan),
            _ => Err("maintenance payload is not supported by this adapter".to_string()),
        }
    }
}
