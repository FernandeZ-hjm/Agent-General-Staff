//! Thin CLI adapter for lifecycle-owned update application.

use crate::cli::UpdateLane;
use crate::context::{
    guard_writable_target, home_dir, private_install_target, source_root_or_exit, unix_timestamp,
};
use crate::receipt_bridge::emit_ags_action_receipt;
use crate::update::lanes::{inspect_projects, lifecycle_lane};
use crate::update::plan::cmd_update_plan;
use ags_lifecycle::update::apply::{ApplyRequest, RuntimeUpdate, UpdateEffects};
use std::path::{Path, PathBuf};

struct CliUpdateEffects<'a> {
    source_root: &'a Path,
    runtime_target: &'a Path,
    home: &'a Path,
    force: bool,
    include_optional_extensions: bool,
    backup_stamp: u64,
}

impl UpdateEffects for CliUpdateEffects<'_> {
    fn apply_runtime(&mut self) -> RuntimeUpdate {
        let result =
            ags_lifecycle::setup::apply_private(ags_lifecycle::setup::PrivateApplyRequest {
                source_root: self.source_root,
                target: self.runtime_target,
                home: self.home,
                force: self.force,
                include_optional_extensions: self.include_optional_extensions,
                register_claude: false,
                backup_stamp: self.backup_stamp,
            });
        let report = result.report;
        RuntimeUpdate {
            ok: report.exit_code() == 0,
            exit_code: report.exit_code(),
            human_output: crate::update::render_setup_report(&report),
        }
    }

    fn refresh_projects(&mut self) -> ags_lifecycle::update::ProjectInventory {
        inspect_projects(self.source_root, self.runtime_target, true)
    }
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
    let runtime_target = private_install_target(target.clone());
    guard_writable_target("ags update apply", &runtime_target);
    let home = home_dir();
    let mut effects = CliUpdateEffects {
        source_root: &source,
        runtime_target: &runtime_target,
        home: &home,
        force,
        include_optional_extensions: false,
        backup_stamp: unix_timestamp(),
    };
    let outcome = ags_lifecycle::update::apply::execute(
        &ApplyRequest {
            lane: lane.map(lifecycle_lane),
            source_root: source.clone(),
        },
        &mut effects,
    );

    if format != "json" {
        for step in &outcome.steps {
            println!(
                "  [{}] {} — {}",
                if step.ok { "ok" } else { "FAIL" },
                step.label,
                step.detail
            );
        }
        if let Some(runtime) = &outcome.runtime {
            println!("{}", runtime.human_output);
        }
        for project in outcome
            .projects
            .iter()
            .filter(|project| project.status != "stale")
        {
            let ok = matches!(
                project.status.as_str(),
                "applied" | "clean" | "suite-authority"
            );
            let detail = format!(
                "status={} changed={} unchanged={} blocked={}",
                project.status,
                project.changed_files.len(),
                project.unchanged_files.len(),
                project.blocked_reasons.len()
            );
            println!(
                "  [{}] project {} — {}",
                if ok { "ok" } else { "FAIL" },
                project.target,
                detail
            );
        }
    }

    let receipt = ags_evidence::build_action_receipt(
        "update-apply",
        Some(&runtime_target.display().to_string()),
        ags_evidence::GateResult {
            decision: outcome.decision.to_string(),
            reason: outcome.reason.clone(),
        },
        outcome
            .writes
            .iter()
            .map(|write| ags_evidence::ReceiptWrite {
                op: "refresh".to_string(),
                path: write.path.clone(),
                from: None,
                backup: None,
                detail: write.detail.clone(),
            })
            .collect(),
        vec![],
        outcome
            .advised
            .iter()
            .map(|advised| ags_evidence::ReceiptAdvised {
                command: advised.command.clone(),
                reason: advised.reason.clone(),
            })
            .collect(),
        outcome
            .verifications
            .iter()
            .map(|verification| ags_evidence::VerificationResult {
                command: verification.command.clone(),
                exit_code: verification.exit_code,
                output_hash: ags_evidence::sha256_hex(verification.detail.as_bytes()),
            })
            .collect(),
        ags_evidence::RollbackPlan::backup_restore(vec![]),
        outcome.apply_status,
        outcome.applied,
    );
    let receipt_path = emit_ags_action_receipt(&receipt).ok();
    if format == "json" {
        let mut projects_json = outcome
            .projects
            .iter()
            .map(|project| {
                if project.status == "stale" {
                    serde_json::json!({
                        "target": project.target,
                        "slug": project.slug,
                        "status": project.status,
                        "drift": project.drift,
                        "changed_files": project.changed_files,
                        "blocked_reasons": project.blocked_reasons,
                    })
                } else {
                    serde_json::json!({
                        "target": project.target,
                        "slug": project.slug,
                        "status": project.status,
                        "drift": project.drift,
                        "changed_files": project.changed_files,
                        "unchanged_files": project.unchanged_files,
                        "blocked_reasons": project.blocked_reasons,
                    })
                }
            })
            .collect::<Vec<_>>();
        if let Some(error) = &outcome.project_registry_error {
            projects_json.push(serde_json::json!({
                "status": "blocked",
                "drift": true,
                "changed_files": [],
                "blocked_reasons": [error],
            }));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update apply",
                "apply_status": outcome.apply_status,
                "applied": outcome.applied,
                "executed_local": outcome.executed_local,
                "steps": outcome.steps.iter().map(|step| serde_json::json!({
                    "step": step.label,
                    "ok": step.ok,
                    "detail": step.detail,
                })).collect::<Vec<_>>(),
                "projects": projects_json,
                "receipt_ref": receipt_path.as_ref().map(|path| path.display().to_string()),
                "note": "core/runtime/projects execute locally under --apply; agents/public/skills remain advise-only. Project refresh never commits or pushes.",
            }))
            .unwrap()
        );
    } else if let Some(path) = &receipt_path {
        println!(
            "\n{}",
            ags_evidence::render_action_receipt_summary_line(path)
        );
    }
    if outcome.executed_local && !outcome.all_ok {
        std::process::exit(1);
    }
}
