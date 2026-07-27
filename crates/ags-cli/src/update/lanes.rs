//! CLI adapters for lifecycle-owned update lanes.

use crate::cli::UpdateLane as CliUpdateLane;
use ags_lifecycle::update::{CapabilityInventory, UpdateLane, UpdateLanePlan};
use std::path::Path;

pub(in crate::update) fn lifecycle_lane(lane: CliUpdateLane) -> UpdateLane {
    match lane {
        CliUpdateLane::Core => UpdateLane::Core,
        CliUpdateLane::Runtime => UpdateLane::Runtime,
        CliUpdateLane::Agents => UpdateLane::Agents,
        CliUpdateLane::Skills => UpdateLane::Skills,
        CliUpdateLane::Projects => UpdateLane::Projects,
        CliUpdateLane::Public => UpdateLane::Public,
    }
}

pub(in crate::update) fn build_all_update_lanes(
    source_root: &Path,
    runtime_home: &Path,
) -> Vec<UpdateLanePlan> {
    let projects = ags_lifecycle::update::apply::inspect_projects(source_root, runtime_home, false);
    let capabilities =
        match ags_capability_governance::third_party_manifest::resolve_third_party_manifest(
            source_root,
        ) {
            Ok(registry) => CapabilityInventory {
                summary: format!(
                    "third-party capability registry + skill thin-index distribution ({} entries)",
                    registry.manifest.capabilities.len()
                ),
                details: vec![serde_json::json!({
                    "third_party_registry_source": registry.source,
                    "third_party_registry_hash": registry.content_hash,
                    "capabilities": registry.manifest.capabilities.len(),
                    "kinds": ["skill", "cli", "mcp", "hook"],
                })],
            },
            Err(error) => CapabilityInventory {
                summary: "third-party capability registry unavailable".to_string(),
                details: vec![serde_json::json!({"third_party_registry_error": error})],
            },
        };
    ags_lifecycle::update::lanes::build_all_update_lanes(
        source_root,
        runtime_home,
        crate::context::AGS_VERSION,
        &projects,
        &capabilities,
    )
}

pub(in crate::update) use ags_lifecycle::update::lanes::update_lane_json;
