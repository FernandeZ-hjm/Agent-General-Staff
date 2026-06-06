//! Diagnostic check functions for suite-doctor.
//!
//! Each check runs a lightweight diagnostic and returns a `Finding`.
//! Checks that shell out are behind simple functions so they can be
//! replaced with stubs in tests — no dynamic dispatch needed.
//!
//! Full-blood checks cover: git status, cargo fmt, workspace structure,
//! memory capsule, task archive, skill directory, auto-trigger status,
//! runner status, receipt/compliance status.

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

// ── Full-blood checks ────────────────────────────────────────────────────

/// Resolve the user memory base directory.
/// Uses $AGS_MEMORY_DIR if set, otherwise defaults to $HOME/.agents/memory/projects.
fn user_memory_base() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("AGS_MEMORY_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        let home = std::env::var("HOME").unwrap_or_default();
        std::path::PathBuf::from(home).join(".agents/memory/projects")
    }
}

/// Resolve the user skills directory (where `ags skill install` writes).
fn user_skills_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".agents/skills")
}

/// Detect the project slug from the repository root directory name.
fn project_slug(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project")
        .to_string()
}

/// Check whether the memory capsule exists for this project.
pub fn memory_capsule_check(repo_root: &Path) -> Finding {
    let slug = project_slug(repo_root);
    let memory_base = user_memory_base();
    let capsule = memory_base.join(&slug).join("context-capsule.md");
    let task_memory = memory_base.join(&slug).join("task-memory.md");

    if capsule.exists() && task_memory.exists() {
        Finding::pass("memory-capsule", "memory capsule and task memory present")
    } else if capsule.exists() {
        Finding::warn(
            "memory-capsule",
            "context capsule found but task memory missing",
            format!(
                "Capsule: {} (exists). Task memory: {} (missing). Run `ags project integrate --target {} --confirm` to initialize project memory, or let `ags archive` create task memory after the first task.",
                capsule.display(),
                task_memory.display(),
                repo_root.display()
            ),
        )
    } else if task_memory.exists() {
        Finding::warn(
            "memory-capsule",
            "task memory found but context capsule missing",
            format!(
                "Task memory: {} (exists). Capsule: {} (missing). Run `ags project integrate --target {} --confirm` to initialize the manual context capsule.",
                task_memory.display(),
                capsule.display(),
                repo_root.display()
            ),
        )
    } else {
        Finding::warn(
            "memory-capsule",
            "memory capsule not initialized",
            format!(
                "Expected at: {}. Run `ags project integrate --target {} --confirm` to create the memory capsule and task memory.",
                memory_base.join(&slug).display(),
                repo_root.display()
            ),
        )
    }
}

/// Check whether the task archive directory exists and has content.
pub fn task_archive_check(repo_root: &Path) -> Finding {
    let slug = project_slug(repo_root);
    let memory_base = user_memory_base();
    let archive_dir = memory_base.join(&slug).join("task-archive");

    if !archive_dir.exists() {
        return Finding::warn(
            "task-archive",
            "task archive directory not found",
            format!(
                "Expected at: {}. Archives are created by `ags archive` or the stop-archive-hook.",
                archive_dir.display()
            ),
        );
    }

    // Count archive files
    match std::fs::read_dir(&archive_dir) {
        Ok(entries) => {
            let count = entries.filter_map(|e| e.ok()).count();
            if count > 0 {
                Finding::pass(
                    "task-archive",
                    format!("task archive has {} entry/entries", count),
                )
            } else {
                Finding::warn(
                    "task-archive",
                    "task archive directory exists but is empty",
                    format!(
                        "Archive dir: {}. Archives are created on task completion.",
                        archive_dir.display()
                    ),
                )
            }
        }
        Err(e) => Finding::fail(
            "task-archive",
            "cannot read task archive",
            format!("{}: {}", archive_dir.display(), e),
        ),
    }
}

