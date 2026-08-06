//! Shared runtime setup lifecycle.
//!
//! This module is the single authority for setup planning, mutation,
//! verification, generated templates, and host-memory
//! mutation. Human-facing adapters resolve CLI arguments, render the returned
//! presentations, and own process exit behaviour.

mod apply;
mod global_entry;
pub(crate) mod plan;
mod recommendations;
mod templates;
mod verify;

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use ags_host_integration::{claude_mcp_list_line_at, command_in_path};
use serde::{Deserialize, Serialize};

use global_entry::{
    global_entry_protocol_json, global_entry_protocol_plan, render_global_entry_protocol_text,
};
use plan::{
    render_runtime_plan_json, render_runtime_plan_text, runtime_install_plan,
    runtime_install_plan_with_hosts, runtime_install_plan_with_hosts_and_authority,
};
use recommendations::{render_third_party_recommendations_text, third_party_recommendations_json};

pub const RUNTIME_INSTALL_SCHEMA: &str = "0.4.13-runtime-install";
const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RUNTIME_SOURCE_REQUIRED_FILES: &[&str] = &[
    "manifests/suite.yaml",
    "manifests/skills-registry.yaml",
    "manifests/mcp-registry.yaml",
    "manifests/third-party-capabilities.yaml",
    "protocol/agent-task-protocol.md",
    "protocol/task-card-template.md",
    "protocol/runtime-adapters.md",
    "protocol/task-routing.md",
];

/// Whether `root` contains the complete typed input surface required by runtime
/// setup. A signed public runtime is intentionally source-free, so Rust
/// workspace files are not part of this product identity.
pub fn is_runtime_source_root(root: &Path) -> bool {
    RUNTIME_SOURCE_REQUIRED_FILES
        .iter()
        .all(|relative| root.join(relative).is_file())
}

pub fn approved_lifecycle_hosts(target: &Path) -> Result<Vec<String>, String> {
    let path = target.join("install-manifest.json");
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let hosts = value
        .pointer("/lifecycle/approved_hosts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let supported = ags_host_integration::lifecycle_specs()
        .map(|spec| spec.host_id)
        .collect::<std::collections::BTreeSet<_>>();
    let mut approved = std::collections::BTreeSet::new();
    for host in hosts {
        let host = host
            .as_str()
            .ok_or_else(|| "lifecycle.approved_hosts must contain strings".to_string())?;
        if !supported.contains(host) {
            return Err(format!("unsupported approved lifecycle host `{host}`"));
        }
        approved.insert(host.to_string());
    }
    Ok(approved.into_iter().collect())
}

/// Read the optional machine-local authority constraint persisted by setup.
/// Public installs omit this field and therefore project from their own suite
/// root. Machine-local tooling may set it once and every later setup/update
/// continues enforcing the same stable authority.
pub fn configured_suite_skill_authority_root(target: &Path) -> Result<Option<PathBuf>, String> {
    let path = target.join("install-manifest.json");
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    match value.pointer("/suite_skill_projection/required_authority_root") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(root)) if !root.trim().is_empty() => {
            Ok(Some(PathBuf::from(root)))
        }
        Some(_) => Err(
            "suite_skill_projection.required_authority_root must be a non-empty path string"
                .to_string(),
        ),
    }
}

pub(in crate::setup) fn lifecycle_selection_source(target: &Path) -> String {
    std::fs::read_to_string(target.join("install-manifest.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|value| {
            value
                .pointer("/lifecycle/selection_source")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .filter(|source| matches!(source.as_str(), "setup" | "agents-govern"))
        .unwrap_or_else(|| "setup".to_string())
}

pub fn add_approved_lifecycle_hosts(
    target: &Path,
    hosts: &[String],
) -> Result<Vec<String>, String> {
    let path = target.join("install-manifest.json");
    let body = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    let mut approved = approved_lifecycle_hosts(target)?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let supported = ags_host_integration::lifecycle_specs()
        .map(|spec| spec.host_id)
        .collect::<std::collections::BTreeSet<_>>();
    for host in hosts {
        if !supported.contains(host.as_str()) {
            return Err(format!("unsupported lifecycle host `{host}`"));
        }
        approved.insert(host.clone());
    }
    let approved = approved.into_iter().collect::<Vec<_>>();
    value["lifecycle"] = serde_json::json!({
        "approved_hosts": approved,
        "selection_source": "agents-govern"
    });
    let mut rendered = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    rendered.push(b'\n');
    ags_platform::atomic_write(&path, &rendered)
        .map_err(|error| format!("cannot update {}: {error}", path.display()))?;
    Ok(approved)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum SetupSeverity {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "fail")]
    Fail,
}

impl fmt::Display for SetupSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(formatter, "INFO"),
            Self::Warn => write!(formatter, "WARN"),
            Self::Fail => write!(formatter, "FAIL"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SetupCheckStatus {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "skip")]
    Skip,
}

