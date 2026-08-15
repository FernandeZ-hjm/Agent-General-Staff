// ── Check execution helpers ─────────────────────────────────────────────────

use super::*;
use std::collections::BTreeMap;
use std::path::Path;

/// Run one external ReadOnly command through the audited, shell-free child
/// runner and return its bounded output prefixes.
pub(crate) fn run_command(
    repo_root: &Path,
    program: &str,
    args: &[&str],
    env_vars: &[(&str, &str)],
) -> (i32, String, String) {
    let spec = crate::test_execution::CommandSpec {
        program: program.to_string(),
        argv: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: ".".into(),
        env: env_vars
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<BTreeMap<_, _>>(),
        timeout_ms: 120_000,
        allowed_write_paths: Vec::new(),
    };
    match crate::test_execution::run_read_only_command(repo_root, &spec) {
        Ok(output) if output.receipt.zero_write_preserved => {
            (output.receipt.exit_code, output.stdout, output.stderr)
        }
        Ok(output) => {
            let detail = format!(
                "read_only_write_detected: {:?}",
                output.receipt.observed_write_set
            );
            (-1, output.stdout, detail)
        }
        Err(error) => (-1, String::new(), format!("Failed to execute: {error}")),
    }
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
/// Uses char boundaries to avoid splitting multi-byte UTF-8 characters.
pub(crate) fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...", truncated)
    }
}

pub(super) fn check_task_card_fixtures(repo_root: &Path) -> Vec<CheckItem> {
    let cases = [
        (
            "fixture-valid-full",
            "tests/fixtures/valid-full.md",
            true,
            "Canonical full task card is accepted by the CLI validator",
        ),
        (
            "fixture-invalid-compact-rejected",
            "tests/fixtures/invalid-compact.md",
            false,
            "Removed compact task-card format is rejected by the CLI validator",
        ),
    ];

    cases
        .into_iter()
        .map(|(id, relative, should_accept, success)| {
            let path = repo_root.join(relative);
            if !path.is_file() {
                return CheckItem::fail(
                    id,
                    "local",
                    &format!("Required task-card fixture is missing: {relative}"),
                    "Restore the current canonical task-card fixture.",
                );
            }

            let input = match std::fs::read_to_string(&path) {
                Ok(input) => input,
                Err(error) => {
                    return CheckItem::fail(
                        id,
                        "local",
                        &format!("Cannot read {relative}: {error}"),
                        "Restore the current canonical task-card fixture.",
                    )
                }
            };
            let validation_errors = ags_task_contract::validator::validate(&input);
            let accepted = validation_errors.is_empty();
            if accepted == should_accept {
                CheckItem::pass(id, "local", success)
            } else {
                CheckItem::fail(
                    id,
                    "local",
                    &format!(
                        "{relative} expected accepted={should_accept}: {}",
                        truncate(&validation_errors.join("; "), 400)
                    ),
                    "Repair the CLI validator or its current-contract fixture.",
                )
                .with_command(&format!(
                    "cargo run -q -p ags-cli -- govern task validate --task-card {relative}"
                ))
                .with_exit_code(if accepted { 0 } else { 1 })
            }
        })
        .collect()
}

pub(super) fn check_governance_yaml(repo_root: &Path) -> Vec<CheckItem> {
    let yaml_files = ["manifests/suite.yaml", "manifests/mcp-registry.yaml"];
    let mut items = Vec::new();

    for yaml_file in &yaml_files {
        let path = repo_root.join(yaml_file);
        if !path.exists() {
            items.push(CheckItem::skip(
                &format!("yaml-{}", yaml_file.replace(['/', '.'], "-")),
                "local",
                &format!("YAML file not found: {}", yaml_file),
            ));
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                items.push(CheckItem::fail(
                    &format!("yaml-{}", yaml_file.replace(['/', '.'], "-")),
                    "local",
                    &format!("Cannot read {}: {}", yaml_file, e),
                    "Check file permissions.",
                ));
                continue;
            }
        };

        let id = format!("yaml-{}", yaml_file.replace(['/', '.'], "-"));

        match serde_yaml::from_str::<serde_yaml::Value>(&content) {
            Ok(_) => {
                items.push(CheckItem::pass(
                    &id,
                    "local",
                    &format!("{} is valid YAML", yaml_file),
                ));
            }
            Err(e) => {
                items.push(CheckItem::fail(
                    &id,
                    "local",
                    &format!("{} YAML parse error: {}", yaml_file, e),
                    &format!("Fix YAML syntax in {}.", yaml_file),
                ));
            }
        }
    }

    items
}

