//! `ags update` lifecycle (五段链路第 5 段) — unified update dispatch.

mod apply;
mod lanes;
mod notifier;
mod plan;
mod repair;
mod rollback;

use crate::cli::UpdateAction;

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
