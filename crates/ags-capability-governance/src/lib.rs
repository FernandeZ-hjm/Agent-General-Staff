//! Deterministic capability governance and exact skill resolution.
//!
//! The public interface is unchanged. Internally, authority discovery,
//! catalog resolution, private persistence, snapshot compilation/validation,
//! and deterministic hashing are
//! separate knowledge modules.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

mod authority;
mod catalog;
mod hashing;
mod private_store;
pub mod skill_adoption;
/// Skill-body inventory, host probes, mutation plans and transactions.
pub mod skill_body;
mod snapshot_compiler;
mod snapshot_validation;
pub mod third_party_manifest;

pub use authority::*;
pub use catalog::*;
pub use hashing::sha256;
pub use private_store::*;
pub use snapshot_compiler::*;
pub use snapshot_validation::*;
