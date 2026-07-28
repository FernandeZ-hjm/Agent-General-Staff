//! `ags doctor` thin facade.
use crate::context::{default_private_runtime_home, guard_writable_target, home_dir};
use ags_workspace_facts::managed_projects;
use std::path::Path;

fn compose_doctor_report(
    kernel: ags_verification::doctor::HealthReport,
    project: ags_verification::doctor::HealthReport,
) -> ags_verification::doctor::HealthReport {
    let mut report = ags_verification::doctor::HealthReport::new("ags-doctor");
    report.findings.extend(kernel.findings);
    report.findings.extend(project.findings);
    report
}

fn capability_routing_report(target: &Path) -> ags_verification::doctor::HealthReport {
    let mut report = ags_verification::doctor::HealthReport::new("capability-routing");
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(std::path::PathBuf::from);
    match crate::context::resolve_capability_authority_root(
        target,
        &ags_capability_governance::locate_runtime_home(),
        explicit,
    ) {
        Ok(authority_root) => {
            let ctx = ags_capability_governance::skill_body::console::ConsoleContext::system(
                authority_root,
            );
            let verify = ags_capability_governance::skill_body::console::verify_host(&ctx, "codex");
            report.add(ags_verification::doctor::third_party_capability_routing_finding(&verify));
        }
        Err(detail) => report.add(ags_verification::doctor::Finding::fail(
            "third-party-capability-routing",
            "capability authority root could not be resolved",
            detail,
        )),
    }
    report
}

fn host_entry_semantic_report(core_path: &Path) -> ags_verification::doctor::HealthReport {
    let mut report = ags_verification::doctor::HealthReport::new("host-entry-semantics");
    let core = match std::fs::read_to_string(core_path) {
        Ok(content) => content,
        Err(error) => {
            report.add(ags_verification::doctor::Finding::warn(
                "host-entry-semantics",
                format!("shared host entry is unavailable: {}", core_path.display()),
                error.to_string(),
            ));
            return report;
        }
    };
    let handoff_path = core_path.with_file_name("ags-task-handoff.md");
    let handoff = match std::fs::read_to_string(&handoff_path) {
        Ok(content) => content,
        Err(error) => {
            report.add(ags_verification::doctor::Finding::fail(
                "host-entry-semantics",
                format!(
                    "shared host entry companion is unavailable: {}",
                    handoff_path.display()
                ),
                error.to_string(),
            ));
            return report;
        }
    };
    let content = format!("{core}\n{handoff}");

    let forbidden = [
        "AGS 0.2.8 入口",
        "AGS 0.2.8 Agent",
        "RequestDecision",
        "把完整当前请求交给 `ags_route_request`",
        "`RequestDecision` 的 `SkillDemand`",
        "demand_routes",
    ];
    let stale: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|marker| content.contains(marker))
        .collect();
    let core_required = [
        "HostRouteProposal",
        "RouteResolution",
        "ags://capabilities/current-host",
        "ags-task-handoff.md",
        "verification-before-completion",
    ];
    let handoff_required = ["OMP Plan 单卡出口", "task_card_hash", "validate-first"];
    let mut missing: Vec<&str> = core_required
        .iter()
        .copied()
        .filter(|marker| !core.contains(marker))
        .collect();
    missing.extend(
        handoff_required
            .iter()
            .copied()
            .filter(|marker| !handoff.contains(marker)),
    );

    if stale.is_empty() && missing.is_empty() {
        report.add(ags_verification::doctor::Finding::pass(
            "host-entry-semantics",
            format!(
                "concise shared host entry references typed routing and task-handoff semantics: {}",
                core_path.display(),
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
        report.add(ags_verification::doctor::Finding::fail(
            "host-entry-semantics",
            format!(
                "shared host entry has semantic drift: {}",
                core_path.display()
            ),
            detail.join("; "),
        ));
    }
    report
}

/// Dispatch the current `doctor` command.
pub(crate) fn cmd_doctor(format: &str, repair: bool, dry_run: bool, target: &Path) {
    if !repair {
        // Read-only diagnosis. Doctor is the global-pipeline diagnostic authority;
        // it also surfaces the managed-projects registry (global scan).
        let runtime_home = default_private_runtime_home();
        let kernel = crate::setup::private_install_health_report(&runtime_home, false);
        let project = ags_verification::doctor::run(target);
        let capability = capability_routing_report(target);
        let host_entry = host_entry_semantic_report(&home_dir().join(".agents/rules/ags-core.md"));
        let mut report = compose_doctor_report(kernel, project);
        report.findings.extend(capability.findings);
        report.findings.extend(host_entry.findings);
        let reg = managed_projects::load(&managed_projects::registry_path(
            &default_private_runtime_home(),
        ))
        .unwrap_or_default();
        crate::output::emit_rendered(
            format,
            || ags_verification::doctor::render_json(&report),
            || {
                format!(
                    "{}\n\n{}\nNote: lightweight local repair lives in `ags update repair-local`; doctor stays read-only.",
                    ags_verification::doctor::render_text(&report),
                    managed_projects::render_registry_text(&reg)
                )
            },
        );
        std::process::exit(report.exit_code());
    }

    if dry_run {
        // Repair dry-run: show what would be repaired
        let plan = ags_verification::doctor::repair_plan(target);
        crate::output::emit_rendered(
            format,
            || ags_verification::doctor::render_repair_plan_json(&plan),
            || ags_verification::doctor::render_repair_plan_text(&plan),
        );
        std::process::exit(plan.exit_code());
    }

    // Actual repair (safe items only)
    guard_writable_target("ags doctor --fix", target);
    let result = ags_verification::doctor::repair(target);
    crate::output::emit_rendered(
        format,
        || ags_verification::doctor::render_repair_json(&result),
        || ags_verification::doctor::render_repair_text(&result),
    );
    std::process::exit(result.exit_code());
}

pub(crate) fn run(format: &str, repair: bool, dry_run: bool, target: &Path) {
    cmd_doctor(format, repair, dry_run, target)
}

#[cfg(test)]
mod tests {
    use super::{compose_doctor_report, host_entry_semantic_report};
    use ags_verification::doctor::{Finding, HealthReport};
    use std::fs;

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
        assert_eq!(
            report.exit_code(),
            1,
            "kernel failures must remain blocking"
        );
    }

    #[test]
    fn host_entry_semantic_report_rejects_unsupported_router_and_accepts_typed_plan_flow() {
        let base =
            std::env::temp_dir().join(format!("ags-host-entry-doctor-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let core = base.join("core.md");
        let handoff = base.join("ags-task-handoff.md");

        fs::write(
            &core,
            "AGS 0.2.8 入口\nRequestDecision\n把完整当前请求交给 `ags_route_request`\n",
        )
        .unwrap();
        fs::write(&handoff, "unsupported").unwrap();
        let stale = host_entry_semantic_report(&core);
        assert_eq!(stale.exit_code(), 1);
        assert!(stale.findings[0].message.contains("semantic drift"));

        fs::write(
            &core,
            "HostRouteProposal RouteResolution ags://capabilities/current-host\n\
             ags-task-handoff.md verification-before-completion\n",
        )
        .unwrap();
        fs::write(
            &handoff,
            "## OMP Plan 单卡出口\n\
             task_card_hash validate-first\n",
        )
        .unwrap();
        let current = host_entry_semantic_report(&core);
        assert_eq!(current.exit_code(), 0);
        assert!(current.findings[0]
            .message
            .contains("concise shared host entry"));
        let _ = fs::remove_dir_all(&base);
    }
}
