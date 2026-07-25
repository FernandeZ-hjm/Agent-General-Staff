//! Deterministic capability governance and exact skill resolution.
//!
//! The public interface is unchanged. Internally, authority discovery,
//! catalog resolution, overlay transactions, private persistence, snapshot
//! compilation/validation, usage evidence, and deterministic hashing are
//! separate knowledge modules.

use request_governance::SkillDemand;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use skill_governance::console::{
    build_inventory, inventory_snapshot_hash, CommandOutcome, CommandRunner, ConsoleContext,
    HealthStatus, HostVisibilityStatus, ManagedCapability, ManagedKind, ManagedStatus,
    RegistryStatus, RouteExamples, RouteState,
};
use std::collections::{BTreeMap, HashMap, HashSet};
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
mod usage_ledger;

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
