use super::*;
use std::path::{Path, PathBuf};

pub(super) fn check_private_vs_stable_drift(repo_root: &Path) -> CheckItem {
    let Some(stable_root) = std::env::var_os("AGS_RELEASE_STABLE_ROOT").map(PathBuf::from) else {
        return CheckItem::skip(
            "drift-private-vs-stable",
            "full",
            "AGS_RELEASE_STABLE_ROOT is not configured.",
        );
    };
    if !stable_root.exists() {
        return CheckItem::skip(
            "drift-private-vs-stable",
            "full",
            &format!("Stable root not found: {}", stable_root.display()),
        );
    }

    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "sync",
            "check",
            "--source",
            &repo_root.to_string_lossy(),
            "--target",
            &stable_root.to_string_lossy(),
            "--target-name",
            "stable",
            "--format",
            "json",
        ],
        &[],
    );

    let output = format!("{}\n{}", stdout, stderr);
    if code == 0 {
        CheckItem::pass(
            "drift-private-vs-stable",
            "full",
            "No protocol drift detected between private and stable.",
        )
    } else {
        // Parse JSON to extract structured drift info
        let evidence = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
            let drift_count = json
                .get("projects")
                .and_then(|p| p.get(0))
                .and_then(|p| p.get("drift_count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!(
                "Protocol drift detected: {} drift item(s) between private and stable.",
                drift_count
            )
        } else {
            format!(
                "Drift check failed (exit {}): {}",
                code,
                truncate(&output, 400)
            )
        };

        CheckItem::warn(
            "drift-private-vs-stable",
            "full",
            &evidence,
            "Sync A to A1 with a direct fast-forward push, then fast-forward S from A1.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name stable",
            repo_root.display(),
            stable_root.display()
        ))
        .with_exit_code(code)
    }
}

pub(super) fn check_private_vs_public_boundary(repo_root: &Path) -> CheckItem {
    let Some(public_root) = std::env::var_os("AGS_RELEASE_PUBLIC_ROOT").map(PathBuf::from) else {
        return CheckItem::skip(
            "drift-private-vs-public",
            "full",
            "AGS_RELEASE_PUBLIC_ROOT is not configured.",
        );
    };
    if !public_root.exists() {
        return CheckItem::skip(
            "drift-private-vs-public",
            "full",
            &format!("Public root not found: {}", public_root.display()),
        );
    }

    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "sync",
            "check",
            "--source",
            &repo_root.to_string_lossy(),
            "--target",
            &public_root.to_string_lossy(),
            "--target-name",
            "public-full-sanitized",
            "--format",
            "json",
        ],
        &[],
    );

    let output = format!("{}\n{}", stdout, stderr);

    // Check for hard boundary violations first
    let has_violation = output.contains("INVARIANT_MISSING")
        || output.contains("INVARIANT_CONTRADICTED")
        || output.contains("PUBLIC_FORBIDDEN_PAYLOAD");

    if code == 0 {
        CheckItem::pass(
            "drift-private-vs-public",
            "full",
            "No public-full sanitized boundary violations detected.",
        )
    } else if has_violation {
        CheckItem::fail(
            "drift-private-vs-public",
            "full",
            &format!(
                "Public-full sanitized boundary violation detected (exit {}): {}",
                code,
                truncate(&output, 500)
            ),
            "Review public-full sanitized boundary: INVARIANT or PUBLIC_FORBIDDEN_PAYLOAD violation.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name public-full-sanitized",
            repo_root.display(),
            public_root.display()
        ))
        .with_exit_code(code)
    } else {
        // Allowlist gap — warn but don't hard-fail
        CheckItem::warn(
            "drift-private-vs-public",
            "full",
            &format!(
                "Public-full sanitized allowlist gap (exit {}): content drift within PUBLIC_MANIFEST files.",
                code
            ),
            "Review public promotion allowlist and update public manifest.",
        )
        .with_command(&format!(
            "ags sync check --source {} --target {} --target-name public-full-sanitized",
            repo_root.display(),
            public_root.display()
        ))
        .with_exit_code(code)
    }
}

