use super::*;

/// Run all default suite-doctor checks and populate a `HealthReport`.
///
/// The `repo_root` is typically the current working directory or a configured
/// suite root.
pub fn run_checks(report: &mut HealthReport, repo_root: &Path) {
    let identity = ags_workspace_facts::detect_project(repo_root);
    report.add(git_status_check(repo_root));
    report.add(project_integration_check(&identity));
    report.add(project_protocol_check(repo_root));
    for finding in canonical_conformance_checks(repo_root) {
        report.add(finding);
    }

    // Source-policy checks apply only to the AGS suite itself. `ags doctor`
    // never treats Cargo or the suite workspace layout as requirements of a
    // managed target project; source formatting/build checks belong to
    // `ags verify`.
    if identity.is_ags_suite {
        for finding in skill_resolution_drift_check(repo_root) {
            report.add(finding);
        }
        for finding in skill_resolution_coverage_check(repo_root) {
            report.add(finding);
        }
        report.add(mcp_registry_codegraph_active(repo_root));
    }

    if identity.is_ags_suite {
        report.add(host_skill_body_singleton_check(repo_root));
    }
    report.add(project_task_memory_status(repo_root));
    report.add(context_capsule_integrity(repo_root));
}
