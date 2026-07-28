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

use ags_host_integration::{claude_mcp_list_line, command_in_path};
use serde::{Deserialize, Serialize};

use apply::{add_claude_registration_checks, write_install_file};
use global_entry::{
    global_entry_protocol_json, global_entry_protocol_plan, render_global_entry_protocol_text,
    write_ags_global_entry,
};
use plan::{
    cleanup_install_dir, private_install_plan, render_private_plan_json, render_private_plan_text,
};
use recommendations::{render_third_party_recommendations_text, third_party_recommendations_json};

pub use memory::{apply_host_memory_adapter, MergeOutcome};

pub const PRIVATE_INSTALL_SCHEMA: &str = "0.3.5-private-install";
const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");

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
}

impl SetupFinding {
    pub fn pass(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: SetupCheckStatus::Pass,
            severity: SetupSeverity::Info,
            message: message.into(),
            detail: None,
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
        }
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
}

/// Apply one already-authorized private-runtime setup transaction.
///
/// The caller owns target protection and confirmation. This function owns the
/// mutation sequence and returns evidence instead of rendering or exiting.
pub fn apply_private(request: PrivateApplyRequest<'_>) -> PrivateApplyResult {
    let plan = private_install_plan(request.source_root, request.target, request.home);
    let plan_text = render_private_plan_text(&plan);
    let mut report = crate::setup::SetupReport::new("private-install-apply");

    for file in &plan.files {
        report.add(write_install_file(file, request.force));
    }
    for dir in &plan.cleanup_dirs {
        report.add(cleanup_install_dir(dir, request.force));
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
) -> crate::setup::SetupReport {
    verify::private_install_health_report(target, home)
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

fn portable_validate_script() -> String {
    "#!/usr/bin/env bash\n# AGS portable task-card validator wrapper.\nset -euo pipefail\nexec ags task validate \"$@\"\n".to_string()
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
