//! Human output adapter for private-runtime rollback planning.

use crate::context::{home_dir, private_install_target};
use std::path::PathBuf;

pub(crate) fn cmd_private_rollback_plan(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags rollback plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let presentation = ags_lifecycle::setup::private_rollback_plan(&target, &home_dir());
    match format {
        "json" => println!("{}", presentation.json),
        _ => println!("{}", presentation.text),
    }
}