/// Check the recommended skill installation status.
/// Verifies that installed skills have proper directory structure and SKILL.md.
pub fn skill_directory_check(_repo_root: &Path) -> Vec<Finding> {
    let skills_dir = user_skills_dir();
    let mut findings = Vec::new();

    if !skills_dir.exists() {
        findings.push(Finding::warn(
            "skill-directory",
            "skills directory not found — no skills installed",
            format!(
                "Expected at: {}. Run `ags skill install --skill recommended --confirm` to install recommended skills.",
                skills_dir.display()
            ),
        ));
        return findings;
    }

    // Check known skill names for proper installation
    let auto_skills = ["auto-brainstorm", "auto-debug", "auto-verify"];
    let manual_skills = [
        "tdd",
        "diagnose",
        "verification-before-completion",
        "webapp-testing",
        "caveman-review",
        "caveman-commit",
    ];

    let mut installed_ok: Vec<String> = Vec::new();
    let mut installed_flat: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    for name in auto_skills.iter().chain(manual_skills.iter()) {
        let dir_path = skills_dir.join(name);
        let skill_file = dir_path.join("SKILL.md");
        let flat_file = skills_dir.join(format!("{}.md", name));

        if skill_file.exists() {
            // Verify frontmatter
            if let Ok(content) = std::fs::read_to_string(&skill_file) {
                let has_name = content.lines().any(|l| l.trim().starts_with("name:"));
                let has_desc = content
                    .lines()
                    .any(|l| l.trim().starts_with("description:"));
                if has_name && has_desc {
                    installed_ok.push(name.to_string());
                } else {
                    findings.push(Finding::warn(
                        format!("skill-{}-frontmatter", name),
                        format!("SKILL.md for '{}' missing frontmatter fields", name),
                        "Expected 'name:' and 'description:' in YAML frontmatter.",
                    ));
                }
            }
        } else if flat_file.exists() {
            installed_flat.push(name.to_string());
        } else {
            missing.push(name.to_string());
        }
    }

    if !installed_ok.is_empty() {
        findings.push(Finding::pass(
            "skill-directory-ok",
            format!(
                "{} skill(s) properly installed with SKILL.md",
                installed_ok.len()
            ),
        ));
    }

    if !installed_flat.is_empty() {
        findings.push(Finding::warn(
            "skill-directory-flat",
            format!(
                "{} skill(s) installed as flat files (legacy format): {}",
                installed_flat.len(),
                installed_flat.join(", ")
            ),
            format!(
                "Reinstall with: ags skill install --skill <name> --confirm --target {}",
                skills_dir.display()
            ),
        ));
    }

    if !missing.is_empty() {
        findings.push(Finding::warn(
            "skill-directory-missing",
            format!(
                "{} recommended skill(s) not installed: {}",
                missing.len(),
                missing.join(", ")
            ),
            format!(
                "Install with: ags skill install --skill recommended --confirm --target {}",
                skills_dir.display()
            ),
        ));
    }

    // Check for install receipt
    let receipt = skills_dir.join("install-receipt.yaml");
    if receipt.exists() {
        findings.push(Finding::pass(
            "skill-install-receipt",
            "skill install receipt found",
        ));
    }

    findings
}

/// Check auto-trigger skill status.
/// Auto-trigger skills (auto-brainstorm, auto-debug, auto-verify) are
/// critical for the full governance experience.
pub fn auto_trigger_check(_repo_root: &Path) -> Finding {
    let skills_dir = user_skills_dir();
    let auto_skills = ["auto-brainstorm", "auto-debug", "auto-verify"];

    let mut installed: Vec<&str> = Vec::new();
    let mut not_installed: Vec<&str> = Vec::new();
    let mut flat_format: Vec<&str> = Vec::new();

    for name in &auto_skills {
        let skill_file = skills_dir.join(name).join("SKILL.md");
        let flat_file = skills_dir.join(format!("{}.md", name));

        if skill_file.exists() {
            installed.push(name);
        } else if flat_file.exists() {
            flat_format.push(name);
        } else {
            not_installed.push(name);
        }
    }

    if installed.len() == 3 {
        Finding::pass(
            "auto-trigger",
            "all 3 auto-trigger skills installed (auto-brainstorm, auto-debug, auto-verify)",
        )
    } else if installed.is_empty() && flat_format.is_empty() {
        Finding::warn(
            "auto-trigger",
            "no auto-trigger skills installed",
            format!(
                "Auto-trigger skills (auto-brainstorm, auto-debug, auto-verify) are not installed. \
                 These are recommended for the full governance experience. \
                 Install with: ags skill install --skill recommended --confirm"
            ),
        )
    } else {
        let mut details: Vec<String> = Vec::new();
        if !installed.is_empty() {
            details.push(format!("Installed: {}", installed.join(", ")));
        }
        if !flat_format.is_empty() {
            details.push(format!(
                "Flat format (reinstall recommended): {}",
                flat_format.join(", ")
            ));
        }
        if !not_installed.is_empty() {
            details.push(format!("Missing: {}", not_installed.join(", ")));
        }
        Finding::warn(
            "auto-trigger",
            "auto-trigger skills partially set up",
            details.join(". "),
        )
    }
}

/// Check runner status — whether the gate-first execution pipeline is functional.
pub fn runner_check(repo_root: &Path) -> Finding {
    // Check key runner components exist
    let run_script = repo_root.join("scripts/run-task-card.sh");
    let validate_script = repo_root.join("scripts/validate.sh");
    let verify_script = repo_root.join("scripts/verify.sh");

    let mut present: Vec<&str> = Vec::new();
    let mut absent: Vec<&str> = Vec::new();

    if run_script.exists() {
        present.push("run-task-card.sh");
    } else {
        absent.push("run-task-card.sh");
    }
    if validate_script.exists() {
        present.push("validate.sh");
    } else {
        absent.push("validate.sh");
    }
    if verify_script.exists() {
        present.push("verify.sh");
    } else {
        absent.push("verify.sh");
    }

    if absent.is_empty() {
        // Check if a release binary exists for the runner
        let has_binary = repo_root.join("target/release/ags").exists()
            || repo_root.join("target/debug/ags").exists()
            || which_in_path("ags").is_some();
        if has_binary {
            Finding::pass(
                "runner",
                "runner scripts and ags binary present — gate-first pipeline functional",
            )
        } else {
            Finding::warn(
                "runner",
                "runner scripts present but ags binary not found",
                "Run `cargo build --release` to build the ags binary for gate-first execution.",
            )
        }
    } else {
        Finding::warn(
            "runner",
            format!("runner scripts incomplete — missing: {}", absent.join(", ")),
            "Run `ags bootstrap --apply --target <dir>` to deploy the full runner suite.",
        )
    }
}

