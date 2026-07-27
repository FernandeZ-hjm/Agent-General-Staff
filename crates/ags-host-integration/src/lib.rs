//! Governed-host identity and native lifecycle facts.
//!
//! This module owns host normalization and the evidence needed to claim a
//! complete project-memory lifecycle. Workspace discovery consumes these facts
//! but does not know host configuration formats.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod agents;
mod host_identity;
mod mcp_probe;
mod memory_lifecycle;
mod platforms;

pub use agents::{
    agents_governance_chain, agents_scan_rows, ags_mcp_tool_surface, default_agents_probe,
    AgentScanRow,
};
pub use host_identity::{recognized_host_display, AgentType};
pub use mcp_probe::{claude_mcp_list_line, codex_mcp_list_line, command_in_path, mcp_server_ids};
pub use memory_lifecycle::{
    compute_memory_lifecycle_at_for_host, compute_memory_lifecycle_for_host, extract_profile_slug,
    MemoryLifecycle,
};
pub use platforms::{
    cross_platform_init_plan, cross_platform_init_plan_with_detectors, AgentPlatformSpec,
    AgentPlatformStatus, CrossPlatformInitPlan, AGENT_PLATFORM_SPECS,
};
