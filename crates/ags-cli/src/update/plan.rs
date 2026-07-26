use crate::cli::UpdateLane;
use crate::context::{
    capability_authority_root_or_exit, default_private_runtime_home, source_root_or_exit,
    AGS_VERSION,
};
use crate::managed_projects;
use crate::update::lanes::{build_all_update_lanes, lifecycle_lane, update_lane_json};
use ags_lifecycle::update::UpdateLanePlan;
use std::path::PathBuf;

/// Runtime home verified by `ags update verify`. An explicit `--target` is the
/// operator's target runtime; without it, fall back to the normal AGS runtime
/// home.
fn update_verify_runtime_home(target: Option<PathBuf>) -> PathBuf {
    target.unwrap_or_else(default_private_runtime_home)
}

pub(in crate::update) fn cmd_update_check(format: &str) {
    let source = source_root_or_exit("ags update check");
    let home = default_private_runtime_home();
    let lanes = build_all_update_lanes(&source, &home);
    // Read-only notifier status: reflects stored update-state.json only. NO
    // network probe, NO state write here — due fetch/write belongs to
    // `ags update notify`. Read from the notifier's own runtime home.
    let notify_home = ags_capability_governance::locate_runtime_home();
    if format == "json" {
        let arr: Vec<_> = lanes.iter().map(update_lane_json).collect();
        let reg =
            managed_projects::load(&managed_projects::registry_path(&home)).unwrap_or_default();
        let reg_json: serde_json::Value =
            serde_json::from_str(&managed_projects::render_registry_json(&reg)).unwrap_or_default();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update check",
                "version": AGS_VERSION,
                "lanes": arr,
                "managed_projects": reg_json,
                "notifier": crate::update::notifier::notifier_status_json(&notify_home),
            }))
            .unwrap()
        );
    } else {
        println!("AGS Update — drift check (read-only)");
        println!("Version: {AGS_VERSION}");
        for p in &lanes {
            let drift = match p.drift {
                Some(true) => "DRIFT",
                Some(false) => "ok",
                None => "unknown",
            };
            println!(
                "  [{:<8}] {:<6} {:<7} {}",
                p.lane.id(),
                p.risk_tier,
                drift,
                p.summary
            );
        }
        println!(
            "{}",
            crate::update::notifier::notifier_status_line(&notify_home)
        );
        println!("\nNext: `ags update plan` for the full plan; `ags update apply --apply` updates local executable lanes.");
    }
}
pub(in crate::update) fn cmd_update_plan(lane: Option<UpdateLane>, format: &str) {
    let source = source_root_or_exit("ags update plan");
    let home = default_private_runtime_home();
    let lanes: Vec<UpdateLanePlan> = ags_lifecycle::update::plan::select_lanes(
        build_all_update_lanes(&source, &home),
        lane.map(lifecycle_lane),
    );
    if format == "json" {
        let arr: Vec<_> = lanes.iter().map(update_lane_json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update plan",
                "lanes": arr,
                "receipt_outline": "apply / repair-local emit a receipt to <runtime home>/receipts/",
            }))
            .unwrap()
        );
    } else {
        println!("AGS Update Plan (plan-only)");
        for p in &lanes {
            let exec = if p.auto_executes {
                "auto (local)"
            } else {
                "advice-only"
            };
            println!("  → [{}] {} ({})", p.lane.id(), p.summary, exec);
            for c in &p.commands {
                println!("       $ {c}");
            }
            for detail in &p.details {
                let target = detail.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                let status = detail.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let changed = detail
                    .get("changed_files")
                    .and_then(|v| v.as_array())
                    .map(Vec::len)
                    .unwrap_or(0);
                println!("       - {target}: {status}, {changed} file(s)");
            }
        }
        println!("\nNOTE: apply executes core/runtime/projects locally under explicit --apply; agents/skills/public stay advice. Project refresh never commits or pushes. Receipt written on apply.");
    }
}
pub(in crate::update) fn cmd_update_verify(target: Option<PathBuf>, strict: bool, format: &str) {
    let home = update_verify_runtime_home(target);
    let runtime_present = home.is_dir();

    // Registry/auth checks and the machine-local active-skill snapshot are
    // read-only here. A stale snapshot blocks skill routing until explicitly
    // refreshed with `ags capability snapshot --write`.
    let source = source_root_or_exit("ags update verify");
    let capability_authority = capability_authority_root_or_exit("ags update verify");
    let cr_findings = ags_verification::doctor::skill_resolution_drift_check(&capability_authority);
    let auth_boundary_clean = !cr_findings.iter().any(|f| {
        f.check_name == "skill-resolution-auth-boundary"
            && f.status == ags_verification::doctor::CheckStatus::Fail
    });
    let snapshot_path = ags_capability_governance::snapshot_path(&home, "codex");
    let snapshot_present = snapshot_path.is_file();
    let skill_snapshot_current =
        ags_capability_governance::load_validated_snapshot(&capability_authority, &home, "codex")
            .is_ok();
    let project_lane = build_all_update_lanes(&source, &home)
        .into_iter()
        .find(|lane| lane.lane == ags_lifecycle::update::UpdateLane::Projects)
        .expect("projects lane is always present");
    let projects_drift = project_lane.drift.unwrap_or(true);

    let drift = ags_lifecycle::update::plan::VerificationFacts {
        runtime_present,
        auth_boundary_clean,
        skill_snapshot_current,
        projects_drift,
    }
    .drift();

    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update verify",
                "version": AGS_VERSION,
                "runtime_home": home.display().to_string(),
                "runtime_present": runtime_present,
                "drift": drift,
                "projects": update_lane_json(&project_lane),
                "skill_resolver": {
                    "active_host": "codex",
                    "snapshot_path": snapshot_path.display().to_string(),
                    "snapshot_present": snapshot_present,
                    "snapshot_current": skill_snapshot_current,
                    "auth_evidence_boundary_clean": auth_boundary_clean,
                    "refresh_command": "ags capability snapshot --host codex --write",
                },
            }))
            .unwrap()
        );
    } else {
        println!("AGS Update Verify");
        println!("  version: {AGS_VERSION}");
        println!(
            "  runtime home: {} ({})",
            home.display(),
            if runtime_present {
                "present"
            } else {
                "MISSING"
            }
        );
        println!(
            "  projects: {}",
            if projects_drift { "DRIFT" } else { "clean" }
        );
        println!(
            "  skill snapshot: {} auth_boundary={}",
            if skill_snapshot_current {
                "current"
            } else if snapshot_present {
                "STALE"
            } else {
                "MISSING"
            },
            if auth_boundary_clean {
                "clean"
            } else {
                "VIOLATION"
            },
        );
    }
    if strict && drift {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod update_verify_tests {
    use super::update_verify_runtime_home;
    use std::path::PathBuf;

    fn tmp_home(tag: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("ags-update-verify-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn update_verify_target_selects_runtime_home() {
        let explicit = tmp_home("target");
        let home = update_verify_runtime_home(Some(explicit.clone()));
        assert_eq!(home, explicit);
        let _ = std::fs::remove_dir_all(&home);
    }
}
