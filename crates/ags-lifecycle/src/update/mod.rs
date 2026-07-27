//! Unified update lifecycle.
//!
//! The CLI resolves paths and authorization flags; lifecycle owns the mutation
//! sequence and returns evidence for the CLI to render.

pub mod apply;
pub mod lanes;
pub mod plan;

pub use lanes::{CapabilityInventory, ProjectInventory, ProjectUpdate, UpdateLane, UpdateLanePlan};
