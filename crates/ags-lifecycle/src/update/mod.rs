//! Unified update lifecycle.
//!
//! The CLI is an adapter at this seam: it supplies resolved paths and the
//! host-specific write effects, then renders the returned outcomes.

pub mod apply;
pub mod lanes;
pub mod notifier;
pub mod plan;
pub mod repair;
pub mod rollback;

pub use lanes::{CapabilityInventory, ProjectInventory, ProjectUpdate, UpdateLane, UpdateLanePlan};
