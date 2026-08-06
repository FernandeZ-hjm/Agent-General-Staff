//! Shared maintenance transaction kernel used by CLI and MCP adapters.
//!
//! Network acquisition and subject-specific mutations live behind
//! [`MaintenanceBackend`]. This module owns the invariant shared by every
//! maintenance surface: immutable plan, binding/expiry checks, explicit risk
//! acknowledgement, durable receipt, and recoverable backend execution.

mod activation;
mod migration;
mod model;
mod notice;
mod router;
mod runtime_setup;
mod service;
mod skill;
mod store;
mod suite;

pub use activation::*;
pub use migration::*;
pub use model::*;
pub use notice::*;
pub use router::MaintenanceBackendRouter;
pub use runtime_setup::{
    path_state_hash, recover_incomplete_runtime_setups,
    recover_incomplete_runtime_setups_with_activation, recover_runtime_setup_plan,
    recover_runtime_setup_plan_with_activation, RuntimeSetupMaintenanceBackend,
};
pub use service::{MaintenanceBackend, MaintenanceService, ServiceClock, ServiceContext};
pub use skill::{maintenance_source_from_spec, SkillMaintenanceBackend};
pub use suite::SuiteSkillMaintenanceBackend;

/// Content identity safe for use as a recovery filename on every supported OS.
///
/// Canonical evidence hashes retain the `sha256:` scheme prefix. Recovery
/// filenames deliberately use the bare hexadecimal encoding because `:` is an
/// invalid filename character on Windows.
pub(crate) fn recovery_file_identity(transaction_id: &str) -> String {
    ags_platform::sha256_hex(transaction_id.as_bytes())
}
