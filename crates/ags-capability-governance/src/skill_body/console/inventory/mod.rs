//! Unified capability inventory facade.

use super::*;
#[allow(unused_imports)]
use super::{actions::*, host_probe::*, model::*};

mod build;
mod host_directory;
mod source;
mod summary;

pub use build::build_inventory;
pub(super) use host_directory::discover_host_dir_capabilities;
pub(super) use source::canonical_skill_present;
pub use summary::inventory_snapshot_hash;
pub(super) use summary::summarize;