impl fmt::Display for SetupCheckStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(formatter, "PASS"),
            Self::Fail => write!(formatter, "FAIL"),
            Self::Warn => write!(formatter, "WARN"),
            Self::Skip => write!(formatter, "SKIP"),
        }
    }
}

impl SetupCheckStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Pass | Self::Skip)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetupFinding {
    pub check_name: String,
    pub status: SetupCheckStatus,
    pub severity: SetupSeverity,
    pub message: String,
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl SetupFinding {
    pub fn pass(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: SetupCheckStatus::Pass,
            severity: SetupSeverity::Info,
            message: message.into(),
            detail: None,
            expected: None,
            observed: None,
            remediation: None,
        }
    }

    pub fn fail(
        check_name: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            check_name: check_name.into(),
            status: SetupCheckStatus::Fail,
            severity: SetupSeverity::Fail,
            message: message.into(),
            detail: Some(detail.into()),
            expected: None,
            observed: None,
            remediation: None,
        }
    }

    pub fn warn(
        check_name: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            check_name: check_name.into(),
            status: SetupCheckStatus::Warn,
            severity: SetupSeverity::Warn,
            message: message.into(),
            detail: Some(detail.into()),
            expected: None,
            observed: None,
            remediation: None,
        }
    }

    pub fn info(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::pass(check_name, message)
    }

    pub fn skip(check_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: SetupCheckStatus::Skip,
            severity: SetupSeverity::Info,
            message: reason.into(),
            detail: None,
            expected: None,
            observed: None,
            remediation: None,
        }
    }

    pub fn with_conformance(
        mut self,
        expected: impl Into<String>,
        observed: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        self.expected = Some(expected.into());
        self.observed = Some(observed.into());
        self.remediation = Some(remediation.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SetupReport {
    pub title: String,
    pub findings: Vec<SetupFinding>,
}

impl SetupReport {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            findings: Vec::new(),
        }
    }

    pub fn add(&mut self, finding: SetupFinding) {
        self.findings.push(finding);
    }

    pub fn passed(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity == SetupSeverity::Fail)
    }

    pub fn exit_code(&self) -> i32 {
        if self.passed() {
            0
        } else {
            1
        }
    }

    pub fn total_failures(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == SetupSeverity::Fail)
            .count()
    }

    pub fn total_warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == SetupSeverity::Warn)
            .count()
    }

    pub fn total_infos(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == SetupSeverity::Info)
            .count()
    }

    pub fn total_skipped(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == SetupCheckStatus::Skip)
            .count()
    }

    pub fn total_passed_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == SetupCheckStatus::Pass)
            .count()
    }

    pub fn total_failed_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == SetupCheckStatus::Fail)
            .count()
    }

    pub fn total_warned_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == SetupCheckStatus::Warn)
            .count()
    }

    pub fn total(&self) -> usize {
        self.findings.len()
    }
}

type InstallFile = crate::maintenance::RuntimeInstallFile;

