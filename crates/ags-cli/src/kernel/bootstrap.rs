use crate::context::{ensure_bootstrap_source_repo, guard_writable_target};
use std::path::{Path, PathBuf};

fn render_bootstrap_apply_json(
    plan: &bootstrap_dry_run::BootstrapPlan,
    report: &suite_doctor::HealthReport,
) -> String {
    let output = serde_json::json!({
        "schema_version": bootstrap_dry_run::SCHEMA_VERSION,
        "plan": plan,
        "apply_report": report,
    });
    serde_json::to_string_pretty(&output)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {e}"}}"#))
}

// ── Private runtime install profile ───────────────────────────────────────
/// Shared dispatch: `bootstrap --apply`
fn cmd_bootstrap_apply(target: &Path, format: &str) {
    let source_repo = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ensure_bootstrap_source_repo(&source_repo);

    let plan = bootstrap_dry_run::plan(&source_repo, target);

    // Print plan first
    if format != "json" {
        println!("{}", bootstrap_dry_run::render_plan_text(&plan));
    }

    // Execute plan
    let report = bootstrap_dry_run::apply(&source_repo, &plan);

    match format {
        "json" => println!("{}", render_bootstrap_apply_json(&plan, &report)),
        _ => {
            println!();
            println!("{}", suite_doctor::render_text(&report));
        }
    }

    if !report.passed() {
        std::process::exit(1);
    }
}
/// Shared dispatch: `bootstrap --dry-run` / `bootstrap-dry-run`
pub(crate) fn cmd_bootstrap_dry_run(format: &str) {
    cmd_bootstrap_dry_run_target(std::path::Path::new("."), format);
}
/// Shared dispatch: `bootstrap --dry-run --target <dir>`
fn cmd_bootstrap_dry_run_target(target: &Path, format: &str) {
    let report = bootstrap_dry_run::run(target);
    match format {
        "json" => println!("{}", suite_doctor::render_json(&report)),
        _ => println!("{}", suite_doctor::render_text(&report)),
    }
    std::process::exit(report.exit_code());
}

// ── M2 dispatch functions ──────────────────────────────────────────────────

pub(crate) fn run(dry_run: bool, apply: bool, target: Option<PathBuf>, format: &str) {
    match (dry_run, apply) {
        (false, false) => {
            eprintln!("ags bootstrap: one of --dry-run or --apply is required.");
            eprintln!("  ags bootstrap --dry-run              Check this workspace");
            eprintln!("  ags bootstrap --apply --target <dir>  Bootstrap a target directory");
            std::process::exit(2);
        }
        (true, true) => {
            eprintln!("ags bootstrap: --dry-run and --apply are mutually exclusive.");
            std::process::exit(2);
        }
        (true, false) => {
            let t = target.as_deref().unwrap_or_else(|| Path::new("."));
            cmd_bootstrap_dry_run_target(t, format);
        }
        (false, true) => {
            let t = match target {
                Some(ref t) => t.clone(),
                None => {
                    eprintln!("ags bootstrap: --apply requires --target.");
                    eprintln!("  ags bootstrap --apply --target /tmp/my-target");
                    std::process::exit(2);
                }
            };
            guard_writable_target("ags bootstrap --apply", &t);
            cmd_bootstrap_apply(&t, format);
        }
    }
}
