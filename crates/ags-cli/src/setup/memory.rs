//! Compatibility adapter for setup-owned host memory mutation.

pub(crate) fn apply_host_memory_adapter(
    report: &mut ags_verification::doctor::HealthReport,
    home: &std::path::Path,
    workspace_root: &std::path::Path,
    host: &str,
    backup_stamp: u64,
) {
    let mut setup_report = ags_lifecycle::setup::SetupReport::new(report.title.clone());
    ags_lifecycle::setup::apply_host_memory_adapter(
        &mut setup_report,
        home,
        workspace_root,
        host,
        backup_stamp,
    );
    super::append_setup_report(report, setup_report);
}
