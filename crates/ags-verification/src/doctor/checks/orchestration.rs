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
        report.add(mcp_registry_codegraph_adopted(repo_root));
    }

    // ── AGS project-memory capture chain (advisory) ────────────────────
    report.add(memory_capture_scripts_present());
    if identity.is_ags_suite {
        report.add(host_skill_body_singleton_check(repo_root));
    }
    // Host-specific start/close wiring is covered by the composite lifecycle
    // check below. The legacy Claude-only checks remain callable for focused
    // diagnostics but are not duplicated in the default report.
    report.add(raw_tool_call_stop_guard_present(repo_root));
    report.add(project_task_memory_status(repo_root));
    report.add(context_capsule_integrity(repo_root));
    // ── Memory lifecycle closure (composite) ──────────────────────────
    report.add(project_memory_lifecycle_closure(repo_root));
}
