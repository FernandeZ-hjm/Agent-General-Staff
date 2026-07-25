//! Governed-host identity and native lifecycle facts.
//!
//! This module owns host normalization and the evidence needed to claim a
//! complete project-memory lifecycle. Workspace discovery consumes these facts
//! but does not know host configuration formats.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod host_identity;
mod memory_lifecycle;

pub use host_identity::{recognized_host_display, AgentType};
pub use memory_lifecycle::{
    compute_memory_lifecycle, compute_memory_lifecycle_at, compute_memory_lifecycle_at_for_host,
    compute_memory_lifecycle_for_host, extract_profile_slug, MemoryLifecycle,
};
