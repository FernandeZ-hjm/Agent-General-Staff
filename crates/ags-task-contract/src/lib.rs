//! ags-task-contract — the contract-v3 task surface.
//!
//! Owns the canonical ≤13-field card skeleton, its validator, the structured
//! command runner (no shell interpolation), and the `ags run` orchestration
//! (prepare → execute → verify → close). It depends inward on `ags-kernel`
//! and owns no policy.

pub mod authority;
pub mod command;
pub mod runner;
pub mod template;
pub mod validator;

pub use authority::{derive_authority, derive_state, TaskAuthority};
pub use runner::{run_close, run_prepare, run_verify};
pub use validator::{validate_file, ValidationError, ValidationResult};
