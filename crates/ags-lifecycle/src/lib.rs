//! Deterministic public onboarding assessment for AGS.
//!
//! This module owns the `assess -> plan -> apply -> verify` vocabulary. Its
//! assessment and planning never launch a process or write files. The separate
//! [`execute_action`] entry accepts only a closed [`OnboardingAction`] returned
//! by a plan, after the caller has enforced explicit confirmation or an MCP
//! DecisionLease.

pub mod conformance;
pub mod host_memory;
pub mod init;
pub mod lifecycle_projection;
pub mod maintenance;
pub mod setup;
pub mod suite_skill_projection;
pub mod workspace_lifecycle;

mod onboarding;

pub use onboarding::*;
