//! ags-kernel — Thin AGS deep module (contract v3).
//!
//! The kernel owns the four invariants that must never degrade:
//! sealed decide/apply transactions, the content-addressed evidence log,
//! workspace identity/binding, and the ownership-safe projection. It also
//! owns the policy vocabulary: the `ags.toml` permission matrix, capability
//! lock, memory closure, and host health. Everything above the kernel is a
//! thin adapter (CLI / MCP / hooks) and must not reimplement policy.

pub mod capabilities;
pub mod config;
pub mod delegation;
pub mod effects;
pub mod error;
pub mod evidence;
pub mod git_projection;
pub mod govern;
pub mod host_projection;
pub mod hosts;
pub mod matrix;
pub mod memory;
pub mod projection;
pub mod route;
pub mod seal;
pub mod skill_adoption;
pub mod skills;
pub mod sync;
pub mod workspace;

pub use error::{Error, Result};
