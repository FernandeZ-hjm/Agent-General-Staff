use crate::cli::VerifyAction;
use std::path::Path;

/// Run the top-level `verify` command.
fn cmd_verify_run(scope: &str, format: &str, target: &Path, public_root: Option<&Path>) {
    if !target.exists() {
        eprintln!("verify: target does not exist — {}", target.display());
        std::process::exit(1);
    }

    let scope = match ags_verification::Scope::from_str(scope) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("verify: {}", e);
            std::process::exit(2);
        }
    };

    let options = ags_verification::VerificationOptions {
        public_root: public_root.map(Path::to_path_buf),
    };
    let report = ags_verification::run_verify_with_options(scope, target, &options);

    match format {
        "json" => println!("{}", ags_verification::render_json(&report)),
        _ => println!("{}", ags_verification::render_text(&report)),
    }

    std::process::exit(report.exit_code());
}
/// `ags verify lane` — classify the change lane for a git diff range.
///
/// Deterministic, read-only. `range` is the commit range under review (e.g.
/// `<a1-head>..HEAD`), or `cached` / `staged` for the index. Release/sync
/// automation can use this to route hygiene changes onto a minimal path; it never defaults the
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

    match ags_verification::classify_from_git_range(target, &range_norm) {
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

pub(crate) fn run(
    action: Option<VerifyAction>,
    scope: &str,
    format: &str,
    target: &Path,
    public_root: Option<&Path>,
) {
    match action {
        Some(VerifyAction::Lane {
            range,
            format,
            target,
        }) => cmd_verify_lane(&range, &format, &target),
        None => cmd_verify_run(scope, format, target, public_root),
    }
}
