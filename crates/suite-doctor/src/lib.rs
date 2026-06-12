//! Suite doctor — shared diagnostics core.
//!
//! Provides a shared vocabulary of diagnostic types (`HealthReport`,
//! `Finding`, `Severity`, `CheckStatus`) and report rendering (text / JSON)
//! reusable by `ags suite-doctor`, `ags bootstrap-dry-run`, and any future
//! diagnostic CLI.
//!
//! # Library usage
//!
//! ```ignore
//! use suite_doctor::{
//!     render_json, render_text, CheckStatus, Finding, HealthReport, Severity,
//! };
//!
//! let mut report = HealthReport::new("Suite Doctor v2.4.0");
//! report.add(Finding::pass("cargo-fmt", "cargo fmt --check passed"));
//! report.add(Finding::fail("cargo-test", "2 tests failed",
//!     "Run `cargo test` for details."));
//!
//! if !report.passed() {
//!     eprintln!("{}", render_text(&report));
//!     std::process::exit(report.exit_code());
//! }
//! ```

mod checks;
mod report;
mod types;

pub use checks::run_checks;
pub use report::{render_json, render_text};
pub use types::{CheckStatus, Finding, HealthReport, Severity};

use std::path::Path;

/// Run all default suite-doctor checks and return a populated report.
///
/// Uses the given `repo_root` as the base for all checks.
pub fn run(repo_root: &Path) -> HealthReport {
    let mut report = HealthReport::new("suite-doctor");
    run_checks(&mut report, repo_root);
    report
}

/// Stub entry point — kept for backward compatibility with existing
/// callers that don't yet pass `--format`.
///
/// Prefer `run()` + `render_text()` / `render_json()` in new code.
pub fn run_stub() {
    let report = run(Path::new("."));
    println!("{}", render_text(&report));
}

// ── M7 Doctor Repair ──────────────────────────────────────────────────────

/// A single repairable item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepairItem {
    pub check_name: String,
    pub repairable: bool,
    pub description: String,
    pub action: String,
}

/// A repair plan: what can be fixed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepairPlan {
    pub target: String,
    pub items: Vec<RepairItem>,
}

/// Result of a repair operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepairResult {
    pub target: String,
    pub items: Vec<RepairItem>,
    pub repaired: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

/// Generate a repair plan for the target directory (read-only).
pub fn repair_plan(target: &Path) -> RepairPlan {
    let mut items: Vec<RepairItem> = Vec::new();

    // Check 1: scripts directory
    let scripts_dir = target.join("scripts");
    if !scripts_dir.exists() {
        items.push(RepairItem {
            check_name: "scripts-dir".into(),
            repairable: true,
            description: "scripts/ directory is missing".into(),
            action: format!("mkdir -p {}", scripts_dir.display()),
        });
    }

    // Check 2: script permissions
    if scripts_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "sh") {
                    // Simple check — we don't do full POSIX permission checks
                    // in MVP; just report what we'd fix
                    items.push(RepairItem {
                        check_name: "scripts-chmod".into(),
                        repairable: true,
                        description: format!(
                            "ensure +x on {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        action: format!("chmod +x {}", path.display()),
                    });
                }
            }
        }
    }

    // Check 3: cargo fmt (only when Cargo.toml + src/ exist)
    if target.join("Cargo.toml").exists() && target.join("src").exists() {
        items.push(RepairItem {
            check_name: "cargo-fmt".into(),
            repairable: true,
            description: "run cargo fmt on target crate".into(),
            action: format!(
                "cargo fmt --manifest-path {}",
                target.join("Cargo.toml").display()
            ),
        });
    }

    RepairPlan {
        target: target.display().to_string(),
        items,
    }
}

