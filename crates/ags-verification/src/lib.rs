//! AGS verification core.
//!
//! The crate root is a facade over one deep verification interface:
//! [`run_verify_with_options`] produces a structured [`VerificationReport`],
//! while [`render_text`] and [`render_json`] preserve the stable presentation
//! surface. Check implementation is private and split by change reason.

/// Bootstrap readiness checks and dry-run reporting.
pub mod bootstrap;
/// Suite health diagnostics.
pub mod doctor;
/// Canonical public/private release-package planning.
pub mod release_package;
/// Private/stable/public protocol and release drift verification.
pub mod sync;

pub mod change_lane;
pub use change_lane::{
    classify_from_git_range, classify_lane, ChangeClassification, ChangeLane, VerificationProfile,
};

pub mod visible_status;
#[allow(deprecated)]
pub use visible_status::{
    derive_governance_status, derive_visible_status, GovernanceStatus, StatusSignals, VisibleStatus,
};

mod local_checks;
mod model;
mod orchestrator;
mod promotion;
mod release;
mod render;
mod version;

pub use model::{
    CheckItem, CheckStatus, Scope, Severity, VerificationOptions, VerificationReport,
    VerificationSummary,
};
pub use orchestrator::{run_verify, run_verify_with_options};
pub use render::{render_json, render_text};

// Private implementation imports preserve a single orchestration seam without
// exposing individual checks as a second public interface.
use local_checks::*;
use promotion::*;
use release::*;
use version::*;

#[cfg(test)]
mod tests;
