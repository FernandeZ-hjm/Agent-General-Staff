//! AGS verification core.
//!
//! The crate root is a facade over one deep verification interface:
//! [`run_verify_with_options`] produces a structured [`VerificationReport`],
//! while [`render_text`] and [`render_json`] preserve the stable presentation
//! surface. Check implementation is private and split by change reason.

/// Suite health diagnostics.
pub mod doctor;
/// Contract v2 default-output and tool-schema budgets.
pub mod output_budget;
/// Exact-hash ownership decisions for lightweight project migration.
pub mod projection_migration;
/// Typed public capability projection and closed generated manifest set.
pub mod public_capability_projection;
pub mod public_source_projection;
/// Exact public release payload authority and verification.
pub mod release_manifest;
/// Canonical public/private release-package planning.
pub mod release_package;
/// Structured, shell-free project test execution and receipts.
pub mod test_execution;
/// Content-addressed verification evidence and reuse validation.
pub mod verification_bundle;

pub mod change_lane;
pub use change_lane::{
    classify_from_git_range, classify_lane, ChangeClassification, ChangeLane, VerificationProfile,
};

pub use ags_governance_decision::GovernanceStatus;

#[cfg(test)]
mod edition;
mod local_checks;
mod model;
#[cfg(test)]
mod mutation_guard;
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
pub use output_budget::{
    check_human_output_budget, check_json_output_budget, check_tool_schema_budget,
    DEFAULT_HUMAN_LINE_BUDGET, DEFAULT_JSON_BYTE_BUDGET, TOOL_SCHEMA_BYTE_BUDGET,
};
pub use projection_migration::{
    apply_project_projection, apply_projection_migration, plan_project_projection,
    plan_projection_migration, recover_projection_migration, MigrationDisposition,
    ProjectProjectionDisposition, ProjectProjectionFile, ProjectProjectionFileReceipt,
    ProjectProjectionMutation, ProjectProjectionPlan, ProjectProjectionReceipt, ProjectionConflict,
    ProjectionMigration, ProjectionMigrationReceipt,
};
pub use render::{render_json, render_text};
pub use test_execution::{
    load_project_test_profiles, local_execution_platform_support, run_host_project_test,
    run_project_test, run_read_only_command, CommandSpec, LocalExecutionPlatformSupport,
    ProjectTestProfiles, ReadOnlyCommandOutput, ReadOnlyCommandReceipt, TestExecutionError,
    TestExecutionErrorCode, TestExecutionStatus, TestProfile, TestReceipt,
};
pub use verification_bundle::{
    current_input_identity, validate_bundle_for_reuse, VerificationBundle,
    VerificationInputIdentity, TEST_POLICY_VERSION, VERIFICATION_BUNDLE_SCHEMA_VERSION,
};

// Private implementation imports preserve a single orchestration seam without
// exposing individual checks as a second public interface.
use local_checks::*;
use promotion::*;
use release::*;
use version::*;

#[cfg(test)]
mod tests;
