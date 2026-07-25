//! Capability inventory and mutation console.
//!
//! The console exposes the established public interface while keeping host
//! probing, inventory projection, guarded mutations, synchronization,
//! deduplication, and rendering in separate internal knowledge modules.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

mod actions;
mod apply_transaction;
mod dedupe;
mod host_probe;
mod host_verify;
mod inventory;
mod model;
mod rendering;
mod sync;

pub use actions::*;
pub use apply_transaction::*;
pub use dedupe::*;
pub use host_probe::*;
pub use host_verify::*;
pub use inventory::*;
pub use model::*;
pub use rendering::*;
pub use sync::*;

#[cfg(test)]
mod tests;
