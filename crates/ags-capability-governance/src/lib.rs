//! Deterministic capability governance and exact skill resolution.
//!
//! The public interface is unchanged. Internally, authority discovery,
//! catalog resolution, overlay transactions, private persistence, snapshot
//! compilation/validation, usage evidence, and deterministic hashing are
//! separate knowledge modules.

use ags_governance_decision::SkillDemand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skill_body::console::{build_inventory, ConsoleContext, ManagedKind};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

mod authority;
mod catalog;
mod hashing;
mod overlay_transaction;
mod private_store;
mod snapshot_compiler;
mod snapshot_validation;
pub mod third_party_manifest;
mod usage_ledger;

/// Read-only discovery of project-local capabilities.
pub mod project_registry;
/// Skill-body inventory, host probes, mutation plans and transactions.
pub mod skill_body;

pub use authority::*;
pub use catalog::*;
pub use overlay_transaction::*;
pub use private_store::*;
pub use snapshot_compiler::*;
pub use snapshot_validation::*;
pub use usage_ledger::*;

pub use hashing::sha256;

#[cfg(test)]
mod tests;
