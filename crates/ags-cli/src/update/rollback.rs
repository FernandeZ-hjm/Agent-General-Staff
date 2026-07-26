//! Thin CLI adapter for lifecycle-owned rollback plans.

use crate::context::{home_dir, private_install_target};
use crate::kernel::rollback::cmd_rollback_plan;
use std::path::PathBuf;

pub(in crate::update) fn cmd_update_rollback(scope: &str, target: Option<PathBuf>, format: &str) {
    if scope != "runtime" {
        cmd_rollback_plan(format);
        return;
    }
    let presentation =
        ags_lifecycle::setup::private_rollback_plan(&private_install_target(target), &home_dir());
    if format == "json" {
        println!("{}", presentation.json);
    } else {
        println!("{}", presentation.text);
    }
}
