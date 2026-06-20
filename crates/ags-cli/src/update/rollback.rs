use crate::kernel::rollback::cmd_rollback_plan;
use crate::setup::rollback::cmd_private_rollback_plan;
use std::path::PathBuf;

pub(in crate::update) fn cmd_update_rollback(scope: &str, target: Option<PathBuf>, format: &str) {
    match scope {
        "runtime" => cmd_private_rollback_plan("private", target, format),
        _ => cmd_rollback_plan(format),
    }
}
