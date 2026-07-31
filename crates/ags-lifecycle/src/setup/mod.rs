//! Private runtime setup lifecycle.
//!
//! This module is the single authority for setup planning, mutation,
//! verification, generated templates, and host-memory
//! mutation. Human-facing adapters resolve CLI arguments, render the returned
//! presentations, and own process exit behaviour.

mod apply;
mod global_entry;
mod memory;
mod plan;
mod recommendations;
mod templates;
mod verify;

use std::fmt;
use std::path::{Path, PathBuf};

use ags_host_integration::{claude_mcp_list_line_at, command_in_path};
use serde::{Deserialize, Serialize};

use apply::{add_claude_registration_checks, write_install_file};
use global_entry::{
    global_entry_protocol_json, global_entry_protocol_plan, render_global_entry_protocol_text,
    write_ags_global_entry,
};
use plan::{
    cleanup_install_dir, private_install_plan, private_install_plan_with_hosts,
    render_private_plan_json, render_private_plan_text,
};
use recommendations::{render_third_party_recommendations_text, third_party_recommendations_json};

pub use memory::{apply_host_memory_adapter, lifecycle_migration_preview};

pub const PRIVATE_INSTALL_SCHEMA: &str = "0.4.1-private-install";
const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");

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

#[derive(Debug, Clone)]
struct InstallFile {
    path: PathBuf,
    description: String,
    content: String,
    mode: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SetupHostEntry {
    pub id: String,
    pub display: String,
    pub config_subdirs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PrivatePlanPresentation {
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
pub fn private_plan_presentation(
    source_root: &Path,
    target: &Path,
    home: &Path,
    host_entries: &[SetupHostEntry],
    _include_optional_extensions: bool,
) -> PrivatePlanPresentation {
    let plan = private_install_plan(source_root, target, home);
    let entries = global_entry_protocol_plan(&plan, host_entries);
    PrivatePlanPresentation {
        install_json: serde_json::from_str(&render_private_plan_json(&plan))
            .unwrap_or_else(|_| serde_json::json!({})),
        install_text: render_private_plan_text(&plan),
        global_entry_json: global_entry_protocol_json(&entries),
        global_entry_text: render_global_entry_protocol_text(&entries),
        recommendations_json: third_party_recommendations_json(source_root, home),
        recommendations_text: render_third_party_recommendations_text(source_root, home),
    }
}

#[derive(Debug)]
pub struct PrivateApplyResult {
    pub report: crate::setup::SetupReport,
    pub target: PathBuf,
    pub plan_text: String,
}

#[derive(Debug, Clone)]
pub struct PrivateApplyRequest<'a> {
    pub source_root: &'a Path,
    pub target: &'a Path,
    pub home: &'a Path,
    pub force: bool,
    pub include_optional_extensions: bool,
    pub register_claude: bool,
    pub approved_lifecycle_hosts: Option<&'a [String]>,
}

/// Apply one already-authorized private-runtime setup transaction.
///
/// The caller owns target protection and confirmation. This function owns the
/// mutation sequence and returns evidence instead of rendering or exiting.
pub fn apply_private(request: PrivateApplyRequest<'_>) -> PrivateApplyResult {
    let plan = match request.approved_lifecycle_hosts {
        Some(hosts) => private_install_plan_with_hosts(
            request.source_root,
            request.target,
            request.home,
            hosts,
            "setup",
        ),
        None => private_install_plan(request.source_root, request.target, request.home),
    };
    let plan_text = render_private_plan_text(&plan);
    let mut report = crate::setup::SetupReport::new("private-install-apply");

    for file in &plan.files {
        report.add(write_install_file(file, request.force));
    }
    for path in &plan.cleanup_paths {
        report.add(cleanup_install_dir(path, request.force));
    }
    if request.register_claude {
        add_claude_registration_checks(&mut report, request.target);
        memory::add_workspace_memory_capture(&mut report, request.home, request.source_root);
    }
    report.add(write_ags_global_entry(request.target));
    if report.passed() {
        for host in ["codex", "claude-code", "omp", "codebuddy-code", "cursor"] {
            match refresh_skill_snapshot(request.source_root, request.target, host) {
                Ok(path) => report.add(crate::setup::SetupFinding::pass(
                    format!("skill-active-table-snapshot-{host}"),
                    format!("refreshed {}", path.display()),
                )),
                Err(error) => report.add(crate::setup::SetupFinding::fail(
                    format!("skill-active-table-snapshot-{host}"),
                    "failed to refresh ActiveSkillTable snapshot",
                    error,
                )),
            }
        }
    }

    PrivateApplyResult {
        report,
        target: request.target.to_path_buf(),
        plan_text,
    }
}

pub fn private_install_health_report(
    target: &Path,
    home: &Path,
    _include_optional_extensions: bool,
    run_mcp_smoke: bool,
) -> crate::setup::SetupReport {
    verify::private_install_health_report(target, home, run_mcp_smoke)
}

pub fn render_memory_capture_plan(
    home: &Path,
    source_root: &Path,
    register_claude: bool,
) -> String {
    memory::render_memory_capture_plan(home, source_root, register_claude)
}

fn refresh_skill_snapshot(
    authority_root: &Path,
    runtime_home: &Path,
    active_host: &str,
) -> Result<PathBuf, String> {
    let snapshot = ags_capability_governance::build_capability_snapshot_with_runtime_home(
        authority_root,
        active_host,
        runtime_home,
    )
    .map_err(|error| format!("skill snapshot build failed: {error:?}"))?;
    let path = ags_capability_governance::snapshot_path(runtime_home, active_host);
    let parent = path
        .parent()
        .ok_or_else(|| "skill snapshot path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("skill snapshot serialization failed: {error}"))?;
    ags_capability_governance::write_private_atomic(&path, (json + "\n").as_bytes())?;
    Ok(path)
}

fn claude_ags_command_path(home: &Path) -> PathBuf {
    home.join(".claude").join("commands").join("ags.md")
}

fn codex_ags_named_skill_dir(home: &Path, name: &str) -> PathBuf {
    home.join(".codex").join("skills").join(name)
}

fn codex_ags_named_skill_path(home: &Path, name: &str) -> PathBuf {
    codex_ags_named_skill_dir(home, name).join("SKILL.md")
}

fn codex_ags_named_skill_agent_metadata_path(home: &Path, name: &str) -> PathBuf {
    codex_ags_named_skill_dir(home, name)
        .join("agents")
        .join("openai.yaml")
}

fn retired_codex_ags_skill_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        codex_ags_named_skill_dir(home, "ags"),
        codex_ags_named_skill_dir(home, "ags-preflight"),
        codex_ags_named_skill_dir(home, "ags-verify"),
        codex_ags_named_skill_dir(home, "ags-capability"),
    ]
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
