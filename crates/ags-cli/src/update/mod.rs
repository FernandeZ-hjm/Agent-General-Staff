//! `ags update` lifecycle (五段链路第 5 段) — unified update dispatch.

mod apply;
mod lanes;
mod plan;
mod repair;

use crate::cli::UpdateAction;

pub(in crate::update) fn render_setup_report(report: &ags_lifecycle::setup::SetupReport) -> String {
    ags_verification::doctor::render_text(report)
}

pub(crate) fn run(action: UpdateAction) {
    match action {
        UpdateAction::Check { format } => plan::cmd_update_check(&format),
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
        UpdateAction::RepairLocal {
            target,
            apply,
            force,
            format,
        } => repair::cmd_update_repair_local(target, apply, force, &format),
    }
}
