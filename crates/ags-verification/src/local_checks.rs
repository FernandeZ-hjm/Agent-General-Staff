// ── Check execution helpers ─────────────────────────────────────────────────

use super::*;
use std::path::Path;
use std::process::Command;

/// Run a shell command and return (exit_code, stdout, stderr).
pub(crate) fn run_command(
    repo_root: &Path,
    program: &str,
    args: &[&str],
    env_vars: &[(&str, &str)],
) -> (i32, String, String) {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.current_dir(repo_root);
    for (key, value) in env_vars {
        cmd.env(key, value);
    }
    // Suppress cargo's progress output for cleaner evidence
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    match cmd.output() {
        Ok(output) => {
            let code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (code, stdout, stderr)
        }
        Err(e) => (-1, String::new(), format!("Failed to execute: {}", e)),
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
                    "cargo run -q -p ags-cli -- task validate {relative}"
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
            "Run `ags session preflight --for claude-code --format json --target {}` to diagnose.",
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
            "ags session preflight --for claude-code --format json --target {}",
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
            "Run `ags session preflight --for claude-code --target {}` to diagnose.",
            repo_root.display()
        ),
    )
    .with_command(&format!(
        "ags session preflight --for claude-code --target {}",
        repo_root.display()
    ))
    .with_exit_code(preflight.exit_code)
}
