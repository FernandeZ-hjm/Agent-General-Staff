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

fn host_entry_semantic_report(policy_path: &Path) -> suite_doctor::HealthReport {
    let mut report = suite_doctor::HealthReport::new("host-entry-semantics");
    let content = match std::fs::read_to_string(policy_path) {
        Ok(content) => content,
        Err(error) => {
            report.add(suite_doctor::Finding::warn(
                "host-entry-semantics",
                format!(
                    "installed host entry policy is unavailable: {}",
                    policy_path.display()
                ),
                error.to_string(),
            ));
            return report;
        }
    };

    let forbidden = [
        "AGS 0.2.8 入口",
        "AGS 0.2.8 Agent",
        "RequestDecision",
        "把完整当前请求交给 `ags_route_request`",
        "`RequestDecision` 的 `SkillDemand`",
    ];
    let stale: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|marker| content.contains(marker))
        .collect();
    let required = [
        "HostRouteProposal",
        "RouteResolution",
        "ags://capabilities/current-host",
        "OMP Plan mode",
        "task_card_hash",
        "validates the existing card first",
    ];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|marker| !content.contains(marker))
        .collect();

    if stale.is_empty() && missing.is_empty() {
        report.add(suite_doctor::Finding::pass(
            "host-entry-semantics",
            format!(
                "installed host entry policy uses AGS 0.3 typed routing and OMP Plan single-card semantics: {}",
                policy_path.display()
            ),
        ));
    } else {
        let mut detail = Vec::new();
        if !stale.is_empty() {
            detail.push(format!("stale markers: {}", stale.join(", ")));
        }
        if !missing.is_empty() {
            detail.push(format!("missing markers: {}", missing.join(", ")));
        }
        report.add(suite_doctor::Finding::fail(
            "host-entry-semantics",
            format!(
                "installed host entry policy has semantic drift: {}",
                policy_path.display()
            ),
            detail.join("; "),
        ));
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
        let host_entry =
            host_entry_semantic_report(&runtime_home.join("hosts/host-entry-policy.md"));
        let mut report = compose_doctor_report(kernel, project);
        report.findings.extend(capability.findings);
        report.findings.extend(host_entry.findings);
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
    use super::{compose_doctor_report, host_entry_semantic_report};
    use std::fs;
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

    #[test]
    fn host_entry_semantic_report_rejects_legacy_router_and_accepts_typed_plan_flow() {
        let base =
            std::env::temp_dir().join(format!("ags-host-entry-doctor-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let policy = base.join("host-entry-policy.md");

        fs::write(
            &policy,
            "AGS 0.2.8 入口\nRequestDecision\n把完整当前请求交给 `ags_route_request`\n",
        )
        .unwrap();
        let stale = host_entry_semantic_report(&policy);
        assert_eq!(stale.exit_code(), 1);
        assert!(stale.findings[0].message.contains("semantic drift"));

        fs::write(
            &policy,
            "HostRouteProposal RouteResolution ags://capabilities/current-host\n\
             OMP Plan mode task_card_hash validates the existing card first\n",
        )
        .unwrap();
        let current = host_entry_semantic_report(&policy);
        assert_eq!(current.exit_code(), 0);
        assert!(current.findings[0].message.contains("AGS 0.3"));
        let _ = fs::remove_dir_all(&base);
    }
}
