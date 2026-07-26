//! Core types for structured drift checking.
//!
//! These types replace the old flat `Finding` / `SyncReport` model with a
//! multi-project, section-level, severity-graded drift model designed to
//! distinguish legal differences from dangerous drift.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

// ── Error codes ────────────────────────────────────────────────────────

pub mod error_code {
    // Structural
    pub const SOURCE_ROOT_MISSING: &str = "DRIFT_SOURCE_ROOT_MISSING";
    pub const TARGET_ROOT_MISSING: &str = "DRIFT_TARGET_ROOT_MISSING";
    pub const FILE_MISSING_IN_TARGET: &str = "DRIFT_FILE_MISSING_IN_TARGET";
    pub const FILE_MISSING_IN_SOURCE: &str = "DRIFT_FILE_MISSING_IN_SOURCE";
    pub const FILE_READ_FAILED: &str = "DRIFT_FILE_READ_FAILED";

    // Section-level drift
    pub const SECTION_MISSING: &str = "DRIFT_SECTION_MISSING";
    pub const EXTRA_SECTION: &str = "DRIFT_EXTRA_SECTION";
    pub const CONTENT_DRIFT: &str = "DRIFT_CONTENT_DRIFT";
    pub const STRUCTURE_DRIFT: &str = "DRIFT_STRUCTURE_DRIFT";

    // Meta
    pub const EXTRA_PROTOCOL_FILE: &str = "DRIFT_EXTRA_PROTOCOL_FILE";
    pub const LEGAL_REDACTION: &str = "DRIFT_LEGAL_REDACTION";

    // Allowlist
    pub const ALLOWLIST_LOAD_FAILED: &str = "DRIFT_ALLOWLIST_LOAD_FAILED";

    // Protocol safety assertions
    pub const INVARIANT_MISSING: &str = "INVARIANT_MISSING";
    pub const INVARIANT_CONTRADICTED: &str = "INVARIANT_CONTRADICTED";

    // Public release boundary
    pub const PUBLIC_FORBIDDEN_PAYLOAD: &str = "PUBLIC_FORBIDDEN_PAYLOAD";
}

// ── Project identity ───────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProjectKind {
    /// The development source of truth (private suite).
    Private,
    /// The stable runtime baseline.
    Stable,
    /// Public-full sanitized distribution with legal redactions permitted.
    PublicCoreOnly,
    /// User-specified custom target.
    Custom(String),
}

impl fmt::Display for ProjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectKind::Private => write!(f, "private"),
            ProjectKind::Stable => write!(f, "stable"),
            ProjectKind::PublicCoreOnly => write!(f, "public-full-sanitized"),
            ProjectKind::Custom(label) => write!(f, "custom:{label}"),
        }
    }
}

// ── Drift severity ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum Severity {
    /// Informational — legal redaction or allowlisted difference.
    #[serde(rename = "info")]
    Info,
    /// Warning — unexpected but non-blocking difference.
    #[serde(rename = "warn")]
    Warn,
    /// Fail — dangerous drift requiring attention.
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

// ── Drift type ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DriftKind {
    /// A required file is missing from the target.
    #[serde(rename = "file_missing_in_target")]
    FileMissingInTarget,
    /// A file exists in the target but not in the source.
    #[serde(rename = "file_missing_in_source")]
    FileMissingInSource,
    /// A section (heading path) is present in source but missing in target.
    #[serde(rename = "section_missing")]
    SectionMissing,
    /// A section exists in target but not in source.
    #[serde(rename = "extra_section")]
    ExtraSection,
    /// Section content differs between source and target.
    #[serde(rename = "content_drift")]
    ContentDrift,
    /// Structural difference (heading order, level mismatch).
    #[serde(rename = "structure_drift")]
    StructureDrift,
    /// A legal redaction — content adjusted per public-full sanitized allowlist.
    #[serde(rename = "legal_redaction")]
    LegalRedaction,
    /// A file in protocol/ that is not in the sync manifest.
    #[serde(rename = "extra_protocol_file")]
    ExtraProtocolFile,
    /// Allowlist file failed to load (missing, unreadable, or invalid JSON).
    #[serde(rename = "allowlist_load_failed")]
    AllowlistLoadFailed,
    /// A required protocol safety assertion is absent from the target.
    #[serde(rename = "invariant_missing")]
    InvariantMissing,
    /// A protocol safety assertion is present but has been contradictorily rewritten.
    #[serde(rename = "invariant_contradicted")]
    InvariantContradicted,
    /// A forbidden private runtime/build artifact is present in a public-full target.
    #[serde(rename = "public_forbidden_payload")]
    PublicForbiddenPayload,
}

