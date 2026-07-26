//! Deterministic public onboarding assessment for AGS.
//!
//! This module owns the `assess -> plan -> apply -> verify` vocabulary. It is
//! assessment and planning never launch a process or write files. The separate
//! [`execute_action`] entry accepts only a closed [`OnboardingAction`] returned
//! by a plan, after the caller has enforced explicit confirmation or an MCP
//! DecisionLease.

pub mod init;
pub mod setup;
pub mod update;

mod onboarding;

pub use onboarding::*;