/// Execute repairs on the target directory.
///
/// Only safe, well-defined repairs are performed:
/// 1. Create scripts/ directory if missing
/// 2. chmod +x on scripts/*.sh files
/// 3. cargo fmt (if Cargo.toml + src/ present)
///
/// Never: create protocol files, install hooks, modify .claude/ config.
pub fn repair(target: &Path) -> RepairResult {
    let plan = repair_plan(target);
    let mut repaired: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for item in &plan.items {
        if !item.repairable {
            skipped.push(format!("{} (not repairable)", item.check_name));
            continue;
        }

        match item.check_name.as_str() {
            "scripts-dir" => {
                let dir = target.join("scripts");
                match std::fs::create_dir_all(&dir) {
                    Ok(_) => {
                        repaired.push(format!("{}: created {}", item.check_name, dir.display()))
                    }
                    Err(e) => failed.push(format!("{}: {}", item.check_name, e)),
                }
            }
            "scripts-chmod" => {
                // Extract path from action string: "chmod +x /path/to/file"
                let path_str = item.action.strip_prefix("chmod +x ").unwrap_or("");
                if path_str.is_empty() {
                    skipped.push(format!("{}: could not parse path", item.check_name));
                    continue;
                }
                let script_path = Path::new(path_str);
                // Use std::fs::set_permissions to set executable bit on unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(script_path) {
                        let mut perms = meta.permissions();
                        let mode = perms.mode();
                        perms.set_mode(mode | 0o111);
                        match std::fs::set_permissions(script_path, perms) {
                            Ok(_) => repaired.push(format!(
                                "{}: chmod +x {}",
                                item.check_name,
                                script_path.display()
                            )),
                            Err(e) => failed.push(format!("{}: {}", item.check_name, e)),
                        }
                    } else {
                        skipped.push(format!(
                            "{}: {} not found",
                            item.check_name,
                            script_path.display()
                        ));
                    }
                }
                #[cfg(not(unix))]
                {
                    skipped.push(format!(
                        "{}: chmod not supported on this platform",
                        item.check_name
                    ));
                }
            }
            "cargo-fmt" => {
                // Only run if Cargo.toml + src/ exist (already validated in plan)
                if target.join("Cargo.toml").exists() && target.join("src").exists() {
                    let output = std::process::Command::new("cargo")
                        .arg("fmt")
                        .arg("--manifest-path")
                        .arg(target.join("Cargo.toml"))
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {
                            repaired.push(format!("{}: cargo fmt OK", item.check_name));
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            failed.push(format!(
                                "{}: cargo fmt failed: {}",
                                item.check_name,
                                stderr.trim()
                            ));
                        }
                        Err(e) => {
                            failed.push(format!("{}: {}", item.check_name, e));
                        }
                    }
                } else {
                    skipped.push(format!("{}: no Cargo.toml + src/ found", item.check_name));
                }
            }
            other => {
                skipped.push(format!("{}: unknown repair type", other));
            }
        }
    }

    RepairResult {
        target: target.display().to_string(),
        items: plan.items,
        repaired,
        skipped,
        failed,
    }
}

impl RepairPlan {
    pub fn exit_code(&self) -> i32 {
        if self.items.is_empty() {
            0
        } else {
            0
        }
    }
}

impl RepairResult {
    pub fn exit_code(&self) -> i32 {
        if self.failed.is_empty() {
            0
        } else {
            1
        }
    }
}

// ── Render functions for repair ───────────────────────────────────────

pub fn render_repair_plan_text(plan: &RepairPlan) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Repair Plan (target: {})", plan.target));
    if plan.items.is_empty() {
        lines.push("  (nothing to repair)".into());
    } else {
        for item in &plan.items {
            lines.push(format!(
                "  [{}] {} → {}",
                if item.repairable {
                    "repairable"
                } else {
                    "skip"
                },
                item.description,
                item.action
            ));
        }
    }
    lines.join("\n")
}

pub fn render_repair_plan_json(plan: &RepairPlan) -> String {
    serde_json::to_string_pretty(plan).unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e))
}

pub fn render_repair_text(result: &RepairResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Repair Result (target: {})", result.target));

    if !result.repaired.is_empty() {
        lines.push("Repaired:".into());
        for r in &result.repaired {
            lines.push(format!("  ✓ {}", r));
        }
    }
    if !result.skipped.is_empty() {
        lines.push("Skipped:".into());
        for s in &result.skipped {
            lines.push(format!("  → {}", s));
        }
    }
    if !result.failed.is_empty() {
        lines.push("Failed:".into());
        for f in &result.failed {
            lines.push(format!("  ✗ {}", f));
        }
    }
    if result.repaired.is_empty() && result.skipped.is_empty() && result.failed.is_empty() {
        lines.push("  (nothing to repair)".into());
    }
    lines.join("\n")
}

pub fn render_repair_json(result: &RepairResult) -> String {
    serde_json::to_string_pretty(result).unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e))
}
