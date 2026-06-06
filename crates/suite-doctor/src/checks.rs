//! Diagnostic check functions for suite-doctor.
//!
//! Each check runs a lightweight diagnostic and returns a `Finding`.
//! Checks that shell out are behind simple functions so they can be
//! replaced with stubs in tests — no dynamic dispatch needed.

use crate::types::{Finding, HealthReport};
use std::path::Path;
use std::process::Command;

// ── Public check functions ───────────────────────────────────────────────

/// Run `git status --porcelain` and report uncommitted changes.
pub fn git_status_check(repo_root: &Path) -> Finding {
    match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Finding::fail(
                    "git-status",
                    "git status failed",
                    format!(
                        "git exited with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let changed: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
            if changed.is_empty() {
                Finding::pass("git-status", "working tree clean")
            } else {
                Finding::warn(
                    "git-status",
                    format!("{} uncommitted file(s)", changed.len()),
                    format!("Changed: {}", changed.join(", ")),
                )
            }
        }
        Err(e) => Finding::fail(
            "git-status",
            "git not available",
            format!("Failed to run git: {e}"),
        ),
    }
}

/// Run `cargo fmt --check` and report formatting issues.
pub fn cargo_fmt_check(repo_root: &Path) -> Finding {
    match Command::new("cargo")
        .args(["fmt", "--check"])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Finding::pass("cargo-fmt", "cargo fmt --check passed")
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Finding::fail(
                    "cargo-fmt",
                    "cargo fmt --check failed",
                    format!("Run `cargo fmt` to fix. {}", stderr.trim()),
                )
            }
        }
        Err(e) => Finding::fail(
            "cargo-fmt",
            "cargo not available",
            format!("Failed to run cargo: {e}"),
        ),
    }
}

/// Check that key workspace files exist on disk.
pub fn workspace_structure_check(repo_root: &Path) -> Vec<Finding> {
    let required = [
        ("Cargo.toml", "workspace root manifest"),
        ("crates", "crates directory"),
        ("scripts/verify.sh", "verification script"),
    ];

    let mut findings = Vec::new();
    for (rel_path, label) in &required {
        let full = repo_root.join(rel_path);
        if full.exists() {
            findings.push(Finding::pass(
                format!("structure-{}", rel_path.replace('/', "-")),
                format!("{label} present"),
            ));
        } else {
            findings.push(Finding::warn(
                format!("structure-{}", rel_path.replace('/', "-")),
                format!("{label} missing"),
                format!("Expected at: {}", full.display()),
            ));
        }
    }
    findings
}

/// Run all default suite-doctor checks and populate a `HealthReport`.
///
/// The `repo_root` is typically the current working directory or a
/// configured suite root.
pub fn run_checks(report: &mut HealthReport, repo_root: &Path) {
    report.add(git_status_check(repo_root));
    report.add(cargo_fmt_check(repo_root));
    for finding in workspace_structure_check(repo_root) {
        report.add(finding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Severity;

    // ── workspace_structure_check (pure Rust, no shell-out) ───────────

    #[test]
    fn workspace_structure_finds_cargo_toml() {
        // Use the workspace root (where Cargo.toml lives).
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let findings = workspace_structure_check(repo_root);

        // At minimum we should have 3 findings
        assert_eq!(findings.len(), 3);
        // Cargo.toml should be present → pass
        let cargo_finding = findings
            .iter()
            .find(|f| f.check_name == "structure-Cargo.toml")
            .unwrap();
        assert_eq!(cargo_finding.severity, Severity::Info);
    }

    #[test]
    fn workspace_structure_reports_missing_path() {
        let tmp = std::env::temp_dir().join("ags-suite-doctor-nonexistent-test");
        // Ensure it doesn't exist
        let _ = std::fs::remove_dir_all(&tmp);
        let findings = workspace_structure_check(&tmp);

        // All 3 should be missing → warn
        assert_eq!(findings.len(), 3);
        for f in &findings {
            assert_eq!(f.severity, Severity::Warn);
            assert!(f.message.contains("missing"));
        }
    }

    // ── Finding constructors smoke ────────────────────────────────────

    #[test]
    fn git_status_pass_has_correct_shape() {
        let f = Finding::pass("git-status", "working tree clean");
        assert_eq!(f.check_name, "git-status");
        assert_eq!(f.severity, Severity::Info);
        assert!(f.detail.is_none());
    }

    #[test]
    fn cargo_fmt_fail_has_detail() {
        let f = Finding::fail("cargo-fmt", "failed", "run cargo fmt");
        assert_eq!(f.severity, Severity::Fail);
        assert_eq!(f.detail.as_deref(), Some("run cargo fmt"));
    }
}