pub(super) fn check_promotion_boundary(
    repo_root: &Path,
    public_root: Option<&Path>,
) -> Vec<CheckItem> {
    let Some(public_root) = public_root else {
        return vec![CheckItem::fail(
            "promotion-public-root-required",
            "promotion",
            "Promotion verification requires an explicit public root.",
            "Pass `--public-root <path>` for the exact public worktree under review.",
        )];
    };
    if !public_root.is_dir() {
        return vec![CheckItem::fail(
            "promotion-public-root",
            "promotion",
            &format!("Public root is not a directory: {}", public_root.display()),
            "Provide an existing public worktree path; promotion never guesses a machine-local path.",
        )];
    }

    let public_root_arg = public_root.to_string_lossy().to_string();
    let source_root_arg = repo_root.to_string_lossy().to_string();
    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "sync",
            "check",
            "--source",
            &source_root_arg,
            "--target",
            &public_root_arg,
            "--target-name",
            "public-full-sanitized",
            "--format",
            "json",
        ],
        &[],
    );
    let output = format!("{stdout}\n{stderr}");
    let mut items = Vec::new();
    if code == 0 {
        items.push(CheckItem::pass(
            "promotion-boundary-sync",
            "promotion",
            "Explicit private/stable to public boundary check passed.",
        ));
    } else if let Some(redactions) = allowlisted_promotion_redaction_count(&stdout) {
        items.push(CheckItem::pass(
            "promotion-boundary-sync",
            "promotion",
            &format!(
                "Explicit public boundary contains {redactions} classified legal redaction drift(s) and no blocking drift."
            ),
        ));
    } else {
        items.push(
            CheckItem::fail(
                "promotion-boundary-sync",
                "promotion",
                &format!(
                    "Explicit public promotion boundary check failed (exit {code}): {}",
                    truncate(&output, 500)
                ),
                "Resolve invariant, allowlist, or forbidden-payload drift before promotion.",
            )
            .with_command(&format!(
                "ags sync check --source {} --target {} --target-name public-full-sanitized",
                repo_root.display(),
                public_root.display()
            ))
            .with_exit_code(code),
        );
    }

    let manifest = crate::sync::manifest::verify_promotion_manifest(repo_root, public_root);
    if manifest.passed {
        items.push(CheckItem::pass(
            "promotion-public-manifest",
            "promotion",
            "Explicit public target exactly satisfies the canonical tracked public payload.",
        ));
    } else {
        items.push(CheckItem::fail(
            "promotion-public-manifest",
            "promotion",
            &format!(
                "Public target payload failed: missing=[{}], forbidden=[{}], extra=[{}], content=[{}], authority=[{}]",
                manifest.required_missing.join(", "),
                manifest.forbidden_found.join(", "),
                manifest.extra_files.join(", "),
                manifest.content_mismatches.join(", "),
                manifest.authority_errors.join(", "),
            ),
            "Re-project the public target from manifests/public-release-payload.yaml; do not allowlist non-authority files.",
        ));
    }
    let mut version = check_release_version_surfaces(public_root);
    version.scope = "promotion".to_string();
    version.id = "promotion-version-surfaces".to_string();
    items.push(version);
    items
}

pub(super) fn allowlisted_promotion_redaction_count(output: &str) -> Option<usize> {
    let payload: serde_json::Value = serde_json::from_str(output).ok()?;
    let projects = payload.get("projects")?.as_array()?;
    let mut count = 0;
    for project in projects {
        for drift in project.get("drifts")?.as_array()? {
            let is_legal = drift.get("code").and_then(|value| value.as_str())
                == Some("DRIFT_LEGAL_REDACTION")
                && drift.get("kind").and_then(|value| value.as_str()) == Some("legal_redaction")
                && drift.get("severity").and_then(|value| value.as_str()) == Some("info");
            if !is_legal {
                return None;
            }
            count += 1;
        }
    }
    (count > 0).then_some(count)
}
