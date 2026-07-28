use crate::context::{ensure_bootstrap_source_repo, guard_writable_target};
use std::path::{Path, PathBuf};

fn bootstrap_apply_output(
    plan: &ags_verification::bootstrap::BootstrapPlan,
    report: &ags_verification::doctor::HealthReport,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": ags_verification::bootstrap::SCHEMA_VERSION,
        "plan": plan,
        "apply_report": report,
    })
}

// ── Private runtime install profile ───────────────────────────────────────
/// Shared dispatch: `bootstrap --apply`
fn cmd_bootstrap_apply(target: &Path, format: &str) {
    let source_repo = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    ensure_bootstrap_source_repo(&source_repo);

    let plan = ags_verification::bootstrap::plan(&source_repo, target);

    // Execute plan
    let report = ags_verification::bootstrap::apply(&source_repo, &plan);
    let output = bootstrap_apply_output(&plan, &report);
    crate::output::emit(format, &output, || {
        format!(
            "{}\n\n{}",
            ags_verification::bootstrap::render_plan_text(&plan),
            ags_verification::doctor::render_text(&report)
        )
    });

    if !report.passed() {
        std::process::exit(1);
    }
}
/// Shared dispatch: `bootstrap --dry-run --target <dir>`
fn cmd_bootstrap_dry_run_target(target: &Path, format: &str) {
    let report = ags_verification::bootstrap::run(target);
    crate::output::emit_rendered(
        format,
        || ags_verification::doctor::render_json(&report),
        || ags_verification::doctor::render_text(&report),
    );
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
