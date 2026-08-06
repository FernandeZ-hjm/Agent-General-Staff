use crate::cli::{VerifyAction, VerifyBundleAction};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

    crate::output::emit_rendered(
        format,
        || ags_verification::render_json(&report),
        || ags_verification::render_text(&report),
    );

    std::process::exit(report.exit_code());
}
/// `ags verify lane` — classify the change lane for a git diff range.
///
/// Deterministic, read-only. `range` is the commit range under review (e.g.
/// `<a1-head>..HEAD`), or `cached` / `staged` for the index. Release
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
        Ok(classification) => {
            crate::output::emit(format, &classification, || {
                let components: Vec<&str> = classification
                    .components
                    .iter()
                    .map(|c| c.as_str())
                    .collect();
                format!(
                    "Lane: {}\nProfile: {}\nComponents: {}\nChanged files: {}",
                    classification.lane.as_str(),
                    classification.profile.as_str(),
                    components.join(", "),
                    classification.changed_files.len()
                )
            });
        }
        Err(e) => {
            eprintln!("verify lane: {}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_verify_bundle(action: VerifyBundleAction) {
    match action {
        VerifyBundleAction::Create {
            target,
            source_scope,
            report,
            commands,
            test_ids,
            artifacts,
            output,
        } => {
            let report: ags_verification::VerificationReport = read_json(&report, "report");
            let mut artifact_hashes = BTreeMap::new();
            for binding in &artifacts {
                let (name, hash) = parse_artifact(binding).unwrap_or_else(|error| {
                    eprintln!("verify bundle create: {error}");
                    std::process::exit(2);
                });
                if artifact_hashes.insert(name.clone(), hash).is_some() {
                    eprintln!("verify bundle create: duplicate artifact name: {name}");
                    std::process::exit(2);
                }
            }
            let bundle = ags_verification::VerificationBundle::create(
                &target,
                source_scope,
                commands,
                test_ids,
                artifact_hashes,
                report,
                true,
            )
            .unwrap_or_else(|error| {
                eprintln!("verify bundle create: {error}");
                std::process::exit(1);
            });
            let bytes = serde_json::to_vec_pretty(&bundle).expect("bundle serializes");
            ags_platform::atomic_write(&output, &bytes).unwrap_or_else(|error| {
                eprintln!(
                    "verify bundle create: cannot write {}: {error}",
                    output.display()
                );
                std::process::exit(1);
            });
            println!("{}", bundle.bundle_hash);
        }
        VerifyBundleAction::Validate {
            target,
            source_scope,
            bundle,
            format,
        } => {
            let bundle: ags_verification::VerificationBundle = read_json(&bundle, "bundle");
            let result = bundle.validate_reuse_for(
                &target,
                &source_scope,
                ags_verification::TEST_POLICY_VERSION,
            );
            let output = match &result {
                Ok(()) => serde_json::json!({
                    "valid": true,
                    "bundle_hash": bundle.bundle_hash,
                    "commit_sha": bundle.commit_sha,
                    "tree_hash": bundle.tree_hash,
                    "source_scope": bundle.source_scope,
                    "test_policy_version": bundle.test_policy_version,
                }),
                Err(error) => serde_json::json!({"valid": false, "error": error}),
            };
            crate::output::emit(&format, &output, || {
                if result.is_ok() {
                    format!("VerificationBundle valid: {}", bundle.bundle_hash)
                } else {
                    format!("VerificationBundle invalid: {}", output["error"])
                }
            });
            if result.is_err() {
                std::process::exit(1);
            }
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> T {
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        eprintln!(
            "verify bundle: cannot read {label} {}: {error}",
            path.display()
        );
        std::process::exit(1);
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        eprintln!(
            "verify bundle: invalid {label} JSON {}: {error}",
            path.display()
        );
        std::process::exit(1);
    })
}

fn parse_artifact(binding: &str) -> Result<(String, String), String> {
    let (name, path) = binding
        .split_once('=')
        .ok_or_else(|| format!("artifact must be NAME=PATH: {binding}"))?;
    if name.trim().is_empty() || path.trim().is_empty() {
        return Err(format!("artifact must be NAME=PATH: {binding}"));
    }
    let hash = ags_platform::sha256_file(&PathBuf::from(path))?;
    Ok((name.to_string(), hash))
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
        Some(VerifyAction::Bundle { action }) => cmd_verify_bundle(action),
        None => cmd_verify_run(scope, format, target, public_root),
    }
}
