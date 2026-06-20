//! `ags doctor` thin facade.
use crate::context::{default_private_runtime_home, guard_writable_target};
use crate::managed_projects;
use std::path::Path;

/// Shared dispatch: `doctor` / `suite-doctor`
pub(crate) fn cmd_doctor(format: &str, repair: bool, dry_run: bool, target: &Path) {
    if !repair {
        // Read-only diagnosis. Doctor is the global-pipeline diagnostic authority;
        // it also surfaces the managed-projects registry (global scan).
        let report = suite_doctor::run(target);
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
