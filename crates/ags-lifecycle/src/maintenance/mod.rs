//! Shared maintenance transaction kernel used by CLI and MCP adapters.
//!
//! Network acquisition and subject-specific mutations live behind
//! [`MaintenanceBackend`]. This module owns the invariant shared by every
//! maintenance surface: immutable plan, binding/expiry checks, explicit risk
//! acknowledgement, durable receipt, and recoverable backend execution.

mod migration;
mod model;
mod notice;
mod router;
mod runtime_setup;
mod service;
mod skill;
mod store;
mod suite;

pub use migration::*;
pub use model::*;
pub use notice::*;
pub use router::MaintenanceBackendRouter;
pub use runtime_setup::{
    path_state_hash, recover_incomplete_runtime_setups, recover_runtime_setup_plan,
    RuntimeSetupMaintenanceBackend,
};
pub use service::{MaintenanceBackend, MaintenanceService, ServiceClock, ServiceContext};
pub use skill::{maintenance_source_from_spec, SkillMaintenanceBackend};
pub use suite::SuiteSkillMaintenanceBackend;
