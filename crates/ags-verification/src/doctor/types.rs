//! Core types for suite diagnostics.
//!
//! These types define a shared vocabulary for health checks, findings,
//! and aggregated reports — reusable by suite-doctor, bootstrap-dry-run,
//! and any future diagnostic CLI.
//!
//! The `Severity` enum uses the same canonical values as
//! `workflow_sync_check::Severity` for cross-crate consistency.

use serde::{Deserialize, Serialize};
use std::fmt;

// ── Severity ────────────────────────────────────────────────────────────

/// Severity of a diagnostic finding.
///
/// Uses the same canonical serde values as `workflow_sync_check::Severity`
/// (`"fail"`, `"warn"`, `"info"`) for cross-crate consistency.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Severity {
    /// Informational — non-blocking observation.
    #[serde(rename = "info")]
    Info,
    /// Warning — unexpected but non-blocking.
    #[serde(rename = "warn")]
    Warn,
    /// Fail — blocking issue requiring attention.
    #[serde(rename = "fail")]
    Fail,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warn => write!(f, "WARN"),
            Severity::Fail => write!(f, "FAIL"),
        }
    }
}

// ── Check status ────────────────────────────────────────────────────────

/// Result of a single diagnostic check.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CheckStatus {
    /// Check completed with no issues.
    #[serde(rename = "pass")]
    Pass,
    /// Check found a blocking issue.
    #[serde(rename = "fail")]
    Fail,
    /// Check found a non-blocking issue.
    #[serde(rename = "warn")]
    Warn,
    /// Check was skipped (e.g. optional check with missing prerequisites).
    #[serde(rename = "skip")]
    Skip,
}

impl fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckStatus::Pass => write!(f, "PASS"),
            CheckStatus::Fail => write!(f, "FAIL"),
            CheckStatus::Warn => write!(f, "WARN"),
            CheckStatus::Skip => write!(f, "SKIP"),
        }
    }
}

impl CheckStatus {
    /// Whether this status counts as a successful check.
    pub fn is_ok(&self) -> bool {
        matches!(self, CheckStatus::Pass | CheckStatus::Skip)
    }
}

// ── Finding ─────────────────────────────────────────────────────────────

/// A single diagnostic finding produced by a check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Name of the check that produced this finding.
    pub check_name: String,
    /// Result status for this check.
    pub status: CheckStatus,
    /// Severity level (used for report aggregation).
    pub severity: Severity,
    /// Short human-readable summary.
    pub message: String,
    /// Optional detail or remediation guidance.
    pub detail: Option<String>,
}

impl Finding {
    /// Create a new pass finding.
    pub fn pass(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: CheckStatus::Pass,
            severity: Severity::Info,
            message: message.into(),
            detail: None,
        }
    }

    /// Create a new fail finding.
    pub fn fail(
        check_name: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            check_name: check_name.into(),
            status: CheckStatus::Fail,
            severity: Severity::Fail,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    /// Create a new warn finding.
    pub fn warn(
        check_name: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            check_name: check_name.into(),
            status: CheckStatus::Warn,
            severity: Severity::Warn,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    /// Create a new info finding.
    pub fn info(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: CheckStatus::Pass,
            severity: Severity::Info,
            message: message.into(),
            detail: None,
        }
    }

    /// Create a skipped check entry.
    pub fn skip(check_name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: CheckStatus::Skip,
            severity: Severity::Info,
            message: reason.into(),
            detail: None,
        }
    }
}

// ── Health report ───────────────────────────────────────────────────────

/// Aggregated diagnostic report from one or more checks.
///
/// This is the primary output type for suite-doctor and similar diagnostics.
/// It provides text/JSON rendering, exit-code calculation, and counter
/// methods compatible with the drift checker's reporting conventions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HealthReport {
    /// Human-readable label for this diagnostic run.
    pub title: String,
    /// All findings from all checks.
    pub findings: Vec<Finding>,
}

impl HealthReport {
    /// Create a new empty report.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            findings: Vec::new(),
        }
    }

    /// Add a finding to the report.
    pub fn add(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    /// Whether the report contains no blocking failures.
    ///
    /// Matches the drift checker convention: only `Severity::Fail` blocks
    /// the gate.  Warnings and infos are advisory.
    pub fn passed(&self) -> bool {
        !self.findings.iter().any(|f| f.severity == Severity::Fail)
    }

    /// Recommended process exit code.
    ///
    /// `0` when `passed()` is true, `1` otherwise.
    pub fn exit_code(&self) -> i32 {
        if self.passed() {
            0
        } else {
            1
        }
    }

    /// Count of fail-severity findings.
    pub fn total_failures(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Fail)
            .count()
    }

    /// Count of warn-severity findings.
    pub fn total_warnings(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .count()
    }

    /// Count of info-severity findings.
    pub fn total_infos(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count()
    }

    /// Count of skipped checks.
    pub fn total_skipped(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.status == CheckStatus::Skip)
            .count()
    }

    /// Count of pass-status checks.
    pub fn total_passed_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.status == CheckStatus::Pass)
            .count()
    }

    /// Count of fail-status checks.
    pub fn total_failed_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.status == CheckStatus::Fail)
            .count()
    }

    /// Count of warn-status checks.
    pub fn total_warned_checks(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.status == CheckStatus::Warn)
            .count()
    }

    /// Total number of findings.
    pub fn total(&self) -> usize {
        self.findings.len()
    }
}
