//! Bootstrap dry run — simulated readiness report.
//!
//! Produces a `HealthReport` using the shared `suite-doctor` diagnostic
//! types.  All checks are **read-only** and do not execute a real
//! bootstrap: no files are written, no config is applied, no hooks are
//! installed.
//!
//! # Library usage
//!
//! ```text
//! use ags_verification::bootstrap::run;
//! use crate::doctor::render_text;
//!
//! let report = run(std::path::Path::new("."));
//! if !report.passed() {
//!     eprintln!("{}", render_text(&report));
//!     std::process::exit(report.exit_code());
//! }
//! ```

use crate::doctor::{Finding, HealthReport};
use std::path::Path;
use std::process::Command;

// ── Check functions ──────────────────────────────────────────────────────

/// Verify that `cargo` is on `$PATH` and reports a version.
fn cargo_available() -> Finding {
    match Command::new("cargo").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Finding::pass("cargo-available", format!("cargo found: {version}"))
        }
        Ok(output) => Finding::fail(
            "cargo-available",
            "cargo exited with error",
            format!(
                "status={}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(e) => Finding::fail(
            "cargo-available",
            "cargo not found on PATH",
            format!("Failed to run cargo: {e}"),
        ),
    }
}

/// Verify that `rustup` (or at minimum `rustc`) is available.
fn rust_toolchain_available() -> Finding {
    match Command::new("rustc").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Finding::pass("rust-toolchain", format!("rustc found: {version}"))
        }
        Ok(output) => Finding::fail(
            "rust-toolchain",
            "rustc exited with error",
            format!(
                "status={}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(e) => Finding::fail(
            "rust-toolchain",
            "rustc not found",
            format!("Failed to run rustc: {e}"),
        ),
    }
}

/// Check that the target workspace has the expected structure for a
/// bootstrap-capable Rust project.
fn bootstrap_structure_check(repo_root: &Path) -> Vec<Finding> {
    let expected = [
        ("Cargo.toml", "workspace manifest"),
        ("crates/", "crates directory"),
        ("scripts/", "scripts directory"),
    ];

    let mut findings = Vec::new();
    for (rel_path, label) in &expected {
        let full = repo_root.join(rel_path);
        if full.exists() {
            findings.push(Finding::pass(
                format!("bootstrap-structure-{}", sanitize_name(rel_path)),
                format!("{label} present ({rel_path})"),
            ));
        } else {
            findings.push(Finding::warn(
                format!("bootstrap-structure-{}", sanitize_name(rel_path)),
                format!("{label} missing ({rel_path})"),
                format!("Bootstrap may need to create: {}", full.display()),
            ));
        }
    }
    findings
}

/// Sanitize a path fragment into a check-name token.
fn sanitize_name(s: &str) -> String {
    s.trim_end_matches('/').replace(['/', '.'], "-")
}

// ── Public entry point ───────────────────────────────────────────────────

/// Run all bootstrap-dry-run checks and return a populated `HealthReport`.
///
/// `repo_root` is the directory that would be the target of a bootstrap
/// operation (usually the workspace root).
pub fn run(repo_root: &Path) -> HealthReport {
    let mut report = HealthReport::new("bootstrap-dry-run");

    report.add(cargo_available());
    report.add(rust_toolchain_available());
    for finding in bootstrap_structure_check(repo_root) {
        report.add(finding);
    }

    report
}

// ── M7 Bootstrap: plan / apply / verify ──────────────────────────────────

use serde::{Deserialize, Serialize};

/// Schema version for bootstrap plan/apply artifacts.
pub const SCHEMA_VERSION: &str = "0.3.4-bootstrap-plan";

/// A single action in a bootstrap plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapAction {
    pub action: String,
    pub path: String,
    pub description: String,
}

/// A bootstrap plan: the set of actions required to bootstrap a target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapPlan {
    pub schema_version: String,
    pub target: String,
    pub actions: Vec<BootstrapAction>,
}

