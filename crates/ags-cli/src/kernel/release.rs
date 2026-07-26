use crate::cli::ReleaseAction;
use std::path::PathBuf;

/// Shared dispatch: `release verify`
fn cmd_release_verify(target: &str, format: &str) {
    let variable = match target {
        "stable" => "AGS_RELEASE_STABLE_ROOT",
        "public" | "public-core" | "public-full" | "public-full-sanitized" => {
            "AGS_RELEASE_PUBLIC_ROOT"
        }
        _ => unreachable!("clap guards target values"),
    };
    let Some(target_root) = std::env::var_os(variable).map(PathBuf::from) else {
        eprintln!(
            "release verify: {variable} must name the explicit target checkout; machine topology is not embedded in AGS"
        );
        std::process::exit(2);
    };

    let target_config = ags_verification::sync::TargetConfig {
        root: target_root.clone(),
        name: target.to_string(),
        kind: match target {
            "stable" => ags_verification::sync::ProjectKind::Stable,
            "public" | "public-core" | "public-full" | "public-full-sanitized" => {
                ags_verification::sync::ProjectKind::PublicCoreOnly
            }
            _ => unreachable!(),
        },
    };

    let options = ags_verification::sync::CheckOptions {
        source_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        source_name: "private".to_string(),
        targets: vec![target_config],
        allowlist_path: None,
    };

    let report_format = match format {
        "json" => ags_verification::sync::ReportFormat::Json,
        _ => ags_verification::sync::ReportFormat::Text,
    };

    let ok = ags_verification::sync::run_cli(options, report_format);
    if !ok {
        std::process::exit(1);
    }
}
fn render_release_package_plan_text(plan: &serde_json::Value) {
    println!("Release Package Plan");
    println!("====================");
    println!("Schema:    {}", plan["schema_version"]);
    println!("Profile:   {}", plan["profile"]);
    println!("Dry run:   {}", plan["dry_run"]);
    println!("Source:    {}", plan["source_root"]);
    println!();
    println!(
        "Files:     {} total, {} included, {} excluded",
        plan["summary"]["total_files"], plan["summary"]["included"], plan["summary"]["excluded"]
    );
    println!();
    println!("Included:");
    if let Some(files) = plan["included_files"].as_array() {
        for file in files.iter().filter_map(|value| value.as_str()) {
            println!("  + {}", file);
        }
    }
    if let Some(files) = plan["forbidden_included"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Forbidden included:");
            for file in files.iter().filter_map(|value| value.as_str()) {
                println!("  ! {}", file);
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
                println!();
                println!("{label}:");
                for file in files.iter().filter_map(|value| value.as_str()) {
                    println!("  ! {file}");
                }
            }
        }
    }
    if let Some(files) = plan["excluded_files"].as_array() {
        if !files.is_empty() {
            println!();
            println!("Excluded:");
            for entry in files {
                let file = entry["file"].as_str().unwrap_or("");
                let reason = entry["reason"].as_str().unwrap_or("");
                println!("  - {}  ({})", file, reason);
            }
        }
    }
    println!();
    println!("Verdict: DRY-RUN — no files written. Ready for review.");
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

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        _ => render_release_package_plan_text(&plan),
    }

    if has_forbidden_included {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: ReleaseAction) {
    match action {
        ReleaseAction::Verify { target, format } => cmd_release_verify(&target, &format),
        ReleaseAction::Package {
            profile,
            dry_run,
            format,
        } => cmd_release_package(&profile, dry_run, &format),
    }
}
