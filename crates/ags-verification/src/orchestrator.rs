// ── Orchestrator ────────────────────────────────────────────────────────────

use super::*;
use std::path::{Path, PathBuf};

/// Run all verification checks for the given scope and return a report.
pub fn run_verify(scope: Scope, repo_root: &Path) -> VerificationReport {
    run_verify_with_options(scope, repo_root, &VerificationOptions::default())
}

/// Run verification with explicit cross-repository inputs.
pub fn run_verify_with_options(
    scope: Scope,
    repo_root: &Path,
    options: &VerificationOptions,
) -> VerificationReport {
    let repo_root = canonical_repo_root(repo_root);
    let mut items: Vec<CheckItem> = Vec::new();

    // Local checks — always run
    items.push(check_cargo_fmt(&repo_root));
    items.push(check_cargo_test(&repo_root));
    items.push(check_cargo_build(&repo_root));
    items.extend(check_task_card_fixtures(&repo_root));
    items.extend(check_governance_yaml(&repo_root));
    items.push(check_session_preflight(&repo_root));
    items.push(check_runtime_profile_templates(&repo_root));

    // Release scope — add release-specific checks
    if matches!(scope, Scope::Release) {
        items.extend(check_release_boundary(release_target_root(
            &repo_root, options,
        )));
    }
    if matches!(scope, Scope::Promotion) {
        items.extend(check_promotion_boundary(
            &repo_root,
            options.public_root.as_deref(),
        ));
    }

    // Build summary
    let total = items.len();
    let passed = items
        .iter()
        .filter(|i| i.status == CheckStatus::Pass)
        .count();
    let failed = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail)
        .count();
    let skipped = items
        .iter()
        .filter(|i| i.status == CheckStatus::Skip)
        .count();
    let errors = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail && i.severity == Severity::Error)
        .count();
    let warnings = items
        .iter()
        .filter(|i| i.status == CheckStatus::Fail && i.severity == Severity::Warn)
        .count();

    VerificationReport {
        schema_version: "0.3.5-verification-report".to_string(),
        scope,
        repo_root: repo_root.to_string_lossy().to_string(),
        items,
        summary: VerificationSummary {
            total,
            passed,
            failed,
            skipped,
            errors,
            warnings,
        },
    }
}

pub(super) fn release_target_root<'a>(
    repo_root: &'a Path,
    options: &'a VerificationOptions,
) -> &'a Path {
    options.public_root.as_deref().unwrap_or(repo_root)
}

fn canonical_repo_root(repo_root: &Path) -> PathBuf {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
}
