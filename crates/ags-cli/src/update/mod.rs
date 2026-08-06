//! Fail-closed Rust fallback for signed AGS core maintenance.
//!
//! Public CLI and MCP packages intercept these commands in the shared verified
//! launcher before Rust starts. A directly invoked kernel may read the cached
//! signed notice, but it never reimplements network, download, pointer, plan or
//! recovery logic.

use crate::cli::UpdateAction;
use crate::context::AGS_VERSION;

pub(crate) fn run(action: UpdateAction) {
    match action {
        UpdateAction::Check { format } => cached_check(&format),
        UpdateAction::Config {
            enabled,
            ignore_version,
            snooze_until_unix,
            format,
        } => configure(
            enabled,
            ignore_version.as_deref(),
            snooze_until_unix,
            &format,
        ),
        UpdateAction::Plan { format } => launcher_required("plan", None, &format),
        UpdateAction::Status { plan_hash, format } => {
            launcher_required("status", Some(&plan_hash), &format)
        }
        UpdateAction::Apply { plan_hash, format } => {
            launcher_required("apply", Some(&plan_hash), &format)
        }
        UpdateAction::Verify { plan_hash, format } => {
            launcher_required("verify", Some(&plan_hash), &format)
        }
        UpdateAction::Recover { format } => launcher_required("recover", None, &format),
    }
}

fn configure(
    enabled: Option<bool>,
    ignore_version: Option<&str>,
    snooze_until_unix: Option<u64>,
    format: &str,
) {
    let root = ags_lifecycle::maintenance::default_update_state_root();
    let result = (|| {
        if let Some(enabled) = enabled {
            ags_lifecycle::maintenance::set_update_checks_enabled(&root, enabled)?;
        }
        if let Some(version) = ignore_version {
            ags_lifecycle::maintenance::ignore_update_version(&root, version)?;
        }
        if let Some(until) = snooze_until_unix {
            ags_lifecycle::maintenance::snooze_update_notices(&root, until)?;
        }
        Ok(ags_lifecycle::maintenance::load_update_state(&root).unwrap_or_default())
    })()
    .unwrap_or_else(|error: String| {
        eprintln!("ags update config: {error}");
        std::process::exit(2);
    });
    crate::output::emit(format, &result, || {
        format!(
            "AGS update notices: enabled={} channel={} ignored={} snoozed_until={:?}",
            result.enabled,
            result.channel,
            result.ignored_versions.len(),
            result.snoozed_until_unix
        )
    });
}

fn cached_check(format: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let notice = ags_lifecycle::maintenance::cached_update_notice(
        &ags_lifecycle::maintenance::default_update_state_root(),
        AGS_VERSION,
        now,
    );
    let output = serde_json::json!({
        "schema_version": "0.4.13-core-update-check",
        "status": "cached",
        "current_version": AGS_VERSION,
        "notice": notice,
        "network_owner": "@agent-governance-suite/launcher",
    });
    crate::output::emit(format, &output, || {
        format!(
            "AGS update check (cached): {:?}. Run through @agent-governance-suite/cli or MCP for a fresh signed check.",
            notice.status
        )
    });
}

fn launcher_required(action: &str, plan_hash: Option<&str>, format: &str) -> ! {
    let output = serde_json::json!({
        "schema_version": "0.4.13-core-update-launcher-required",
        "command": format!("update {action}"),
        "plan_hash": plan_hash,
        "status": "launcher_required",
        "detail": "Signed AGS core maintenance is owned by the shared verified CLI/MCP launcher before Rust starts."
    });
    crate::output::emit(format, &output, || {
        format!(
            "AGS update {action} requires @agent-governance-suite/cli or @agent-governance-suite/mcp."
        )
    });
    std::process::exit(2);
}
