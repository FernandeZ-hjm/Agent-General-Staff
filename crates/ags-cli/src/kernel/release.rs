use crate::cli::ReleaseAction;
use std::path::PathBuf;
fn render_release_package_plan_text(plan: &serde_json::Value) -> String {
    let mut lines = vec![
        "Release Package Plan".to_string(),
        "====================".to_string(),
        format!("Schema:    {}", plan["schema_version"]),
        format!("Profile:   {}", plan["profile"]),
        format!("Dry run:   {}", plan["dry_run"]),
        format!("Source:    {}", plan["source_root"]),
        String::new(),
        format!(
            "Files:     {} total, {} included, {} excluded",
            plan["summary"]["total_files"],
            plan["summary"]["included"],
            plan["summary"]["excluded"]
        ),
        String::new(),
        "Included:".to_string(),
    ];
    if let Some(files) = plan["included_files"].as_array() {
        for file in files.iter().filter_map(|value| value.as_str()) {
            lines.push(format!("  + {file}"));
        }
    }
    if let Some(files) = plan["forbidden_included"].as_array() {
        if !files.is_empty() {
            lines.push(String::new());
            lines.push("Forbidden included:".to_string());
            for file in files.iter().filter_map(|value| value.as_str()) {
                lines.push(format!("  ! {file}"));
            }
        }
    }
    for (label, key) in [
        ("Required missing", "required_missing"),
        ("Non-authority files", "extra_files"),
        ("Content mismatches", "content_mismatches"),
        ("Authority errors", "authority_errors"),
    ] {
        if let Some(files) = plan[key].as_array() {
            if !files.is_empty() {
                lines.push(String::new());
                lines.push(format!("{label}:"));
                for file in files.iter().filter_map(|value| value.as_str()) {
                    lines.push(format!("  ! {file}"));
                }
            }
        }
    }
    if let Some(files) = plan["excluded_files"].as_array() {
        if !files.is_empty() {
            lines.push(String::new());
            lines.push("Excluded:".to_string());
            for entry in files {
                let file = entry["file"].as_str().unwrap_or("");
                let reason = entry["reason"].as_str().unwrap_or("");
                lines.push(format!("  - {file}  ({reason})"));
            }
        }
    }
    lines.push(String::new());
    lines.push("Verdict: DRY-RUN — no files written. Ready for review.".to_string());
    lines.join("\n")
}
/// Shared dispatch: `release package`
fn cmd_release_package(profile: &str, dry_run: bool, format: &str) {
    if !dry_run {
        eprintln!("release package: --dry-run is required for now. Apply not yet implemented.");
        std::process::exit(2);
    }

    let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (plan, has_forbidden_included) =
        ags_verification::release_package::release_package_plan(&source_root, profile, dry_run);

    crate::output::emit(format, &plan, || render_release_package_plan_text(&plan));

    if has_forbidden_included {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: ReleaseAction) {
    match action {
        ReleaseAction::Package {
            profile,
            dry_run,
            format,
        } => cmd_release_package(&profile, dry_run, &format),
        ReleaseAction::StageRuntime {
            plan,
            source,
            target,
            format,
        } => {
            let result =
                ags_verification::release_package::stage_release_runtime(&plan, &source, &target)
                    .unwrap_or_else(|error| {
                        eprintln!("release stage-runtime: {error}");
                        std::process::exit(1);
                    });
            crate::output::emit(&format, &result, || {
                format!(
                    "Release runtime staged\nsource: {}\ntarget: {}\nfiles: {}",
                    result.source_root,
                    result.target_root,
                    result.staged_files.len()
                )
            });
        }
    }
}