/// Generate a bootstrap plan for the target directory.
///
/// The plan describes what files would be written — it is **read-only**
/// and does not create any files.
///
/// `source_repo` is the A repository providing protocol/ and scripts/.
/// `target` is the directory to be bootstrapped.
pub fn plan(source_repo: &Path, target: &Path) -> BootstrapPlan {
    let mut actions: Vec<BootstrapAction> = Vec::new();

    // ── Public-safe protocol files ──────────────────────────────────────
    let protocol_files = [
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
        "task-routing.md",
    ];

    for name in &protocol_files {
        let src = source_repo.join("protocol").join(name);
        let dst = target.join("protocol").join(name);
        if src.exists() {
            actions.push(BootstrapAction {
                action: "copy".into(),
                path: dst.display().to_string(),
                description: format!("copy protocol/{}", name),
            });
        }
    }

    // ── Private suite validation script (NOT public-safe) ────────────────
    // validate.sh requires Cargo.toml + ags-cli. verify.sh is excluded
    // entirely because it references private source topology and build gates.
    let scripts = ["validate.sh"];

    for name in &scripts {
        let src = source_repo.join("scripts").join(name);
        let dst = target.join("scripts").join(name);
        if src.exists() {
            actions.push(BootstrapAction {
                action: "copy".into(),
                path: dst.display().to_string(),
                description: format!("copy scripts/{}", name),
            });
        }
    }

    // ── Bootstrap log (generated, not copied) ───────────────────────────
    actions.push(BootstrapAction {
        action: "create".into(),
        path: target.join(".ags-bootstrap.log").display().to_string(),
        description: "create bootstrap log".into(),
    });

    BootstrapPlan {
        schema_version: SCHEMA_VERSION.into(),
        target: target.display().to_string(),
        actions,
    }
}

/// Execute a bootstrap plan — copy files from source to target.
///
/// Creates directories as needed.  Existing files are skipped (no overwrite).
/// Returns a `HealthReport` with one finding per action.
pub fn apply(source_repo: &Path, plan: &BootstrapPlan) -> HealthReport {
    let mut report = HealthReport::new("bootstrap-apply");

    for action in &plan.actions {
        match action.action.as_str() {
            "copy" | "copy-template" => {
                // Extract relative path from the action path
                let dst_path = Path::new(&action.path);
                let rel = dst_path
                    .strip_prefix(Path::new(&plan.target))
                    .unwrap_or(dst_path);
                let src_path = source_repo.join(rel);

                if dst_path.exists() {
                    report.add(Finding::pass(
                        sanitize_name_for_action(&action.path),
                        format!("skipped (exists): {}", action.path),
                    ));
                    continue;
                }

                // Create parent directories
                if let Some(parent) = dst_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        report.add(Finding::fail(
                            sanitize_name_for_action(&action.path),
                            format!("cannot create directory: {}", parent.display()),
                            format!("{}", e),
                        ));
                        continue;
                    }
                }

                // Copy file
                match std::fs::copy(&src_path, dst_path) {
                    Ok(_) => {
                        report.add(Finding::pass(
                            sanitize_name_for_action(&action.path),
                            format!("copied: {} → {}", src_path.display(), dst_path.display()),
                        ));
                    }
                    Err(e) => {
                        report.add(Finding::fail(
                            sanitize_name_for_action(&action.path),
                            format!("copy failed: {}", action.path),
                            format!("{}", e),
                        ));
                    }
                }
            }
            "create" => {
                // .ags-bootstrap.log
                let log_path = Path::new(&action.path);
                if let Some(parent) = log_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(
                    log_path,
                    format!(
                        "bootstrap-apply {}\ntarget: {}\ntimestamp: {}\n",
                        SCHEMA_VERSION,
                        plan.target,
                        chrono_now(),
                    ),
                ) {
                    Ok(_) => {
                        report.add(Finding::pass(
                            sanitize_name_for_action(&action.path),
                            format!("created: {}", action.path),
                        ));
                    }
                    Err(e) => {
                        report.add(Finding::fail(
                            sanitize_name_for_action(&action.path),
                            format!("create failed: {}", action.path),
                            format!("{}", e),
                        ));
                    }
                }
            }
            other => {
                report.add(Finding::warn(
                    sanitize_name_for_action(&action.path),
                    format!("unknown action type: {}", other),
                    &action.path,
                ));
            }
        }
    }

    report
}

