//! CLI adapters for lifecycle-owned update lanes.

use crate::cli::UpdateLane as CliUpdateLane;
use crate::managed_projects;
use ags_lifecycle::update::{
    CapabilityInventory, ProjectInventory, ProjectUpdate, UpdateLane, UpdateLanePlan,
};
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

pub(in crate::update) fn inspect_projects(
    source_root: &Path,
    runtime_home: &Path,
    apply: bool,
) -> ProjectInventory {
    let registry = match managed_projects::load(&managed_projects::registry_path(runtime_home)) {
        Ok(registry) => registry,
        Err(error) => {
            return ProjectInventory {
                registry_error: Some(error),
                ..ProjectInventory::default()
            };
        }
    };
    let (existing, stale) = managed_projects::partition_existing(&registry);
    let reports = existing
        .iter()
        .map(|project| {
            let report = crate::init::refresh_managed_project(
                Path::new(&project.path),
                &project.slug,
                source_root,
                apply,
            );
            ProjectUpdate {
                target: report.target,
                slug: report.slug,
                status: report.status,
                drift: report.drift,
                changed_files: report.changed_files,
                unchanged_files: report.unchanged_files,
                blocked_reasons: report.blocked_reasons,
            }
        })
        .collect();
    let stale_reports = stale
        .iter()
        .map(|project| ProjectUpdate {
            target: project.path.clone(),
            slug: project.slug.clone(),
            status: "stale".to_string(),
            drift: true,
            changed_files: Vec::new(),
            unchanged_files: Vec::new(),
            blocked_reasons: vec!["registered project directory is missing".to_string()],
        })
        .collect();
    ProjectInventory {
        registered: registry.projects.len(),
        present: existing.len(),
        stale: stale.len(),
        remote_backed: registry
            .projects
            .iter()
            .filter(|project| managed_projects::is_remote_backed(project))
            .count(),
        reports,
        stale_reports,
        registry_error: None,
    }
}

pub(in crate::update) fn build_all_update_lanes(
    source_root: &Path,
    runtime_home: &Path,
) -> Vec<UpdateLanePlan> {
    let projects = inspect_projects(source_root, runtime_home, false);
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
                    "third_party_registry_freshness": registry.freshness,
                    "fallback_reason": registry.fallback_reason,
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
