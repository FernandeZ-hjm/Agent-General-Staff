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

/// Count the longest consecutive run of hex characters in a string.
pub(crate) fn longest_hex_run(s: &str) -> usize {
    let mut max_run = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch.is_ascii_hexdigit() {
            current += 1;
            if current > max_run {
                max_run = current;
            }
        } else {
            current = 0;
        }
    }
    max_run
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

// ── Individual checks ───────────────────────────────────────────────────────

pub(super) fn check_cargo_fmt(repo_root: &Path) -> CheckItem {
    let (code, _stdout, stderr) = run_command(repo_root, "cargo", &["fmt", "--check"], &[]);
    if code == 0 {
        CheckItem::pass("cargo-fmt", "local", "cargo fmt --check passed")
    } else {
        let evidence = if stderr.is_empty() {
            format!("cargo fmt --check failed (exit {})", code)
        } else {
            truncate(&stderr, 500)
        };
        CheckItem::fail(
            "cargo-fmt",
            "local",
            &evidence,
            "Run `cargo fmt` to fix formatting.",
        )
        .with_command("cargo fmt --check")
        .with_exit_code(code)
    }
}

pub(super) fn check_cargo_test(repo_root: &Path) -> CheckItem {
    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &["test"],
        &[("RUSTFLAGS", "-D warnings")],
    );
    if code == 0 {
        // Extract test summary from stdout for evidence
        let summary = stdout
            .lines()
            .filter(|l| l.contains("test result:"))
            .collect::<Vec<_>>()
            .join("\n");
        let evidence = if summary.is_empty() {
            "cargo test passed (warnings as errors)".to_string()
        } else {
            format!(
                "cargo test passed (warnings as errors)\n{}",
                truncate(&summary, 400)
            )
        };
        CheckItem::pass("cargo-test", "local", &evidence)
    } else {
        let combined = format!(
            "stdout:\n{}\nstderr:\n{}",
            truncate(&stdout, 300),
            truncate(&stderr, 300)
        );
        CheckItem::fail(
            "cargo-test",
            "local",
            &combined,
            "Run `RUSTFLAGS=\"-D warnings\" cargo test` to see full output.",
        )
        .with_command("RUSTFLAGS=\"-D warnings\" cargo test")
        .with_exit_code(code)
    }
}

pub(super) fn check_cargo_build(repo_root: &Path) -> CheckItem {
    let (code, _stdout, stderr) = run_command(repo_root, "cargo", &["build", "--release"], &[]);
    if code == 0 {
        CheckItem::pass(
            "cargo-build-release",
            "local",
            "cargo build --release passed",
        )
    } else {
        CheckItem::fail(
            "cargo-build-release",
            "local",
            &truncate(&stderr, 500),
            "Run `cargo build --release` to see full compiler errors.",
        )
        .with_command("cargo build --release")
        .with_exit_code(code)
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

            let (code, stdout, stderr) = run_command(
                repo_root,
                "cargo",
                &[
                    "run",
                    "-q",
                    "-p",
                    "ags-cli",
                    "--",
                    "task",
                    "validate",
                    &path.to_string_lossy(),
                ],
                &[],
            );
            let accepted = code == 0;
            if accepted == should_accept {
                CheckItem::pass(id, "local", success)
            } else {
                CheckItem::fail(
                    id,
                    "local",
                    &format!(
                        "{relative} expected accepted={should_accept}, exit={code}: {}",
                        truncate(&format!("{stdout}\n{stderr}"), 400)
                    ),
                    "Repair the CLI validator or its current-contract fixture.",
                )
                .with_command(&format!(
                    "cargo run -q -p ags-cli -- task validate {relative}"
                ))
                .with_exit_code(code)
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
    // Run `ags session preflight` for smoke verification.
    // Use cargo run since ags may not be on PATH during development.
    let (code, stdout, stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "session",
            "preflight",
            "--for",
            "claude-code",
            "--format",
            "json",
            "--target",
            &repo_root.to_string_lossy(),
        ],
        &[],
    );

    if code == 0 {
        // Verify the JSON output is parseable
        match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(json) => {
                let status = json
                    .get("overall_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                CheckItem::pass(
                    "session-preflight",
                    "local",
                    &format!("session preflight OK (status={})", status),
                )
            }
            Err(e) => CheckItem::fail(
                "session-preflight",
                "local",
                &format!("session preflight produced invalid JSON: {}", e),
                "Check ags session preflight output for errors.",
            )
            .with_command(&format!(
                "ags session preflight --for claude-code --format json --target {}",
                repo_root.display()
            ))
            .with_exit_code(1),
        }
    } else {
        let combined = format!("{}\n{}", truncate(&stdout, 300), truncate(&stderr, 300));
        let remediation = format!(
            "Run `ags session preflight --for claude-code --format json --target {}` to diagnose.",
            repo_root.display()
        );
        CheckItem::fail(
            "session-preflight",
            "local",
            &format!("session preflight failed (exit {}): {}", code, combined),
            &remediation,
        )
        .with_command(&format!(
            "ags session preflight --for claude-code --format json --target {}",
            repo_root.display()
        ))
        .with_exit_code(code)
    }
}
