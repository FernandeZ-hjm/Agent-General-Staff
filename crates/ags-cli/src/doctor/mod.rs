//! `ags doctor` thin facade.
use crate::context::{default_private_runtime_home, guard_writable_target};
use crate::managed_projects;
use std::path::Path;

fn compose_doctor_report(
    kernel: suite_doctor::HealthReport,
    project: suite_doctor::HealthReport,
) -> suite_doctor::HealthReport {
    let mut report = suite_doctor::HealthReport::new("ags-doctor");
    report.findings.extend(kernel.findings);
    report.findings.extend(project.findings);
    report
}

fn capability_routing_report(target: &Path) -> suite_doctor::HealthReport {
    let mut report = suite_doctor::HealthReport::new("capability-routing");
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(std::path::PathBuf::from);
    match crate::context::resolve_capability_authority_root(
        target,
        &skill_resolver::locate_runtime_home(),
        explicit,
    ) {
        Ok(authority_root) => {
            let ctx = skill_governance::console::ConsoleContext::system(authority_root);
            let verify = skill_governance::console::verify_host(&ctx, "codex");
            report.add(suite_doctor::third_party_capability_routing_finding(
                &verify,
            ));
        }
        Err(detail) => report.add(suite_doctor::Finding::fail(
            "third-party-capability-routing",
            "capability authority root could not be resolved",
            detail,
        )),
    }
    report
}

/// Shared dispatch: `doctor` / `suite-doctor`
pub(crate) fn cmd_doctor(format: &str, repair: bool, dry_run: bool, target: &Path) {
    if !repair {
        // Read-only diagnosis. Doctor is the global-pipeline diagnostic authority;
        // it also surfaces the managed-projects registry (global scan).
        let runtime_home = default_private_runtime_home();
        let kernel = crate::setup::private_install_health_report(&runtime_home);
        let project = suite_doctor::run(target);
        let capability = capability_routing_report(target);
        let mut report = compose_doctor_report(kernel, project);
        report.findings.extend(capability.findings);
        match format {
            "json" => println!("{}", suite_doctor::render_json(&report)),
            _ => {
                println!("{}", suite_doctor::render_text(&report));
                let reg = managed_projects::load(&managed_projects::registry_path(
                    &default_private_runtime_home(),
                ))
                .unwrap_or_default();
                println!();
                println!("{}", managed_projects::render_registry_text(&reg));
                println!(
                    "Note: lightweight local repair lives in `ags update repair-local`; doctor stays read-only."
                );
            }
        }
        std::process::exit(report.exit_code());
    }

    if dry_run {
        // Repair dry-run: show what would be repaired
        let plan = suite_doctor::repair_plan(target);
        match format {
            "json" => println!("{}", suite_doctor::render_repair_plan_json(&plan)),
            _ => println!("{}", suite_doctor::render_repair_plan_text(&plan)),
        }
        std::process::exit(plan.exit_code());
    }

    // Actual repair (safe items only)
    guard_writable_target("ags doctor --repair", target);
    let result = suite_doctor::repair(target);
    match format {
        "json" => println!("{}", suite_doctor::render_repair_json(&result)),
        _ => println!("{}", suite_doctor::render_repair_text(&result)),
    }
    std::process::exit(result.exit_code());
}

pub(crate) fn run(format: &str, repair: bool, dry_run: bool, target: &Path) {
    cmd_doctor(format, repair, dry_run, target)
}

#[cfg(test)]
mod tests {
    use super::compose_doctor_report;
    use suite_doctor::{Finding, HealthReport};

    #[test]
    fn doctor_combines_kernel_and_project_findings() {
        let mut kernel = HealthReport::new("kernel");
        kernel.add(Finding::fail("kernel-runtime", "missing", "runtime asset"));
        let mut project = HealthReport::new("project");
        project.add(Finding::warn("project-overlay", "drift", "refresh project"));

        let report = compose_doctor_report(kernel, project);

        assert_eq!(report.title, "ags-doctor");
        assert_eq!(report.findings.len(), 2);
        assert!(report
            .findings
            .iter()
            .any(|f| f.check_name == "kernel-runtime"));
        assert!(report
            .findings
            .iter()
            .any(|f| f.check_name == "project-overlay"));
        assert_eq!(report.exit_code(), 1);
    }
}