impl fmt::Display for DriftKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriftKind::FileMissingInTarget => write!(f, "file_missing_in_target"),
            DriftKind::FileMissingInSource => write!(f, "file_missing_in_source"),
            DriftKind::SectionMissing => write!(f, "section_missing"),
            DriftKind::ExtraSection => write!(f, "extra_section"),
            DriftKind::ContentDrift => write!(f, "content_drift"),
            DriftKind::StructureDrift => write!(f, "structure_drift"),
            DriftKind::LegalRedaction => write!(f, "legal_redaction"),
            DriftKind::ExtraProtocolFile => write!(f, "extra_protocol_file"),
            DriftKind::AllowlistLoadFailed => write!(f, "allowlist_load_failed"),
            DriftKind::InvariantMissing => write!(f, "invariant_missing"),
            DriftKind::InvariantContradicted => write!(f, "invariant_contradicted"),
            DriftKind::PublicForbiddenPayload => write!(f, "public_forbidden_payload"),
        }
    }
}

// ── Section path ───────────────────────────────────────────────────────

/// A hierarchical heading path, e.g. `["Runtime Adapters", "Generic Fields", "Executor"]`.
pub type SectionPath = Vec<String>;

/// Format a section path as a breadcrumb string.
pub fn format_section_path(path: &[String]) -> String {
    path.join(" > ")
}

// ── Drift finding ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Drift {
    /// Error code from `error_code` module.
    pub code: String,
    /// The kind of drift detected.
    pub kind: DriftKind,
    /// How severe this drift is.
    pub severity: Severity,
    /// Relative file path (e.g. `protocol/runtime-adapters.md`).
    pub file: String,
    /// Section path within the file (empty for file-level drift).
    pub section_path: SectionPath,
    /// Human-readable description.
    pub message: String,
    /// Recommended action.
    pub suggested_action: String,
}

impl Drift {
    pub fn new(
        code: &str,
        kind: DriftKind,
        severity: Severity,
        file: &str,
        section_path: SectionPath,
        message: impl Into<String>,
        suggested_action: &str,
    ) -> Self {
        Self {
            code: code.to_string(),
            kind,
            severity,
            file: file.to_string(),
            section_path,
            message: message.into(),
            suggested_action: suggested_action.to_string(),
        }
    }
}

// ── Project-level result ───────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectDrift {
    pub project_name: String,
    pub project_kind: ProjectKind,
    pub project_root: PathBuf,
    pub drifts: Vec<Drift>,
}

impl ProjectDrift {
    pub fn passed(&self) -> bool {
        !self.drifts.iter().any(|d| d.severity == Severity::Fail)
    }

    pub fn failures(&self) -> usize {
        self.drifts
            .iter()
            .filter(|d| d.severity == Severity::Fail)
            .count()
    }

    pub fn warnings(&self) -> usize {
        self.drifts
            .iter()
            .filter(|d| d.severity == Severity::Warn)
            .count()
    }

    pub fn infos(&self) -> usize {
        self.drifts
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .count()
    }
}

// ── Overall report ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub source_root: PathBuf,
    pub source_name: String,
    pub projects: Vec<ProjectDrift>,
}

impl DriftReport {
    pub fn passed(&self) -> bool {
        self.projects.iter().all(|p| p.passed())
    }

    pub fn total_failures(&self) -> usize {
        self.projects.iter().map(|p| p.failures()).sum()
    }

    pub fn total_warnings(&self) -> usize {
        self.projects.iter().map(|p| p.warnings()).sum()
    }

    pub fn total_infos(&self) -> usize {
        self.projects.iter().map(|p| p.infos()).sum()
    }

    pub fn total_drifts(&self) -> usize {
        self.projects.iter().map(|p| p.drifts.len()).sum()
    }
}

// ── Check options ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetConfig {
    pub root: PathBuf,
    pub name: String,
    pub kind: ProjectKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckOptions {
    pub source_root: PathBuf,
    pub source_name: String,
    pub targets: Vec<TargetConfig>,
    /// Optional path to a JSON allowlist file.
    pub allowlist_path: Option<PathBuf>,
}

impl CheckOptions {
    /// Single-target convenience constructor (backward compatible).
    pub fn single(
        source: impl Into<PathBuf>,
        target: impl Into<PathBuf>,
        target_name: impl Into<String>,
    ) -> Self {
        let target_name = target_name.into();
        Self {
            source_root: source.into(),
            source_name: "private".to_string(),
            targets: vec![TargetConfig {
                root: target.into(),
                name: target_name.clone(),
                kind: if target_name == "stable" {
                    ProjectKind::Stable
                } else if matches!(
                    target_name.as_str(),
                    "public"
                        | "public-core"
                        | "public-core-only"
                        | "public-full"
                        | "public-full-sanitized"
                ) {
                    ProjectKind::PublicCoreOnly
                } else {
                    ProjectKind::Custom(target_name)
                },
            }],
            allowlist_path: None,
        }
    }
}
