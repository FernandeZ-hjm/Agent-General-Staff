//! Contract-v2 read-only health inspection.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DOCTOR_REPORT_SCHEMA: &str = "ags://schema/contract/v2/doctor-report";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReport {
    pub schema_version: String,
    pub canonical_workspace: String,
    pub status: CheckStatus,
    pub project_tests_run: bool,
    pub findings: Vec<Finding>,
}

impl HealthReport {
    pub fn passed(&self) -> bool {
        self.status != CheckStatus::Fail
    }

    pub fn exit_code(&self) -> i32 {
        i32::from(!self.passed())
    }
}

/// Inspect only governance/runtime facts. This function never launches project
/// tests and never writes repair state.
pub fn inspect(workspace: &Path, runtime_home: &Path) -> HealthReport {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let mut findings = Vec::new();
    check(
        &mut findings,
        "workspace-git",
        canonical.join(".git").exists(),
        "workspace has Git identity",
        "initialize or select the intended Git workspace",
    );
    check(
        &mut findings,
        "governed-entry",
        canonical.join("AGENTS.md").is_file(),
        "AGS entrypoint is present",
        "run `ags init --workspace <repo>` and apply the sealed plan",
    );
    let profile = canonical.join("config/agent-project-profile.yaml");
    let profile_v2 = std::fs::read_to_string(&profile).is_ok_and(|body| {
        body.lines()
            .any(|line| line.trim() == "schema_version: ags://schema/contract/v2/project-profile")
    });
    check(
        &mut findings,
        "project-profile-v2",
        profile_v2,
        "structured contract-v2 project profile is present",
        "run `ags init --workspace <repo>`; modified user files are preserved",
    );
    let manifest = runtime_home.join("install-manifest.json");
    let runtime_v2 = std::fs::read_to_string(&manifest)
        .is_ok_and(|body| body.contains("ags://schema/contract/v2/runtime-install"));
    findings.push(Finding {
        id: "runtime-manifest-v2".to_string(),
        status: if runtime_v2 {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        },
        message: if runtime_v2 {
            "machine runtime manifest is contract v2".to_string()
        } else {
            "machine runtime is not initialized for contract v2".to_string()
        },
        remediation: (!runtime_v2)
            .then(|| "run `ags setup --workspace <repo>` and apply the sealed plan".to_string()),
    });
    let status = if findings
        .iter()
        .any(|finding| finding.status == CheckStatus::Fail)
    {
        CheckStatus::Fail
    } else if findings
        .iter()
        .any(|finding| finding.status == CheckStatus::Warn)
    {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    };
    HealthReport {
        schema_version: DOCTOR_REPORT_SCHEMA.to_string(),
        canonical_workspace: canonical.to_string_lossy().into_owned(),
        status,
        project_tests_run: false,
        findings,
    }
}

pub fn run(workspace: &Path) -> HealthReport {
    inspect(workspace, &ags_platform::runtime_home())
}

pub fn render_json(report: &HealthReport) -> String {
    serde_json::to_string(report).expect("doctor report serializes")
}

pub fn render_text(report: &HealthReport) -> String {
    let failed = report
        .findings
        .iter()
        .filter(|finding| finding.status == CheckStatus::Fail)
        .count();
    let warned = report
        .findings
        .iter()
        .filter(|finding| finding.status == CheckStatus::Warn)
        .count();
    format!(
        "AGS doctor {:?}\nworkspace: {}\nfindings: {} ({} failed, {} warning)\nproject tests run: false",
        report.status,
        report.canonical_workspace,
        report.findings.len(),
        failed,
        warned
    )
}

fn check(findings: &mut Vec<Finding>, id: &str, passed: bool, success: &str, remediation: &str) {
    findings.push(Finding {
        id: id.to_string(),
        status: if passed {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        message: if passed { success } else { remediation }.to_string(),
        remediation: (!passed).then(|| remediation.to_string()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_is_read_only_and_never_runs_project_tests() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".git")).unwrap();
        std::fs::create_dir_all(temp.path().join("config")).unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "# governed\n").unwrap();
        std::fs::write(
            temp.path().join("config/agent-project-profile.yaml"),
            "schema_version: ags://schema/contract/v2/project-profile\n",
        )
        .unwrap();
        let before = ags_platform::sha256(std::fs::read(temp.path().join("AGENTS.md")).unwrap());
        let report = inspect(temp.path(), &temp.path().join("runtime"));
        assert!(report.passed());
        assert!(!report.project_tests_run);
        assert_eq!(
            before,
            ags_platform::sha256(std::fs::read(temp.path().join("AGENTS.md")).unwrap())
        );
    }
}
