// ── Orchestrator ────────────────────────────────────────────────────────────

use super::*;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocalCheckGroup {
    TaskCardFixtures,
    GovernanceYaml,
    SessionPreflight,
    ProjectSessionPreflight,
}

pub(super) fn local_check_plan(is_ags_suite: bool) -> Vec<LocalCheckGroup> {
    let mut plan = Vec::new();
    if is_ags_suite {
        plan.push(LocalCheckGroup::TaskCardFixtures);
    }
    plan.push(LocalCheckGroup::GovernanceYaml);
    if is_ags_suite {
        plan.push(LocalCheckGroup::SessionPreflight);
    } else {
        plan.push(LocalCheckGroup::ProjectSessionPreflight);
    }
    plan
}

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
    let items = match scope {
        Scope::Governance => {
            let identity = ags_workspace_facts::detect_project(&repo_root);
            let mut items = Vec::new();
            for group in local_check_plan(identity.is_ags_suite) {
                match group {
                    LocalCheckGroup::TaskCardFixtures => {
                        items.extend(check_task_card_fixtures(&repo_root));
                    }
                    LocalCheckGroup::GovernanceYaml => {
                        items.extend(check_governance_yaml(&repo_root));
                    }
                    LocalCheckGroup::SessionPreflight => {
                        items.push(check_session_preflight(&repo_root));
                    }
                    LocalCheckGroup::ProjectSessionPreflight => {
                        items.push(check_project_session_preflight(&repo_root));
                    }
                }
            }
            items
        }
        Scope::Changes => check_workspace_changes(&repo_root),
        Scope::Evidence => check_evidence_contracts(&repo_root),
        Scope::Release => check_release_boundary(release_target_root(&repo_root, options)),
        Scope::Promotion => check_promotion_boundary(&repo_root, options.public_root.as_deref()),
    };

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
        schema_version: "ags://schema/contract/v2/check-report".to_string(),
        scope,
        repo_root: repo_root.to_string_lossy().to_string(),
        project_tests_run: false,
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
