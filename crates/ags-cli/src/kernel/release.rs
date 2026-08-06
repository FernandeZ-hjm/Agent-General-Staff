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
        ReleaseAction::ProjectCapabilities {
            source,
            target,
            apply,
            plan_hash,
            format,
        } => {
            if apply {
                let approved = plan_hash.unwrap_or_else(|| {
                    eprintln!("release project-capabilities: --plan-hash is required with --apply");
                    std::process::exit(2);
                });
                let receipt = ags_verification::public_capability_projection::apply_public_capability_projection(
                    &source,
                    &target,
                    &approved,
                )
                .unwrap_or_else(|error| {
                    eprintln!("release project-capabilities: {error}");
                    std::process::exit(1);
                });
                crate::output::emit(&format, &receipt, || {
                    format!(
                        "Public capability projection applied\nplan_hash: {}\nfiles: {}",
                        receipt.plan_hash,
                        receipt.written_files.join(", ")
                    )
                });
            } else {
                let plan = ags_verification::public_capability_projection::plan_public_capability_projection(
                    &source,
                    &target,
                );
                let blocked = !plan.blocking_findings.is_empty();
                crate::output::emit(&format, &plan, || {
                    let changes = plan
                        .generated_files
                        .iter()
                        .filter(|file| file.changed)
                        .count();
                    format!(
                        "Public capability projection plan\nplan_hash: {}\nchanged: {}\nblocking: {}",
                        plan.plan_hash,
                        changes,
                        plan.blocking_findings.len()
                    )
                });
                if blocked {
                    std::process::exit(1);
                }
            }
        }
        ReleaseAction::ProjectPublic {
            source,
            target,
            apply,
            plan_hash,
            format,
        } => {
            if apply {
                let approved = plan_hash.unwrap_or_else(|| {
                    eprintln!("release project-public: --plan-hash is required with --apply");
                    std::process::exit(2);
                });
                let receipt =
                    ags_verification::public_source_projection::apply_public_source_projection(
                        &source, &target, &approved,
                    )
                    .unwrap_or_else(|error| {
                        eprintln!("release project-public: {error}");
                        std::process::exit(1);
                    });
                crate::output::emit(&format, &receipt, || {
                    format!(
                        "Public source projection applied and verified\nplan_hash: {}\nwritten: {}\ndeleted: {}",
                        receipt.plan_hash,
                        receipt.written_files.len() + receipt.capability_projection.written_files.len(),
                        receipt.deleted_files.len()
                    )
                });
            } else {
                let plan =
                    ags_verification::public_source_projection::plan_public_source_projection(
                        &source, &target,
                    );
                let blocked = !plan.blocking_findings.is_empty();
                crate::output::emit(&format, &plan, || {
                    let generated = plan
                        .capability_projection
                        .generated_files
                        .iter()
                        .filter(|file| file.changed)
                        .count();
                    format!(
                        "Public source projection plan\nplan_hash: {}\nshared writes: {}\ngenerated writes: {}\nretired deletes: {}\nblocking: {}",
                        plan.plan_hash,
                        plan.writes.len(),
                        generated,
                        plan.deletes.len(),
                        plan.blocking_findings.len()
                    )
                });
                if blocked {
                    std::process::exit(1);
                }
            }
        }
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
