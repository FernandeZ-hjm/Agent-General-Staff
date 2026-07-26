//! Thin CLI adapter for the lifecycle-owned update notifier.

use crate::context::AGS_VERSION;
use std::path::Path;

pub(in crate::update) fn notifier_status_json(runtime_home: &Path) -> serde_json::Value {
    ags_lifecycle::update::notifier::notifier_status_json(runtime_home, AGS_VERSION)
}

pub(in crate::update) fn notifier_status_line(runtime_home: &Path) -> String {
    ags_lifecycle::update::notifier::notifier_status_line(runtime_home, AGS_VERSION)
}

pub(in crate::update) fn cmd_update_notify(format: &str) {
    let runtime_home = ags_capability_governance::locate_runtime_home();
    let out = ags_lifecycle::update::notifier::evaluate(&runtime_home, AGS_VERSION);
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&out.to_json()).unwrap_or_else(|_| "{}".to_string())
        );
    } else if out.notify {
        if let Some(message) = out.message {
            println!("{message}");
        }
    } else {
        println!(
            "AGS update notifier: {} (current {}).",
            out.reason, out.current_version
        );
    }
}
