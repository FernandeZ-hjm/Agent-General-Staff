// ── Core types ──────────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Verification scope — determines which checks are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Local-only checks: fmt, test, build, fixtures, YAML, preflight.
    Local,
    /// Local + drift checks against stable and public targets.
    Full,
    /// Self-contained checks for a public release source tree.
    Release,
    /// Private/stable source to explicit public target promotion checks.
    Promotion,
}

impl Scope {
    #[allow(clippy::should_implement_trait)] // inherent parser with domain String error; intentionally not std::str::FromStr
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "local" => Ok(Scope::Local),
            "full" => Ok(Scope::Full),
            "release" => Ok(Scope::Release),
            "promotion" => Ok(Scope::Promotion),
            other => Err(format!(
                "invalid scope: '{}'. Expected one of: local, full, release, promotion",
                other
            )),
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::Local => write!(f, "local"),
            Scope::Full => write!(f, "full"),
            Scope::Release => write!(f, "release"),
            Scope::Promotion => write!(f, "promotion"),
        }
    }
}

/// Explicit inputs for verification scopes that cross repository boundaries.
#[derive(Debug, Clone, Default)]
pub struct VerificationOptions {
    /// Explicit public worktree used by `Scope::Promotion` and, when supplied,
    /// as the self-contained release source for `Scope::Release`.
    pub public_root: Option<PathBuf>,
}

/// Check status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Pass => write!(f, "pass"),
            CheckStatus::Fail => write!(f, "fail"),
            CheckStatus::Skip => write!(f, "skip"),
        }
    }
}

/// Check severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warn => write!(f, "warn"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// A single verification check item — the stable unit of verification evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    /// Stable identifier for this check (e.g. "cargo-fmt", "fixture-valid-full").
    pub id: String,
    /// Which scope(s) this check belongs to.
    pub scope: String,
    /// Pass / fail / skip.
    pub status: CheckStatus,
    /// Info / warn / error.
    pub severity: Severity,
    /// Human-readable evidence summary (command output, parsed result).
    pub evidence: String,
    /// Suggested remediation if the check failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    /// The command that was executed (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Exit code of the executed command (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl CheckItem {
    pub fn pass(id: &str, scope: &str, evidence: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Pass,
            severity: Severity::Info,
            evidence: evidence.to_string(),
            remediation: None,
            command: None,
            exit_code: Some(0),
        }
    }

    pub fn fail(id: &str, scope: &str, evidence: &str, remediation: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Fail,
            severity: Severity::Error,
            evidence: evidence.to_string(),
            remediation: Some(remediation.to_string()),
            command: None,
            exit_code: Some(1),
        }
    }

    pub fn skip(id: &str, scope: &str, reason: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Skip,
            severity: Severity::Info,
            evidence: reason.to_string(),
            remediation: None,
            command: None,
            exit_code: None,
        }
    }

    pub fn warn(id: &str, scope: &str, evidence: &str, remediation: &str) -> Self {
        CheckItem {
            id: id.to_string(),
            scope: scope.to_string(),
            status: CheckStatus::Fail,
            severity: Severity::Warn,
            evidence: evidence.to_string(),
            remediation: Some(remediation.to_string()),
            command: None,
            exit_code: Some(0),
        }
    }

    pub(crate) fn with_command(mut self, cmd: &str) -> Self {
        self.command = Some(cmd.to_string());
        self
    }

    pub(crate) fn with_exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }
}

/// Aggregated verification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: String,
    pub scope: Scope,
    pub repo_root: String,
    pub items: Vec<CheckItem>,
    pub summary: VerificationSummary,
}

/// Summary statistics for a verification report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl VerificationReport {
    /// Whether all blocking checks passed. Advisory WARN items do not fail the report.
    pub fn passed(&self) -> bool {
        let required_scope_skipped =
            matches!(self.scope, Scope::Release | Scope::Promotion) && self.summary.skipped > 0;
        self.summary.errors == 0 && !required_scope_skipped
    }

    /// Exit code: 0 if all blocking checks passed, 1 if any ERROR failed.
    pub fn exit_code(&self) -> i32 {
        if self.passed() {
            0
        } else {
            1
        }
    }
}