#[derive(Debug, Clone)]
pub struct SetupHostEntry {
    pub id: String,
    pub display: String,
    pub config_subdirs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimePlanPresentation {
    pub install_json: serde_json::Value,
    pub install_text: String,
    pub global_entry_json: serde_json::Value,
    pub global_entry_text: String,
    pub recommendations_json: serde_json::Value,
    pub recommendations_text: String,
}

/// Build the complete setup-owned plan presentation without writing host or
/// runtime state. Cross-host detection is deliberately supplied by the host
/// adapter because it is shared with `ags agents`.
pub fn runtime_plan_presentation(
    source_root: &Path,
    target: &Path,
    home: &Path,
    host_entries: &[SetupHostEntry],
    approved_hosts: &[String],
    suite_skill_authority_root: Option<&Path>,
) -> RuntimePlanPresentation {
    let selection_source = lifecycle_selection_source(target);
    let plan = runtime_install_plan_with_hosts_and_authority(
        source_root,
        target,
        home,
        approved_hosts,
        &selection_source,
        suite_skill_authority_root,
    );
    let entries = global_entry_protocol_plan(&plan, host_entries);
    RuntimePlanPresentation {
        install_json: serde_json::from_str(&render_runtime_plan_json(&plan))
            .unwrap_or_else(|_| serde_json::json!({})),
        install_text: render_runtime_plan_text(&plan),
        global_entry_json: global_entry_protocol_json(&entries),
        global_entry_text: render_global_entry_protocol_text(&entries),
        recommendations_json: third_party_recommendations_json(source_root, target, home),
        recommendations_text: render_third_party_recommendations_text(source_root, target, home),
    }
}

#[derive(Debug, serde::Serialize)]
pub struct RuntimeApplyResult {
    pub report: crate::setup::SetupReport,
    pub target: PathBuf,
    pub plan_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration: Option<crate::maintenance::RuntimeMigrationReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maintenance_plan: Option<crate::maintenance::MaintenancePlan>,
    pub maintenance_receipts: Vec<crate::maintenance::MaintenanceReceipt>,
}

impl RuntimeApplyResult {
    fn partial(
        report: crate::setup::SetupReport,
        target: &Path,
        plan_text: String,
        migration: Option<crate::maintenance::RuntimeMigrationReceipt>,
    ) -> Self {
        Self {
            report,
            target: target.to_path_buf(),
            plan_text,
            migration,
            maintenance_plan: None,
            maintenance_receipts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeApplyRequest<'a> {
    pub source_root: &'a Path,
    pub target: &'a Path,
    pub home: &'a Path,
    pub force: bool,
    pub approved_lifecycle_hosts: Option<&'a [String]>,
    pub suite_skill_authority_root: Option<&'a Path>,
}

/// Apply one already-authorized runtime setup transaction.
///
/// The caller owns target protection and confirmation. This function owns the
/// mutation sequence and returns evidence instead of rendering or exiting.
pub fn apply_runtime(request: RuntimeApplyRequest<'_>) -> RuntimeApplyResult {
    let mut report = crate::setup::SetupReport::new("runtime-install-apply");
    let selected_hosts = request
        .approved_lifecycle_hosts
        .map(<[String]>::to_vec)
        .unwrap_or_else(|| approved_lifecycle_hosts(request.target).unwrap_or_default());
    if selected_hosts.is_empty() {
        report.add(crate::setup::SetupFinding::fail(
            "runtime-setup-host-selection",
            "AGS setup requires at least one Agent Host",
            "select the current Host explicitly or rerun setup after Host detection succeeds",
        ));
        return RuntimeApplyResult::partial(
            report,
            request.target,
            "Runtime setup is blocked until at least one Host is selected.".to_string(),
            None,
        );
    }
    let migration = match crate::maintenance::migrate_runtime_state(request.target) {
        Ok(receipt) => receipt,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "stable-runtime-state-migration",
                "could not establish the stable runtime fact store",
                error,
            ));
            return RuntimeApplyResult::partial(
                report,
                request.target,
                "Stable runtime state migration failed before setup planning.".to_string(),
                None,
            );
        }
    };
    report.add(crate::setup::SetupFinding::pass(
        "stable-runtime-state-migration",
        format!(
            "stable runtime fact store {:?}: {} ({})",
            migration.status, migration.stable_root, migration.state_hash
        ),
    ));
    if let Err(error) = crate::maintenance::recover_incomplete_runtime_setups(request.target) {
        report.add(crate::setup::SetupFinding::fail(
            "runtime-setup-wal-recovery",
            "could not recover the previous incomplete runtime transaction",
            error,
        ));
        return RuntimeApplyResult::partial(
            report,
            request.target,
            "Runtime recovery failed before setup planning.".to_string(),
            Some(migration),
        );
    }
    report.add(crate::setup::SetupFinding::pass(
        "runtime-setup-wal-recovery",
        "no incomplete runtime transaction remains before planning",
    ));
    let plan = match (
        request.approved_lifecycle_hosts,
        request.suite_skill_authority_root,
    ) {
        (Some(hosts), Some(authority)) => runtime_install_plan_with_hosts_and_authority(
            request.source_root,
            request.target,
            request.home,
            hosts,
            "setup",
            Some(authority),
        ),
        (Some(hosts), None) => runtime_install_plan_with_hosts(
            request.source_root,
            request.target,
            request.home,
            hosts,
            "setup",
        ),
        (None, Some(authority)) => runtime_install_plan_with_hosts_and_authority(
            request.source_root,
            request.target,
            request.home,
            &approved_lifecycle_hosts(request.target).unwrap_or_default(),
            &lifecycle_selection_source(request.target),
            Some(authority),
        ),
        (None, None) => runtime_install_plan(request.source_root, request.target, request.home),
    };
    let mut plan_text = render_runtime_plan_text(&plan);
    let projection = match &plan.suite_skill_projection {
        Ok(projection) if projection.blocking_findings.is_empty() => projection.clone(),
        Ok(projection) => {
            report.add(crate::setup::SetupFinding::fail(
                "suite-skill-projection-plan",
                "required suite Skill projection is blocked",
                projection.blocking_findings.join("; "),
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "suite-skill-projection-plan",
                "required suite Skill projection could not be planned",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    let files = plan
        .files
        .iter()
        .filter(|file| apply::codex_skill_thin_index_ancestor(&file.path).is_none())
        .cloned()
        .collect::<Vec<_>>();
    let file_before_state_hashes = match files
        .iter()
        .map(|file| {
            crate::maintenance::path_state_hash(&file.path).map(|hash| (file.path.clone(), hash))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()
    {
        Ok(hashes) => hashes,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "runtime-setup-path-state",
                "could not seal runtime file state",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    let cleanup_before_state_hashes = match plan
        .cleanup_paths
        .iter()
        .map(|path| crate::maintenance::path_state_hash(path).map(|hash| (path.clone(), hash)))
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()
    {
        Ok(hashes) => hashes,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "runtime-setup-cleanup-state",
                "could not seal retired runtime entry state",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    let installed =
        match ags_capability_governance::skill_adoption::load_installed_skills(request.target) {
            Ok(installed) => installed,
            Err(error) => {
                report.add(crate::setup::SetupFinding::fail(
                    "legacy-suite-skill-migration-index",
                    "could not read the installed Skill fact store",
                    error,
                ));
                return RuntimeApplyResult::partial(
                    report,
                    request.target,
                    plan_text,
                    Some(migration),
                );
            }
        };
    let catalog_ids =
        match ags_capability_governance::third_party_manifest::read_third_party_manifest(
            request.source_root,
        ) {
            Ok(manifest) => manifest
                .capabilities
                .into_iter()
                .filter(|capability| {
                    capability.kind
                        == ags_capability_governance::third_party_manifest::CapabilityKind::Skill
                })
                .map(|entry| entry.id)
                .collect::<BTreeSet<_>>(),
            Err(error) => {
                report.add(crate::setup::SetupFinding::fail(
                    "legacy-suite-skill-catalog",
                    "could not read the canonical Skill catalog",
                    error,
                ));
                return RuntimeApplyResult::partial(
                    report,
                    request.target,
                    plan_text,
                    Some(migration),
                );
            }
        };
    let retired_catalog_ids = projection
        .operations
        .iter()
        .filter(|operation| {
            operation.kind == crate::suite_skill_projection::ProjectionOperationKind::RemoveRetired
                && catalog_ids.contains(&operation.skill_id)
        })
        .map(|operation| operation.skill_id.clone())
        .collect::<BTreeSet<_>>();
    let adoption_context = ags_capability_governance::skill_adoption::AdoptionContext {
        authority_root: request.source_root.to_path_buf(),
        runtime_home: request.target.to_path_buf(),
        host_home: request.home.to_path_buf(),
        snapshot_discovery: ags_capability_governance::skill_adoption::SnapshotDiscovery::Live,
    };
    let mut legacy_skill_migrations = match retired_catalog_ids
        .iter()
        .map(|skill_id| {
            ags_capability_governance::skill_adoption::plan_legacy_catalog_migration(
                &adoption_context,
                &projection
                    .authority_root
                    .join("global-skills")
                    .join(skill_id),
                skill_id,
                &projection.hosts,
            )
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(migrations) => migrations,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "legacy-suite-skill-migration-plan",
                "could not seal the one-way third-party Skill migration",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    // All migrations are sealed together but the InstalledSkillIndex retains
    // a monotonic CAS revision. Bind each prepared change to the revision
    // produced by its predecessor so the batch cannot skip or reorder writes.
    for (offset, migration) in legacy_skill_migrations.iter_mut().enumerate() {
        migration.registry_revision = installed.revision + offset as u64;
    }
    let prepared_setup = crate::maintenance::PreparedRuntimeSetup {
        source_root: request.source_root.to_path_buf(),
        runtime_home: request.target.to_path_buf(),
        host_home: request.home.to_path_buf(),
        force: request.force,
        files,
        file_before_state_hashes,
        cleanup_paths: plan.cleanup_paths.clone(),
        cleanup_before_state_hashes,
        suite_skills: projection,
        legacy_skill_migrations,
    };
    let binding_material = format!(
        "{}\n{}\n{}",
        request.source_root.display(),
        request.target.display(),
        request.home.display()
    );
    let backend = crate::maintenance::RuntimeSetupMaintenanceBackend {
        runtime_home: request.target.to_path_buf(),
        prepared_change: Some(prepared_setup),
    };
    let service = match crate::maintenance::MaintenanceService::new(
        crate::maintenance::ServiceContext {
            runtime_home: request.target.to_path_buf(),
            binding_id: format!(
                "setup-{}",
                ags_platform::sha256(binding_material.as_bytes())
            ),
            clock: crate::maintenance::ServiceClock::System,
            plan_ttl_seconds: 60 * 60,
        },
        backend,
    ) {
        Ok(service) => service,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "suite-skill-maintenance-service",
                "could not initialize suite Skill maintenance",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    let maintenance_plan = match service.plan(crate::maintenance::MaintenanceIntent::new(
        "runtime-setup",
        crate::maintenance::MaintenanceSubject::Runtime,
        crate::maintenance::MaintenanceOperation::Repair,
        "setup",
    )) {
        Ok(maintenance_plan) => maintenance_plan,
        Err(error) => {
            report.add(crate::setup::SetupFinding::fail(
                "suite-skill-maintenance-plan",
                "could not seal suite Skill maintenance plan",
                error,
            ));
            return RuntimeApplyResult::partial(report, request.target, plan_text, Some(migration));
        }
    };
    plan_text.push_str(&format!(
        "\nMaintenance plan: {} (required acknowledgements: {})",
        maintenance_plan.plan_hash,
        maintenance_plan.required_acknowledgements.len()
    ));
    let mut maintenance_receipts = Vec::new();

    match service.apply(
        &maintenance_plan.plan_hash,
        &maintenance_plan.required_acknowledgements,
    ) {
        Ok(receipt) if receipt.status == crate::maintenance::MaintenanceStatus::Verified => {
            report.add(crate::setup::SetupFinding::pass(
                "runtime-setup-maintenance-closure",
                format!(
                    "applied and verified runtime files, retired-entry cleanup, suite Skills, snapshots and routes ({})",
                    receipt.plan_hash
                ),
            ));
            maintenance_receipts.push(receipt);
        }
        Ok(receipt) => {
            report.add(crate::setup::SetupFinding::fail(
                "runtime-setup-maintenance-closure",
                "runtime setup failed verification and was not left active",
                receipt
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("maintenance status: {:?}", receipt.status)),
            ));
            maintenance_receipts.push(receipt);
        }
        Err(error) => report.add(crate::setup::SetupFinding::fail(
            "runtime-setup-maintenance-closure",
            "failed to execute the runtime setup transaction",
            error,
        )),
    }
    RuntimeApplyResult {
        report,
        target: request.target.to_path_buf(),
        plan_text,
        migration: Some(migration),
        maintenance_plan: Some(maintenance_plan),
        maintenance_receipts,
    }
}

pub fn runtime_install_health_report(
    target: &Path,
    home: &Path,
    run_mcp_smoke: bool,
) -> crate::setup::SetupReport {
    verify::runtime_install_health_report(target, home, run_mcp_smoke)
}

fn claude_ags_command_path(home: &Path) -> PathBuf {
    home.join(".claude").join("commands").join("ags.md")
}

fn codex_shared_ags_named_skill_dir(home: &Path, name: &str) -> PathBuf {
    home.join(".agents").join("skills").join(name)
}

fn codex_native_ags_named_skill_dir(home: &Path, name: &str) -> PathBuf {
    home.join(".codex").join("skills").join(name)
}

fn codex_ags_named_skill_path(home: &Path, name: &str) -> PathBuf {
    codex_shared_ags_named_skill_dir(home, name).join("SKILL.md")
}

fn codex_ags_named_skill_agent_metadata_path(home: &Path, name: &str) -> PathBuf {
    codex_shared_ags_named_skill_dir(home, name)
        .join("agents")
        .join("openai.yaml")
}

fn retired_codex_ags_skill_dirs(home: &Path) -> Vec<PathBuf> {
    ["ags", "ags-preflight", "ags-verify", "ags-capability"]
        .into_iter()
        .flat_map(|name| {
            [
                codex_shared_ags_named_skill_dir(home, name),
                codex_native_ags_named_skill_dir(home, name),
            ]
        })
        .collect()
}

fn retired_ags_memory_script_paths(home: &Path) -> Vec<PathBuf> {
    [
        "context-memory-start.py",
        "claude-stop-memory-capture.py",
        "raw-tool-call-stop-guard.js",
        "context-memory.sh",
        "stop-archive-hook.sh",
    ]
    .into_iter()
    .map(|name| home.join(".agents/scripts").join(name))
    .collect()
}

fn sanitize_name(path: &str) -> String {
    path.trim_matches('/')
        .replace(['/', '\\', '.'], "-")
        .trim_matches('-')
        .to_string()
}

fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn project_protocol_files() -> &'static [&'static str] {
    &[
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
        "task-routing.md",
        "project-profile.md",
        "context-memory.md",
        "cursor-skill-index.md",
    ]
}

#[cfg(test)]
mod runtime_transaction_tests {
    use super::*;

    #[test]
    fn setup_without_a_host_is_rejected_before_runtime_mutation() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let result = apply_runtime(RuntimeApplyRequest {
            source_root: &source,
            target: &runtime,
            home: &home,
            force: false,
            approved_lifecycle_hosts: Some(&[]),
            suite_skill_authority_root: Some(&source),
        });
        assert!(!result.report.passed());
        assert!(
            !runtime.exists(),
            "blocked setup must not create runtime state"
        );
        assert!(result.maintenance_plan.is_none());
    }

    #[test]
    fn setup_applies_files_and_required_skills_as_one_verified_transaction() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // Reproduce the retired architecture: a catalog Skill was presented as
        // suite-owned solely because a Host symlink targeted the authority
        // checkout. Setup must preserve the user's capability by converting it
        // into an InstalledSkillRecord before retiring that ownership model.
        #[cfg(unix)]
        {
            for skill_id in ["diagnosing-bugs", "code-review"] {
                let old_link = home.join(".codex/skills").join(skill_id);
                std::fs::create_dir_all(old_link.parent().unwrap()).unwrap();
                std::os::unix::fs::symlink(source.join("global-skills").join(skill_id), &old_link)
                    .unwrap();
            }
        }
        let hosts = crate::suite_skill_projection::supported_suite_skill_hosts()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let result = apply_runtime(RuntimeApplyRequest {
            source_root: &source,
            target: &runtime,
            home: &home,
            force: false,
            approved_lifecycle_hosts: Some(&hosts),
            suite_skill_authority_root: Some(&source),
        });
        assert!(result.report.passed(), "{:#?}", result.report.findings);
        assert!(runtime.join("install-manifest.json").is_file());
        for host in hosts {
            assert!(ags_capability_governance::snapshot_path(&runtime, &host).is_file());
        }
        #[cfg(unix)]
        {
            let installed =
                ags_capability_governance::skill_adoption::load_installed_skills(&runtime).unwrap();
            for skill_id in ["diagnosing-bugs", "code-review"] {
                let record = installed.skills.get(skill_id).unwrap();
                assert!(record.source_spec.is_upstream_bound());
                assert_eq!(
                    record.catalog_review,
                    ags_capability_governance::skill_adoption::CatalogReviewStatus::Reviewed
                );
                let routes = ags_capability_governance::skill_adoption::verify_adoption_routes(
                    &runtime, &home, skill_id,
                )
                .unwrap();
                assert!(routes.verified_on_all_targets(), "{routes:#?}");
            }
        }
        assert!(result.plan_text.contains("Maintenance plan:"));
        assert!(result.maintenance_receipts.iter().any(|receipt| {
            receipt.phase == crate::maintenance::MaintenancePhase::Apply
                && receipt.status == crate::maintenance::MaintenanceStatus::Verified
        }));
        let plan_hash = &result.maintenance_plan.as_ref().unwrap().plan_hash;
        let recovery = crate::maintenance::recover_runtime_setup_plan(&runtime, plan_hash).unwrap();
        assert_eq!(
            recovery.status,
            crate::maintenance::MaintenanceStatus::Recovered
        );
        assert!(!runtime.join("install-manifest.json").exists());
        #[cfg(unix)]
        {
            let installed =
                ags_capability_governance::skill_adoption::load_installed_skills(&runtime).unwrap();
            for skill_id in ["diagnosing-bugs", "code-review"] {
                assert!(!installed.skills.contains_key(skill_id));
                assert_eq!(
                    std::fs::read_link(home.join(".codex/skills").join(skill_id)).unwrap(),
                    source.join("global-skills").join(skill_id)
                );
            }
        }
    }

    #[test]
    fn setup_with_one_host_mutates_and_verifies_only_that_host() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let hosts = vec!["codex".to_string()];

        let result = apply_runtime(RuntimeApplyRequest {
            source_root: &source,
            target: &runtime,
            home: &home,
            force: false,
            approved_lifecycle_hosts: Some(&hosts),
            suite_skill_authority_root: Some(&source),
        });
        assert!(result.report.passed(), "{:#?}", result.report.findings);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(runtime.join("install-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["lifecycle"]["approved_hosts"],
            serde_json::json!(["codex"])
        );
        assert_eq!(
            manifest["suite_skill_projection"]["hosts"],
            serde_json::json!(["codex"])
        );
        assert!(manifest["host_commands"].get("codex").is_some());
        assert!(manifest["host_commands"].get("claude_code").is_none());
        assert!(ags_capability_governance::snapshot_path(&runtime, "codex").is_file());
        assert!(!ags_capability_governance::snapshot_path(&runtime, "claude-code").exists());
        assert!(!claude_ags_command_path(&home).exists());
        let plan = result.maintenance_plan.unwrap();
        let change = match plan.payload.unwrap() {
            crate::maintenance::MaintenancePayload::RuntimeSetup(change) => change,
            payload => panic!("unexpected payload: {payload:?}"),
        };
        assert_eq!(change.suite_skills.hosts, hosts);
        assert!(change
            .suite_skills
            .projected_links
            .keys()
            .all(|key| key.starts_with("codex/")));
    }

    #[test]
    fn setup_deselects_host_links_commands_and_sealed_snapshot_transactionally() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let initial_hosts = vec!["claude-code".to_string(), "codex".to_string()];
        let initial = apply_runtime(RuntimeApplyRequest {
            source_root: &source,
            target: &runtime,
            home: &home,
            force: false,
            approved_lifecycle_hosts: Some(&initial_hosts),
            suite_skill_authority_root: Some(&source),
        });
        assert!(initial.report.passed(), "{:#?}", initial.report.findings);
        assert!(claude_ags_command_path(&home).is_file());
        assert!(ags_capability_governance::snapshot_path(&runtime, "claude-code").is_file());

        let selected_hosts = vec!["codex".to_string()];
        let updated = apply_runtime(RuntimeApplyRequest {
            source_root: &source,
            target: &runtime,
            home: &home,
            force: true,
            approved_lifecycle_hosts: Some(&selected_hosts),
            suite_skill_authority_root: Some(&source),
        });
        assert!(updated.report.passed(), "{:#?}", updated.report.findings);
        assert!(!claude_ags_command_path(&home).exists());
        assert!(!ags_capability_governance::snapshot_path(&runtime, "claude-code").exists());
        let claude_skill_root = ags_host_integration::platform_spec("claude-code")
            .unwrap()
            .native_skill_subdir
            .unwrap();
        assert!(!home.join(claude_skill_root).join("ags-skill").exists());
        assert!(ags_capability_governance::snapshot_path(&runtime, "codex").is_file());
        let plan = updated.maintenance_plan.unwrap();
        let change = match plan.payload.unwrap() {
            crate::maintenance::MaintenancePayload::RuntimeSetup(change) => change,
            payload => panic!("unexpected payload: {payload:?}"),
        };
        assert_eq!(change.suite_skills.hosts, selected_hosts);
        assert_eq!(change.suite_skills.deactivated_hosts, vec!["claude-code"]);
    }
}
