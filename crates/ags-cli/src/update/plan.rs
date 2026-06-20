use crate::cli::UpdateLane;
use crate::context::{default_private_runtime_home, source_root_or_exit, AGS_VERSION};
use crate::managed_projects;
use crate::update::lanes::{build_all_update_lanes, update_lane_json, UpdateLanePlan};
use std::path::{Path, PathBuf};

/// Runtime home verified by `ags update verify`. An explicit `--target` is the
/// operator's target runtime; without it, fall back to the normal AGS runtime
/// home. Capability Route enrollment reads from this same path so the report is
/// about one runtime, not a mix of default + target state.
fn update_verify_runtime_home(target: Option<PathBuf>) -> PathBuf {
    target.unwrap_or_else(default_private_runtime_home)
}

fn update_verify_enrollment(runtime_home: &Path) -> capability_route::RuntimeEnrollment {
    capability_route::read_enrollment(runtime_home)
}

/// Strict-drift predicate for `ags update verify`. Drift = runtime home missing
/// OR an auth-evidence boundary violation. Enrollment-absent is deliberately NOT
/// an input: a missing machine-local Capability Route enrollment is advisory
/// degraded and must never make `--strict` fail.
fn update_verify_strict_drift(runtime_present: bool, auth_boundary_clean: bool) -> bool {
    !runtime_present || !auth_boundary_clean
}

pub(in crate::update) fn cmd_update_check(format: &str) {
    let source = source_root_or_exit("ags update check");
    let home = default_private_runtime_home();
    let lanes = build_all_update_lanes(&source, &home);
    // Read-only notifier status: reflects stored update-state.json only. NO
    // network probe, NO state write here — due fetch/write belongs to
    // `ags update notify`. Read from the notifier's own runtime home.
    let notify_home = capability_route::locate_runtime_home();
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
        println!("\nNext: `ags update plan` for the full plan; `ags update apply --apply` updates the local kernel.");
    }
}
pub(in crate::update) fn cmd_update_plan(lane: Option<UpdateLane>, format: &str) {
    let source = source_root_or_exit("ags update plan");
    let home = default_private_runtime_home();
    let lanes: Vec<UpdateLanePlan> = build_all_update_lanes(&source, &home)
        .into_iter()
        .filter(|p| lane.map(|l| l == p.lane).unwrap_or(true))
        .collect();
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
        }
        println!("\nNOTE: apply executes only core/runtime locally; agents/projects/public stay advice. Receipt written on apply.");
    }
}
pub(in crate::update) fn cmd_update_verify(target: Option<PathBuf>, strict: bool, format: &str) {
    let home = update_verify_runtime_home(target);
    let runtime_present = home.is_dir();

    // Capability Route drift (read-only): reuse the doctor drift check against the
    // source repo for manifest routing + auth-evidence boundary, plus the
    // machine-local runtime enrollment. Host visibility is not probed here (read-
    // only verify); `ags skill verify --host` is the host-visibility authority.
    let source = source_root_or_exit("ags update verify");
    let cr_findings = suite_doctor::capability_route_drift_check(&source);
    let auth_boundary_clean = !cr_findings.iter().any(|f| {
        f.check_name == "capability-route-auth-boundary"
            && f.status == suite_doctor::CheckStatus::Fail
    });
    let routing_annotated = cr_findings.iter().any(|f| {
        f.check_name == "capability-route-manifest-routing"
            && f.status == suite_doctor::CheckStatus::Pass
    });
    let enrollment = update_verify_enrollment(&home);

    // Strict drift = runtime missing OR an auth-evidence boundary violation.
    // Enrollment-absent is advisory degraded, NOT a strict failure.
    let drift = update_verify_strict_drift(runtime_present, auth_boundary_clean);

    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update verify",
                "version": AGS_VERSION,
                "runtime_home": home.display().to_string(),
                "runtime_present": runtime_present,
                "drift": drift,
                "capability_route": {
                    "runtime_enrollment_present": enrollment.present,
                    "enrollment_mode": enrollment.mode.as_str(),
                    "manifest_routing_annotated": routing_annotated,
                    "auth_evidence_boundary_clean": auth_boundary_clean,
                    "host_visibility": "not probed (read-only verify); use `ags skill verify --host`",
                    "fail_closed": "enrollment absent ⇒ advisory degraded, never a strict failure or a block",
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
            "  capability route: enrollment={} mode={} routing_annotated={} auth_boundary={}",
            if enrollment.present {
                "present"
            } else {
                "absent (advisory)"
            },
            enrollment.mode.as_str(),
            routing_annotated,
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
    use super::{update_verify_enrollment, update_verify_runtime_home, update_verify_strict_drift};
    use capability_route::{enrollment_file_path, render_enrollment_json, EnrollmentMode};
    use std::path::{Path, PathBuf};

    fn tmp_home(tag: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("ags-update-verify-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    fn write_enrollment(home: &Path, mode: EnrollmentMode) {
        let path = enrollment_file_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, render_enrollment_json(mode, "test")).unwrap();
    }

    #[test]
    fn strict_drift_ignores_enrollment_absence() {
        // Healthy runtime + clean auth boundary ⇒ no strict drift. Machine-local
        // enrollment is NOT an input, so its absence can never flip this.
        assert!(!update_verify_strict_drift(true, true));
        // Runtime home missing ⇒ drift (existing contract).
        assert!(update_verify_strict_drift(false, true));
        // Auth-evidence boundary violation ⇒ drift (the one capability-route fail).
        assert!(update_verify_strict_drift(true, false));
        // Both ⇒ drift.
        assert!(update_verify_strict_drift(false, false));
    }

    #[test]
    fn update_verify_target_selects_runtime_home_and_enrollment() {
        let explicit = tmp_home("target");
        write_enrollment(&explicit, EnrollmentMode::Adopted);

        let home = update_verify_runtime_home(Some(explicit.clone()));
        assert_eq!(home, explicit);

        let enrollment = update_verify_enrollment(&home);
        assert!(enrollment.present);
        assert_eq!(enrollment.mode, EnrollmentMode::Adopted);

        let _ = std::fs::remove_dir_all(&home);
    }
}
