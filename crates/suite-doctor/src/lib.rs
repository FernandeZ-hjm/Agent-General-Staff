//! Suite doctor — shared AGS kernel/onboarding diagnostics core.
//!
//! Provides a shared vocabulary of diagnostic types (`HealthReport`,
//! `Finding`, `Severity`, `CheckStatus`) and report rendering (text / JSON)
//! reusable by `ags doctor`, `ags bootstrap-dry-run`, and any future diagnostic
//! CLI. Source formatting, tests, and builds are intentionally owned by
//! `ags verify`, not doctor.
//!
//! # Library usage
//!
//! ```ignore
//! use suite_doctor::{
//!     render_json, render_text, CheckStatus, Finding, HealthReport, Severity,
//! };
//!
//! let mut report = HealthReport::new("Suite Doctor v0.3.0");
//! report.add(Finding::pass("kernel-runtime", "runtime assets present"));
//! report.add(Finding::fail("project-protocol", "validator missing",
//!     "Run `ags init --target <project>` to refresh the projection."));
//!
//! if !report.passed() {
//!     eprintln!("{}", render_text(&report));
//!     std::process::exit(report.exit_code());
//! }
//! ```

mod checks;
mod report;
mod types;

pub use checks::{run_checks, skill_resolution_coverage_check, skill_resolution_drift_check};
pub use report::{render_json, render_text};
pub use types::{CheckStatus, Finding, HealthReport, Severity};

use std::path::Path;

/// Run target-aware onboarding checks and return a populated report.
///
/// Suite-only policy checks run only when the target is the AGS suite. Managed
/// projects never inherit Rust/Cargo or suite workspace structure requirements.
pub fn run(repo_root: &Path) -> HealthReport {
    let mut report = HealthReport::new(format!("Suite Diagnostics v{}", env!("CARGO_PKG_VERSION")));
    run_checks(&mut report, repo_root);
    report
}

/// Convert the canonical host capability verification result into the doctor
/// finding that guards third-party routing readiness. This keeps doctor and
/// `ags capability verify --strict` on the same expected-set semantics.
pub fn third_party_capability_routing_finding(
    verify: &skill_governance::console::HostVerifyResult,
) -> Finding {
    let gaps: Vec<String> = verify
        .checks
        .iter()
        .filter(|check| {
            check.expected
                && check.visibility != skill_governance::console::HostVisibilityStatus::Visible
        })
        .map(|check| format!("{} ({:?})", check.name, check.visibility))
        .collect();
    if verify.status == "ok" && gaps.is_empty() {
        return Finding::pass(
            "third-party-capability-routing",
            format!(
                "{} expected capability entries are visible to {}",
                verify.summary.expected, verify.host
            ),
        );
    }

    Finding::fail(
        "third-party-capability-routing",
        format!(
            "{} capability routing is not ready for {}",
            gaps.len(),
            verify.host
        ),
        format!(
            "Expected capability gaps: {}. Install or restore missing third-party bodies through their declared owner, run `ags skill sync --apply` for AGS-owned thin indexes, restart {}, then run `ags capability verify --host {} --strict`.",
            if gaps.is_empty() {
                verify.status.clone()
            } else {
                gaps.join(", ")
            },
            verify.host,
            verify.host
        ),
    )
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
                if path.extension().is_some_and(|e| e == "sh") {
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
///
/// Never: format/build/test project source, create protocol files, install
/// hooks, or modify .claude/ config.
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
        // Repair plans are advisory and never set a non-zero exit code.
        0
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

#[cfg(test)]
mod doctor_scope_tests {
    use super::{repair_plan, third_party_capability_routing_finding, CheckStatus};
    use skill_governance::console::{
        HostCheck, HostVerifyResult, HostVerifySummary, HostVisibilityStatus,
    };

    #[test]
    fn third_party_capability_routing_is_formal_failure_for_expected_gap() {
        let verify = HostVerifyResult {
            schema_version: "test".to_string(),
            host: "codex".to_string(),
            supported: true,
            status: "incomplete".to_string(),
            checks: vec![HostCheck {
                name: "superpowers".to_string(),
                kind: "skill".to_string(),
                visibility: HostVisibilityStatus::NotVisible,
                expected: true,
                evidence: vec!["required host entry missing".to_string()],
            }],
            summary: HostVerifySummary {
                total: 1,
                visible: 0,
                not_visible: 1,
                degraded: 0,
                expected: 1,
                failed: 1,
                all_visible: false,
            },
            thin_index_drift: None,
            shared_thin_index_drift: None,
            note: String::new(),
        };

        let finding = third_party_capability_routing_finding(&verify);
        assert_eq!(finding.check_name, "third-party-capability-routing");
        assert_eq!(finding.status, CheckStatus::Fail);
        assert!(finding
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("superpowers")));
    }

    #[test]
    fn repair_plan_never_formats_target_source() {
        let tmp =
            std::env::temp_dir().join(format!("ags-doctor-repair-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(
            tmp.join("Cargo.toml"),
            "[package]\nname='fixture'\nversion='0.1.0'\n",
        )
        .unwrap();

        let plan = repair_plan(&tmp);

        assert!(
            plan.items.iter().all(|item| item.check_name != "cargo-fmt"),
            "doctor repair must not run project source formatters: {:?}",
            plan.items
        );
        let _ = std::fs::remove_dir_all(tmp);
    }
}