/// Verify a bootstrapped target — checks that expected files and directories
/// are present after a bootstrap apply.
///
/// This is a lighter check than `run()` — it does NOT require a Rust
/// workspace (Cargo.toml, cargo, rustc).  Bootstrapped targets may be
/// non-Rust projects that only need protocol/ and scripts/.
pub fn verify(target: &Path) -> HealthReport {
    let mut report = HealthReport::new("bootstrap-verify");

    let expected = [
        ("scripts/validate.sh", "validate script"),
        ("protocol/agent-task-protocol.md", "agent task protocol"),
        ("protocol/task-card-template.md", "task card template"),
        (".ags-bootstrap.log", "bootstrap log"),
    ];

    for (rel_path, label) in &expected {
        let full = target.join(rel_path);
        if full.exists() {
            report.add(Finding::pass(
                format!("bootstrap-verify-{}", sanitize_name(rel_path)),
                format!("{label} present ({rel_path})"),
            ));
        } else {
            report.add(Finding::fail(
                format!("bootstrap-verify-{}", sanitize_name(rel_path)),
                format!("{label} missing ({rel_path})"),
                format!("expected at: {}", full.display()),
            ));
        }
    }

    // A Windows image can expose an unrunnable WSL bash.exe launcher on PATH.
    // Prove the selected executable can actually run before treating its
    // syntax-check exit status as evidence about the copied scripts.
    let bash_usable = ags_platform::is_on_path("bash")
        && std::process::Command::new("bash")
            .args(["-c", "exit 0"])
            .current_dir(target)
            .output()
            .is_ok_and(|output| output.status.success());

    // ── bash -n syntax check on copied scripts ─────────────────────────
    for script in &["scripts/validate.sh"] {
        let full = target.join(script);
        if full.exists() {
            if !bash_usable {
                report.add(Finding::skip(
                    format!("bootstrap-verify-bash-n-{}", sanitize_name(script)),
                    format!("bash -n {script} skipped (bash is not runnable)"),
                ));
                continue;
            }
            let output = std::process::Command::new("bash")
                .arg("-n")
                .arg(script)
                .current_dir(target)
                .output();
            match output {
                Ok(o) if o.status.success() => {
                    report.add(Finding::pass(
                        format!("bootstrap-verify-bash-n-{}", sanitize_name(script)),
                        format!("bash -n {script} OK"),
                    ));
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    report.add(Finding::fail(
                        format!("bootstrap-verify-bash-n-{}", sanitize_name(script)),
                        format!("bash -n {script} FAILED"),
                        stderr.trim().to_string(),
                    ));
                }
                Err(e) => {
                    report.add(Finding::warn(
                        format!("bootstrap-verify-bash-n-{}", sanitize_name(script)),
                        format!("bash not available, skipped syntax check for {script}"),
                        format!("{e}"),
                    ));
                }
            }
        }
    }

    report
}

/// Render a BootstrapPlan as human-readable text.
pub fn render_plan_text(plan: &BootstrapPlan) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Bootstrap Plan {} (target: {})",
        plan.schema_version, plan.target
    ));
    lines.push(String::new());
    for (i, action) in plan.actions.iter().enumerate() {
        lines.push(format!(
            "  {}. [{}] {} — {}",
            i + 1,
            action.action,
            action.path,
            action.description
        ));
    }
    lines.join("\n")
}

