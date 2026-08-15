//! Deterministic capability governance and exact Skill/MCP resolution.
//!
//! Authority discovery, catalog resolution, installed-state persistence,
//! snapshot compilation/validation, and deterministic hashing are separate
//! knowledge modules behind one capability interface.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

mod authority;
mod catalog;
mod hashing;
mod shared_skill_source;
pub mod skill_adoption;
/// Skill-body inventory, host probes, mutation plans and transactions.
pub mod skill_body;
mod snapshot_compiler;
mod snapshot_validation;
pub mod third_party_manifest;

pub use authority::*;
pub use catalog::*;
pub use hashing::snapshot_input_set_hash;
pub use snapshot_compiler::*;
pub use snapshot_validation::*;