/// Check receipt and compliance status.
/// Verifies that the receipt and compliance infrastructure is present.
pub fn receipt_compliance_check(repo_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Check if the receipt crate exists (for Rust projects)
    let receipt_crate = repo_root.join("crates/receipt/src/lib.rs");
    if receipt_crate.exists() {
        findings.push(Finding::pass(
            "receipt-crate",
            "receipt/compliance crate present (M6)",
        ));
    }

    // Check if stop-archive-hook is available
    let stop_hook = repo_root.join("scripts/stop-archive-hook.sh");
    if stop_hook.exists() {
        findings.push(Finding::pass(
            "stop-archive-hook",
            "stop archive hook script available",
        ));
    } else {
        findings.push(Finding::warn(
            "stop-archive-hook",
            "stop archive hook script not found",
            "Install from scripts/stop-archive-hook.sh to auto-archive on task completion.",
        ));
    }

    // Check recent archives
    let slug = project_slug(repo_root);
    let archive_dir = user_memory_base().join(&slug).join("task-archive");
    if archive_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&archive_dir) {
            let count = entries.filter_map(|e| e.ok()).count();
            if count > 0 {
                findings.push(Finding::pass(
                    "receipt-archives",
                    format!("{} task archive(s) found", count),
                ));
            }
        }
    }

    findings
}

/// Check for full-blood public experience readiness.
/// Aggregates key indicators and reports whether the full governance
/// experience is ready.
pub fn full_blood_readiness_check(repo_root: &Path) -> Finding {
    let skills_dir = user_skills_dir();
    let slug = project_slug(repo_root);
    let memory_base = user_memory_base();
    let capsule = memory_base.join(&slug).join("context-capsule.md");
    let stop_hook = repo_root.join("scripts/stop-archive-hook.sh");

    let auto_installed = ["auto-brainstorm", "auto-debug", "auto-verify"]
        .iter()
        .all(|n| skills_dir.join(n).join("SKILL.md").exists());

    let memory_ready = capsule.exists();

    let stop_hook_available = stop_hook.exists();

    let ags_binary = repo_root.join("target/release/ags").exists()
        || repo_root.join("target/debug/ags").exists()
        || which_in_path("ags").is_some();

    let ready_count = [
        auto_installed,
        memory_ready,
        stop_hook_available,
        ags_binary,
    ]
    .iter()
    .filter(|&&x| x)
    .count();

    if ready_count == 4 {
        Finding::pass(
            "full-blood-readiness",
            "full-blood public experience ready — skills, memory, hooks, binary all present",
        )
    } else {
        let mut missing_items: Vec<&str> = Vec::new();
        if !auto_installed {
            missing_items.push("auto-trigger skills not fully installed");
        }
        if !memory_ready {
            missing_items.push("memory capsule not initialized");
        }
        if !stop_hook_available {
            missing_items.push("stop archive hook not available");
        }
        if !ags_binary {
            missing_items.push("ags binary not in PATH or built");
        }
        Finding::warn(
            "full-blood-readiness",
            format!("full-blood experience: {}/4 components ready", ready_count),
            format!(
                "Missing: {}. Run `ags doctor` for details on each component.",
                missing_items.join("; ")
            ),
        )
    }
}

/// Check if a command exists in PATH.
fn which_in_path(cmd: &str) -> Option<std::path::PathBuf> {
    std::env::var("PATH").ok().and_then(|path| {
        path.split(':')
            .map(|p| std::path::PathBuf::from(p).join(cmd))
            .find(|p| p.exists())
    })
}

// ── Orchestration ────────────────────────────────────────────────────────

/// Run all default suite-doctor checks and populate a `HealthReport`.
///
/// The `repo_root` is typically the current working directory or a
/// configured suite root.
pub fn run_checks(report: &mut HealthReport, repo_root: &Path) {
    // Core checks
    report.add(git_status_check(repo_root));
    report.add(cargo_fmt_check(repo_root));
    for finding in workspace_structure_check(repo_root) {
        report.add(finding);
    }

    // Full-blood checks
    report.add(memory_capsule_check(repo_root));
    report.add(task_archive_check(repo_root));
    for finding in skill_directory_check(repo_root) {
        report.add(finding);
    }
    report.add(auto_trigger_check(repo_root));
    report.add(runner_check(repo_root));
    for finding in receipt_compliance_check(repo_root) {
        report.add(finding);
    }
    report.add(full_blood_readiness_check(repo_root));
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
