//! Data model for project initialization.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

pub(crate) const AGS_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const PROJECT_INIT_SCHEMA: &str = "0.4.1-project-init";
#[derive(Debug, Clone)]
pub struct InitFile {
    pub path: PathBuf,
    pub description: String,
    pub content: String,
    pub mode: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InitSeverity {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "fail")]
    Fail,
}

impl fmt::Display for InitSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(formatter, "INFO"),
            Self::Warn => write!(formatter, "WARN"),
            Self::Fail => write!(formatter, "FAIL"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InitCheckStatus {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "warn")]
    Warn,
}

impl fmt::Display for InitCheckStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => write!(formatter, "PASS"),
            Self::Fail => write!(formatter, "FAIL"),
            Self::Warn => write!(formatter, "WARN"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InitFinding {
    pub check_name: String,
    pub status: InitCheckStatus,
    pub severity: InitSeverity,
    pub message: String,
    pub detail: Option<String>,
}

impl InitFinding {
    pub fn pass(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: InitCheckStatus::Pass,
            severity: InitSeverity::Info,
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
            status: InitCheckStatus::Fail,
            severity: InitSeverity::Fail,
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
            status: InitCheckStatus::Warn,
            severity: InitSeverity::Warn,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    pub fn info(check_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            check_name: check_name.into(),
            status: InitCheckStatus::Pass,
            severity: InitSeverity::Info,
            message: message.into(),
            detail: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InitReport {
    pub title: String,
    pub findings: Vec<InitFinding>,
}

impl InitReport {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            findings: Vec::new(),
        }
    }

    pub fn add(&mut self, finding: InitFinding) {
        self.findings.push(finding);
    }

    pub fn passed(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|finding| finding.severity == InitSeverity::Fail)
    }
}
