//! Deterministic compilation and validation of AGS task contracts.
//!
//! The crate root is intentionally a facade. Wire models, project context,
//! compilation, and rendering have one implementation each in their named
//! modules; validator and runner retain their established public modules.

mod compile;
mod context;
mod fields;
mod intent;
mod render;

/// Gate-first launch-plan preparation. This module never executes a host.
pub mod runner;
/// Canonical task-card parser and validator.
pub mod validator;

pub use compile::{
    compile_simple_with_contract, compile_typed_handoff_contract,
    compile_typed_handoff_contract_with_source, compile_with_contract, compile_with_handoff_source,
    CompileReport, SlotEntry,
};
pub use context::{gather_project_context, ProjectContext, SlotSource};
pub use intent::{
    HandoffContract, HandoffSource, TaskLevel, HANDOFF_CONTRACT_SCHEMA_VERSION, SCHEMA_VERSION,
};
pub use render::{render_card_text, render_report_json, render_report_text};

#[cfg(test)]
use std::path::Path;
#[cfg(test)]
mod compiler_tests;
