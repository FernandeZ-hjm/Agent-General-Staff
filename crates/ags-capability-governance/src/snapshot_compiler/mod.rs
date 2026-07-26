//! Capability snapshot compilation pipeline.

use crate::skill_body::console::{
    build_inventory, inventory_snapshot_hash, CommandOutcome, CommandRunner, ConsoleContext,
    HealthStatus, HostVisibilityStatus, ManagedCapability, ManagedKind, ManagedStatus,
    RegistryStatus, RouteExamples, RouteState,
};
use crate::*;
use ags_governance_decision::SkillDemand;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

mod availability;
mod build;
mod model;
mod source;

pub(crate) use availability::{skill_card, NoProcessDiscovery};
pub use build::{
    build_capability_snapshot, build_capability_snapshot_with_roots,
    build_capability_snapshot_with_roots_and_manifest, build_capability_snapshot_with_runtime_home,
    write_capability_snapshot_with_roots,
};
pub use model::load_demand_routes;
#[cfg(test)]
pub(crate) use model::load_skill_file_metadata;
pub(crate) use model::{load_registry_document, load_skill_metadata_path};
pub use source::hash_skill_source;
#[cfg(test)]
pub(crate) use source::source_hash;
