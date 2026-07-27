//! Capability inventory and host visibility.
//!
//! The console exposes the established public interface while keeping host
//! probing, inventory projection, verification, and rendering in separate
//! internal knowledge modules.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

mod host_probe;
mod host_verify;
mod inventory;
mod model;
mod paths;
mod rendering;

#[allow(unused_imports)]
pub use host_probe::*;
pub use host_verify::*;
pub use inventory::*;
pub use model::*;
use paths::*;
pub use rendering::*;
