//! `ags update` lifecycle (五段链路第 5 段) — unified update dispatch.

mod apply;
mod lanes;
mod notifier;
mod plan;
mod repair;
mod rollback;

use crate::cli::UpdateAction;

pub(in crate::update) fn render_setup_report(report: &ags_lifecycle::setup::SetupReport) -> String {
    let mut converted = ags_verification::doctor::HealthReport::new(report.title.clone());
    for finding in &report.findings {
        converted.add(ags_verification::doctor::Finding {
            check_name: finding.check_name.clone(),
            status: match finding.status {
                ags_lifecycle::setup::SetupCheckStatus::Pass => {
                    ags_verification::doctor::CheckStatus::Pass
                }
                ags_lifecycle::setup::SetupCheckStatus::Fail => {
                    ags_verification::doctor::CheckStatus::Fail
                }
                ags_lifecycle::setup::SetupCheckStatus::Warn => {
                    ags_verification::doctor::CheckStatus::Warn
                }
                ags_lifecycle::setup::SetupCheckStatus::Skip => {
                    ags_verification::doctor::CheckStatus::Skip
                }
            },
            severity: match finding.severity {
                ags_lifecycle::setup::SetupSeverity::Info => {
                    ags_verification::doctor::Severity::Info
                }
                ags_lifecycle::setup::SetupSeverity::Warn => {
                    ags_verification::doctor::Severity::Warn
                }
                ags_lifecycle::setup::SetupSeverity::Fail => {
                    ags_verification::doctor::Severity::Fail
                }
            },
            message: finding.message.clone(),
            detail: finding.detail.clone(),
        });
    }
    ags_verification::doctor::render_text(&converted)
}

pub(crate) fn run(action: UpdateAction) {
    match action {
        UpdateAction::Check { format } => plan::cmd_update_check(&format),
        UpdateAction::Notify { format } => notifier::cmd_update_notify(&format),
        UpdateAction::Plan { lane, format } => plan::cmd_update_plan(lane, &format),
        UpdateAction::Apply {
            lane,
            target,
            apply,
            force,
            format,
        } => apply::cmd_update_apply(lane, target, apply, force, &format),
        UpdateAction::Verify {
            target,
            strict,
            format,
        } => plan::cmd_update_verify(target, strict, &format),
        UpdateAction::Rollback {
            scope,
            target,
            format,
        } => rollback::cmd_update_rollback(&scope, target, &format),
        UpdateAction::RepairLocal {
            target,
            apply,
            force,
            format,
        } => repair::cmd_update_repair_local(target, apply, force, &format),
    }
}