pub(super) fn check_session_preflight(repo_root: &Path) -> CheckItem {
    let preflight = ags_workspace_facts::run_session_preflight(
        repo_root,
        &ags_workspace_facts::AgentType::ClaudeCode,
    );
    if preflight.exit_code == 0 {
        CheckItem::pass(
            "session-preflight",
            "local",
            &format!(
                "session preflight OK (status={:?}, suite={})",
                preflight.overall_status, preflight.is_ags_suite
            )
            .to_ascii_lowercase(),
        )
    } else {
        let remediation = format!(
            "Run `ags doctor --workspace {} --format json` to diagnose.",
            repo_root.display()
        );
        CheckItem::fail(
            "session-preflight",
            "local",
            &format!(
                "session preflight failed (exit {}): {}",
                preflight.exit_code,
                truncate(&preflight.failures.join("; "), 600)
            ),
            &remediation,
        )
        .with_command(&format!(
            "ags doctor --workspace {} --format json",
            repo_root.display()
        ))
        .with_exit_code(preflight.exit_code)
    }
}

pub(super) fn check_project_session_preflight(repo_root: &Path) -> CheckItem {
    let preflight = ags_workspace_facts::run_session_preflight(
        repo_root,
        &ags_workspace_facts::AgentType::ClaudeCode,
    );
    if preflight.exit_code == 0 {
        return CheckItem::pass(
            "session-preflight",
            "local",
            &format!(
                "session preflight OK (status={:?}, integrated={})",
                preflight.overall_status, preflight.is_ags_integrated
            )
            .to_ascii_lowercase(),
        );
    }

    CheckItem::fail(
        "session-preflight",
        "local",
        &format!(
            "session preflight failed: {}",
            truncate(&preflight.failures.join("; "), 500)
        ),
        &format!(
            "Run `ags doctor --workspace {}` to diagnose.",
            repo_root.display()
        ),
    )
    .with_command(&format!("ags doctor --workspace {}", repo_root.display()))
    .with_exit_code(preflight.exit_code)
}

pub(super) fn check_workspace_changes(repo_root: &Path) -> Vec<CheckItem> {
    let (code, stdout, stderr) = run_command(
        repo_root,
        "git",
        &["status", "--porcelain=v1", "--untracked-files=all"],
        &[],
    );
    if code != 0 {
        return vec![CheckItem::fail(
            "changed-path-classification",
            "changes",
            &format!("cannot inspect Git changes: {}", truncate(&stderr, 600)),
            "Restore a readable Git worktree and rerun `ags check changes`.",
        )];
    }
    let changed = stdout
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let classification = crate::classify_lane(&changed);
    vec![CheckItem::pass(
        "changed-path-classification",
        "changes",
        &format!(
            "{} changed path(s) require `{}` verification via `{}` lane",
            classification.changed_files.len(),
            classification.profile.as_str(),
            classification.lane.as_str(),
        ),
    )]
}

pub(super) fn check_evidence_contracts(repo_root: &Path) -> Vec<CheckItem> {
    let cases = [
        ("receipt-valid.json", true, true),
        ("receipt-invalid-hash.json", false, false),
        ("receipt-non-compliant.json", true, false),
    ];
    cases
        .into_iter()
        .map(|(name, expected_valid, expected_compliant)| {
            let relative = format!("tests/fixtures/{name}");
            let result = std::fs::read(repo_root.join(&relative))
                .map_err(|error| format!("cannot read {relative}: {error}"))
                .and_then(|bytes| {
                    serde_json::from_slice::<ags_evidence::Receipt>(&bytes)
                        .map_err(|error| format!("cannot parse {relative}: {error}"))
                });
            let Ok(receipt) = result else {
                return CheckItem::fail(
                    &format!("evidence-{}", name.replace('.', "-")),
                    "evidence",
                    &result.unwrap_err(),
                    "Restore the typed contract-v2 receipt fixture.",
                );
            };
            let valid = ags_evidence::verify_receipt(&receipt).valid;
            let compliant = ags_evidence::check_compliance(&receipt).compliant;
            if valid == expected_valid && compliant == expected_compliant {
                CheckItem::pass(
                    &format!("evidence-{}", name.replace('.', "-")),
                    "evidence",
                    &format!(
                        "{relative} produced valid={valid}, compliant={compliant} as declared"
                    ),
                )
            } else {
                CheckItem::fail(
                    &format!("evidence-{}", name.replace('.', "-")),
                    "evidence",
                    &format!(
                        "{relative} produced valid={valid}, compliant={compliant}; expected valid={expected_valid}, compliant={expected_compliant}"
                    ),
                    "Repair receipt validation or the contract-v2 fixture.",
                )
            }
        })
        .collect()
}
