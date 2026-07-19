use crate::cli::UpdateLane;
use crate::context::{guard_writable_target, private_install_target, source_root_or_exit};
use crate::managed_projects;
use crate::receipt_bridge::emit_ags_action_receipt;
use crate::setup::run_private_apply;
use crate::update::plan::cmd_update_plan;
use std::path::{Path, PathBuf};

/// Run the two local-kernel build steps (git pull, cargo build). argv elements
/// are passed separately so spaced paths are safe. Never runs registrars.
fn orchestrate_local_kernel_build(source_root: &Path) -> Vec<(String, bool, String)> {
    let mut steps: Vec<(String, bool, String)> = Vec::new();
    let dirty = std::process::Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    if dirty {
        steps.push((
            "git pull --ff-only".to_string(),
            false,
            "source worktree has uncommitted changes; commit/stash before updating".to_string(),
        ));
    } else {
        match std::process::Command::new("git")
            .arg("-C")
            .arg(source_root)
            .args(["pull", "--ff-only"])
            .output()
        {
            Ok(o) => steps.push((
                "git pull --ff-only".to_string(),
                o.status.success(),
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
            )),
            Err(e) => steps.push(("git pull --ff-only".to_string(), false, e.to_string())),
        }
    }
    match std::process::Command::new("cargo")
        .args(["build", "--release", "--manifest-path"])
        .arg(source_root.join("Cargo.toml"))
        .output()
    {
        Ok(o) => {
            let detail = if o.status.success() {
                "built".to_string()
            } else {
                String::from_utf8_lossy(&o.stderr)
                    .trim()
                    .chars()
                    .take(200)
                    .collect()
            };
            steps.push((
                "cargo build --release".to_string(),
                o.status.success(),
                detail,
            ));
        }
        Err(e) => steps.push(("cargo build --release".to_string(), false, e.to_string())),
    }
    steps
}
/// Derive `(apply_status, applied)` for `ags update apply`. Advice-only lane
/// selections (no locally-executable lane) never report an applied update.
fn update_apply_status(executed_local: bool, all_ok: bool) -> (&'static str, bool) {
    if !executed_local {
        ("advised-only", false)
    } else if all_ok {
        ("applied", true)
    } else {
        ("failed", false)
    }
}
/// Advised (never-run) commands for the advice-only lanes in scope.
fn update_advised_commands(lane: Option<UpdateLane>) -> Vec<receipt::ReceiptAdvised> {
    UpdateLane::all()
        .into_iter()
        .filter(|l| !l.auto_executes_locally())
        .filter(|l| lane.map(|sel| sel == *l).unwrap_or(true))
        .map(|l| {
            let command = match l {
                UpdateLane::Agents => "ags agents govern",
                UpdateLane::Skills => "ags skill sync --apply",
                UpdateLane::Projects => "ags update apply --lane projects --apply",
                UpdateLane::Public => "review public boundary; AGS never publishes by default",
                _ => "",
            };
            receipt::ReceiptAdvised {
                command: command.to_string(),
                reason: format!("{} lane (advise-only)", l.id()),
            }
        })
        .collect()
}
pub(in crate::update) fn cmd_update_apply(
    lane: Option<UpdateLane>,
    target: Option<PathBuf>,
    apply: bool,
    force: bool,
    format: &str,
) {
    if !apply {
        cmd_update_plan(lane, format);
        return;
    }
    let source = source_root_or_exit("ags update apply");
    let rt_target = private_install_target(target.clone());
    guard_writable_target("ags update apply", &rt_target);

    let run_core = lane.map(|l| l == UpdateLane::Core).unwrap_or(true);
    let run_runtime = lane.map(|l| l == UpdateLane::Runtime).unwrap_or(true);
    let run_projects = lane.map(|l| l == UpdateLane::Projects).unwrap_or(true);
    // Whether any locally-executable lane was actually selected. Advice-only
    // lane selections (agents / public / skills) execute nothing
    // locally and must NOT be reported as an applied update.
    let executed_local = run_core || run_runtime || run_projects;

    let mut verifications: Vec<receipt::VerificationResult> = Vec::new();
    let mut steps_json: Vec<serde_json::Value> = Vec::new();
    let mut project_json: Vec<serde_json::Value> = Vec::new();
    let mut writes: Vec<receipt::ReceiptWrite> = Vec::new();
    let mut all_ok = true;
    if run_core {
        for (label, ok, detail) in orchestrate_local_kernel_build(&source) {
            if !ok {
                all_ok = false;
            }
            // In json mode, step results go into the JSON object — never as human
            // text before it (machine consumers must get pure JSON on stdout).
            if format != "json" {
                println!(
                    "  [{}] {} — {}",
                    if ok { "ok" } else { "FAIL" },
                    label,
                    detail
                );
            }
            steps_json.push(serde_json::json!({"step": label, "ok": ok, "detail": detail}));
            verifications.push(receipt::VerificationResult {
                command: label,
                exit_code: if ok { 0 } else { 1 },
                output_hash: receipt::sha256_hex(detail.as_bytes()),
            });
        }
    }
    if run_runtime && all_ok {
        // Rewrite AGS-owned runtime/thin-index via the non-exiting apply helper
        // so this command still reaches its own receipt / JSON / exit handling.
        let (rt_report, _t, _pt) = run_private_apply(target.clone(), force, false);
        let rt_ok = rt_report.exit_code() == 0;
        if !rt_ok {
            all_ok = false;
        }
        if format != "json" {
            println!("{}", suite_doctor::render_text(&rt_report));
        }
        verifications.push(receipt::VerificationResult {
            command: "ags setup --yes (runtime/thin-index)".to_string(),
            exit_code: if rt_ok { 0 } else { 1 },
            output_hash: receipt::sha256_hex(b"runtime-reapplied"),
        });
    }
    if run_projects && all_ok {
        let reg_path = managed_projects::registry_path(&rt_target);
        match managed_projects::load(&reg_path) {
            Ok(reg) => {
                let (existing, stale) = managed_projects::partition_existing(&reg);
                if !stale.is_empty() {
                    all_ok = false;
                    for project in stale {
                        project_json.push(serde_json::json!({
                            "target": project.path,
                            "slug": project.slug,
                            "status": "stale",
                            "drift": true,
                            "changed_files": [],
                            "blocked_reasons": ["registered project directory is missing"],
                        }));
                    }
                }
                for project in existing {
                    let report = crate::init::refresh_managed_project(
                        Path::new(&project.path),
                        &project.slug,
                        &source,
                        true,
                    );
                    let ok = report.status == "applied"
                        || report.status == "clean"
                        || report.status == "suite-authority";
                    if !ok {
                        all_ok = false;
                    }
                    for path in &report.changed_files {
                        writes.push(receipt::ReceiptWrite {
                            op: "refresh".to_string(),
                            path: path.clone(),
                            from: None,
                            backup: None,
                            detail: format!("managed project AGS projection: {}", report.slug),
                        });
                    }
                    let detail = format!(
                        "status={} changed={} unchanged={} blocked={}",
                        report.status,
                        report.changed_files.len(),
                        report.unchanged_files.len(),
                        report.blocked_reasons.len()
                    );
                    if format != "json" {
                        println!(
                            "  [{}] project {} — {}",
                            if ok { "ok" } else { "FAIL" },
                            report.target,
                            detail
                        );
                    }
                    verifications.push(receipt::VerificationResult {
                        command: format!("ags update projects refresh {}", report.target),
                        exit_code: if ok { 0 } else { 1 },
                        output_hash: receipt::sha256_hex(detail.as_bytes()),
                    });
                    project_json.push(serde_json::json!({
                        "target": report.target,
                        "slug": report.slug,
                        "status": report.status,
                        "drift": report.drift,
                        "changed_files": report.changed_files,
                        "unchanged_files": report.unchanged_files,
                        "blocked_reasons": report.blocked_reasons,
                    }));
                }
            }
            Err(e) => {
                all_ok = false;
                project_json.push(serde_json::json!({
                    "status": "blocked",
                    "drift": true,
                    "changed_files": [],
                    "blocked_reasons": [e],
                }));
            }
        }
    }

    let advised = update_advised_commands(lane);
    let (status, applied_flag) = update_apply_status(executed_local, all_ok);
    let (decision, reason) = match status {
        "advised-only" => (
            "allow",
            Some("advice-only lane selection — no local execution".to_string()),
        ),
        "applied" => ("allow", None),
        _ => ("stop", Some("local kernel build failed".to_string())),
    };

    let ar = receipt::build_action_receipt(
        "update-apply",
        Some(&rt_target.display().to_string()),
        receipt::GateResult {
            decision: decision.to_string(),
            reason,
        },
        writes,
        vec![],
        advised,
        verifications,
        receipt::RollbackPlan::backup_restore(vec![]),
        status,
        applied_flag,
    );
    let receipt_path = emit_ags_action_receipt(&ar).ok();
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update apply",
                "apply_status": status,
                "applied": applied_flag,
                "executed_local": executed_local,
                "steps": steps_json,
                "projects": project_json,
                "receipt_ref": receipt_path.as_ref().map(|p| p.display().to_string()),
                "note": "core/runtime/projects execute locally under --apply; agents/public/skills remain advise-only. Project refresh never commits or pushes.",
            }))
            .unwrap()
        );
    } else if let Some(p) = &receipt_path {
        println!("\n{}", receipt::render_action_receipt_summary_line(p));
    }
    if executed_local && !all_ok {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod update_apply_tests {
    use super::*;
    use crate::cli::UpdateLane;

    #[test]
    fn update_apply_status_advice_only_lane_is_not_applied() {
        // Advice-only lane selection (nothing executed locally) must NOT report
        // an applied update — guards against false "applied" success.
        assert_eq!(update_apply_status(false, true), ("advised-only", false));
        assert_eq!(update_apply_status(false, false), ("advised-only", false));
        // Locally-executable selection reflects the real outcome.
        assert_eq!(update_apply_status(true, true), ("applied", true));
        assert_eq!(update_apply_status(true, false), ("failed", false));
    }

    #[test]
    fn update_advised_commands_scopes_to_selected_advice_lane() {
        // Selecting an auto lane yields no advice; selecting an advice-only lane
        // yields only that lane's advice.
        assert!(update_advised_commands(Some(UpdateLane::Core)).is_empty());
        let agents = update_advised_commands(Some(UpdateLane::Agents));
        assert_eq!(agents.len(), 1);
        assert!(agents[0].command.contains("ags agents govern"));
        // No lane filter advises all three advice-only lanes.
        assert_eq!(update_advised_commands(None).len(), 3);
    }
}