/// Render a BootstrapPlan as JSON string.
pub fn render_plan_json(plan: &BootstrapPlan) -> String {
    serde_json::to_string_pretty(plan).unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e))
}

fn sanitize_name_for_action(path: &str) -> String {
    Path::new(path)
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new(path))
        .to_string_lossy()
        .replace('.', "-")
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix-{}", secs)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::Severity;

    #[test]
    fn structure_check_distinguishes_present_and_missing_workspace() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let findings = bootstrap_structure_check(repo_root);

        assert_eq!(findings.len(), 3);
        // Cargo.toml should exist
        let cargo = findings
            .iter()
            .find(|f| f.check_name == "bootstrap-structure-Cargo-toml")
            .unwrap();
        assert_eq!(cargo.severity, Severity::Info);
        assert!(cargo.message.contains("present"));

        let tmp = std::env::temp_dir().join("ags-bootstrap-nonexistent-test");
        let _ = std::fs::remove_dir_all(&tmp);
        let findings = bootstrap_structure_check(&tmp);

        assert_eq!(findings.len(), 3);
        for f in &findings {
            assert_eq!(f.severity, Severity::Warn);
            assert!(f.message.contains("missing"));
        }
    }

    #[test]
    fn names_are_sanitized_for_check_ids() {
        assert_eq!(sanitize_name("crates/"), "crates");
        assert_eq!(sanitize_name("Cargo.toml"), "Cargo-toml");
        assert_eq!(sanitize_name("scripts/verify.sh"), "scripts-verify-sh");
    }

    #[test]
    fn plan_apply_verify_is_one_public_safe_contract() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bootstrapped");
        let plan = plan(repo_root, &target);

        assert_eq!(plan.schema_version, "0.3.4-bootstrap-plan");
        assert!(!plan.actions.is_empty());
        assert!(plan.actions.iter().any(|action| {
            Path::new(&action.path)
                .strip_prefix(&target)
                .is_ok_and(|relative| relative.starts_with("protocol"))
        }));
        assert!(plan.actions.iter().any(|action| {
            Path::new(&action.path)
                .strip_prefix(&target)
                .is_ok_and(|relative| relative.starts_with("scripts"))
        }));
        assert!(!plan
            .actions
            .iter()
            .any(|action| action.path.contains("verify.sh")));
        assert!(
            plan.actions
                .iter()
                .any(|a| a.path.contains(".ags-bootstrap.log")),
            "should include bootstrap log"
        );
        let template_actions: Vec<_> = plan
            .actions
            .iter()
            .filter(|a| a.action == "copy-template")
            .collect();
        assert!(
            template_actions.is_empty(),
            "public bootstrap must not copy private runtime templates"
        );
        for a in &template_actions {
            assert!(
                Path::new(&a.path)
                    .strip_prefix(&target)
                    .is_ok_and(|relative| relative.starts_with("manifests/templates")),
                "template action path must be under manifests/templates/: {}",
                a.path
            );
            assert!(!a.description.contains("/Users/"));
            assert!(!a.path.starts_with("/Users/"));
        }
        let report = apply(repo_root, &plan);

        assert!(report.passed(), "apply should pass: {:?}", report.findings);
        assert!(
            target
                .join("protocol")
                .join("agent-task-protocol.md")
                .exists(),
            "protocol file should exist"
        );
        assert!(
            target.join("scripts").join("validate.sh").exists(),
            "script should exist"
        );
        assert!(
            target.join(".ags-bootstrap.log").exists(),
            "bootstrap log should exist"
        );
        // verify.sh excluded
        assert!(
            !target.join("scripts").join("verify.sh").exists(),
            "verify.sh must not be in bootstrap payload"
        );
        let report = verify(&target);
        assert!(report.passed(), "verify should pass: {:?}", report.findings);
        assert!(report.findings.iter().all(|finding| matches!(
            finding.status,
            crate::doctor::CheckStatus::Pass | crate::doctor::CheckStatus::Skip
        )));
    }
}
