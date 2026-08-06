//! Capability snapshot compilation pipeline.

use crate::skill_body::console::{
    build_inventory, inventory_snapshot_hash, CommandOutcome, CommandRunner, ConsoleContext,
    HealthStatus, HostVisibilityStatus, ManagedCapability, ManagedKind, ManagedStatus,
    MutationSurface, RegistryStatus, RouteExamples, RouteState, SystemCommandRunner,
};
use crate::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

mod availability;
mod build;
mod model;
mod source;

pub use build::{
    build_capability_snapshot, build_capability_snapshot_with_live_roots,
    build_capability_snapshot_with_live_roots_at, build_capability_snapshot_with_roots,
    build_capability_snapshot_with_roots_and_manifest, build_capability_snapshot_with_runtime_home,
    build_capability_snapshots_with_live_roots, publish_capability_snapshots,
    write_capability_snapshot_with_roots,
};
pub use model::task_card_skill_tags_from_registry_yaml;
pub use source::{hash_single_file_skill_source, hash_skill_source};
