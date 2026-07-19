use crate::cli::VerifyAction;
use std::path::Path;

/// Shared dispatch: `verify` and backward-compatible `verify run`.
fn cmd_verify_run(scope: &str, format: &str, target: &Path) {
    if !target.exists() {
        eprintln!("verify: target does not exist — {}", target.display());
        std::process::exit(1);
    }

    let scope = match ags_verify::Scope::from_str(scope) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("verify: {}", e);
            std::process::exit(2);
        }
    };

    let report = ags_verify::run_verify(scope, target);

    match format {
        "json" => println!("{}", ags_verify::render_json(&report)),
        _ => println!("{}", ags_verify::render_text(&report)),
    }

    std::process::exit(report.exit_code());
}
/// `ags verify lane` — classify the change lane for a git diff range.
///
/// Deterministic, read-only. `range` is the commit range under review (e.g.
/// `<a1-head>..HEAD`), or `cached` / `staged` for the index. The push gate uses
/// this to route hygiene changes onto a minimal path; it never defaults the
/// range so a multi-commit push is not misjudged by a `HEAD~1` assumption.
fn cmd_verify_lane(range: &str, format: &str, target: &Path) {
    if !target.exists() {
        eprintln!("verify lane: target does not exist — {}", target.display());
        std::process::exit(1);
    }

    let range_norm = if range == "cached" || range == "staged" {
        format!("--{}", range)
    } else {
        range.to_string()
    };

    match ags_verify::classify_from_git_range(target, &range_norm) {
        Ok(classification) => match format {
            "json" => match serde_json::to_string_pretty(&classification) {
                Ok(s) => println!("{}", s),
                Err(e) => {
                    eprintln!("verify lane: JSON serialization error: {}", e);
                    std::process::exit(1);
                }
            },
            _ => {
                let components: Vec<&str> = classification
                    .components
                    .iter()
                    .map(|c| c.as_str())
                    .collect();
                println!("Lane: {}", classification.lane.as_str());
                println!("Profile: {}", classification.profile.as_str());
                println!("Components: {}", components.join(", "));
                println!("Changed files: {}", classification.changed_files.len());
            }
        },
        Err(e) => {
            eprintln!("verify lane: {}", e);
            std::process::exit(1);
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────

pub(crate) fn run(action: Option<VerifyAction>, scope: &str, format: &str, target: &Path) {
    match action {
        Some(VerifyAction::Run {
            scope,
            format,
            target,
        }) => cmd_verify_run(&scope, &format, &target),
        Some(VerifyAction::Lane {
            range,
            format,
            target,
        }) => cmd_verify_lane(&range, &format, &target),
        None => cmd_verify_run(scope, format, target),
    }
}
