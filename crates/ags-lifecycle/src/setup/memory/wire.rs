use super::*;
use super::{assets::*, merge::*};

/// Wire the project-memory-capture step into a workspace `settings.json`.
///
/// Reads, merges (preserving existing hooks), backs up the prior file to
/// `.bak.<stamp>` on change, and writes pretty JSON. Returns a diagnostic
/// `Finding`. Never deletes user hooks, and never clobbers the file on
/// unreadable / invalid JSON.
pub(in crate::setup) fn wire_workspace_memory_capture(
    settings_path: &Path,
    command: &str,
    backup_stamp: u64,
) -> crate::setup::SetupFinding {
    let check = "setup-memory-capture-hook";
    let mut value: serde_json::Value = if settings_path.exists() {
        match std::fs::read_to_string(settings_path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    return crate::setup::SetupFinding::fail(
                        check,
                        format!(
                            "{} is not valid JSON — left unchanged",
                            settings_path.display()
                        ),
                        format!("Fix the JSON, then rerun setup. Parse error: {e}"),
                    );
                }
            },
            Err(e) => {
                return crate::setup::SetupFinding::fail(
                    check,
                    format!("cannot read {}", settings_path.display()),
                    e.to_string(),
                );
            }
        }
    } else {
        serde_json::json!({})
    };

    let outcome = merge_memory_capture(&mut value, command);
    if outcome == MergeOutcome::AlreadyPresent {
        return crate::setup::SetupFinding::pass(
            check,
            format!(
                "project memory capture already wired in {} (order: {})",
                settings_path.display(),
                describe_order(&value)
            ),
        );
    }

    if let Some(parent) = settings_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("cannot create {}", parent.display()),
                e.to_string(),
            );
        }
    }
    if settings_path.exists() {
        let backup = settings_path.with_extension(format!("json.bak.{backup_stamp}"));
        if let Err(e) = std::fs::copy(settings_path, &backup) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("backup failed for {}", settings_path.display()),
                e.to_string(),
            );
        }
    }
    let mut serialized = serde_json::to_string_pretty(&value).unwrap_or_default();
    serialized.push('\n');
    if let Err(e) = std::fs::write(settings_path, serialized) {
        return crate::setup::SetupFinding::fail(
            check,
            format!("write failed: {}", settings_path.display()),
            e.to_string(),
        );
    }
    crate::setup::SetupFinding::pass(
        check,
        format!(
            "wired project memory capture into {} (order: {})",
            settings_path.display(),
            describe_order(&value)
        ),
    )
}

/// Wire the read-only project-memory startup injection hook into a workspace
/// `settings.json`. Preserves existing SessionStart hooks and fails closed on
/// unreadable / invalid JSON, matching Stop capture wiring behavior.
pub(in crate::setup) fn wire_workspace_memory_start(
    settings_path: &Path,
    command: &str,
    backup_stamp: u64,
) -> crate::setup::SetupFinding {
    let check = "setup-memory-start-hook";
    let mut value: serde_json::Value = if settings_path.exists() {
        match std::fs::read_to_string(settings_path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    return crate::setup::SetupFinding::fail(
                        check,
                        format!(
                            "{} is not valid JSON — left unchanged",
                            settings_path.display()
                        ),
                        format!("Fix the JSON, then rerun setup. Parse error: {e}"),
                    );
                }
            },
            Err(e) => {
                return crate::setup::SetupFinding::fail(
                    check,
                    format!("cannot read {}", settings_path.display()),
                    e.to_string(),
                );
            }
        }
    } else {
        serde_json::json!({})
    };

    let outcome = merge_memory_start(&mut value, command);
    if outcome == MergeOutcome::AlreadyPresent {
        return crate::setup::SetupFinding::pass(
            check,
            format!(
                "project memory start hook already wired in {}",
                settings_path.display()
            ),
        );
    }

    if let Some(parent) = settings_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("cannot create {}", parent.display()),
                e.to_string(),
            );
        }
    }
    if settings_path.exists() {
        let backup = settings_path.with_extension(format!("json.bak.{backup_stamp}"));
        if let Err(e) = std::fs::copy(settings_path, &backup) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("backup failed for {}", settings_path.display()),
                e.to_string(),
            );
        }
    }
    let mut serialized = serde_json::to_string_pretty(&value).unwrap_or_default();
    serialized.push('\n');
    if let Err(e) = std::fs::write(settings_path, serialized) {
        return crate::setup::SetupFinding::fail(
            check,
            format!("write failed: {}", settings_path.display()),
            e.to_string(),
        );
    }
    crate::setup::SetupFinding::pass(
        check,
        format!(
            "wired project memory start hook into {}",
            settings_path.display()
        ),
    )
}

/// Wire the machine-level Codex SessionStart/SessionEnd lifecycle in one
/// structure-preserving write. Existing hooks and unknown top-level keys are
/// preserved; malformed JSON fails closed.
pub(crate) fn wire_codex_memory_lifecycle(
    hooks_path: &Path,
    backup_stamp: u64,
) -> crate::setup::SetupFinding {
    let check = "agents-codex-memory-lifecycle";
    let mut value: serde_json::Value = if hooks_path.exists() {
        match std::fs::read_to_string(hooks_path)
            .map_err(|e| e.to_string())
            .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
        {
            Ok(value) => value,
            Err(error) => {
                return crate::setup::SetupFinding::fail(
                    check,
                    format!(
                        "{} is unreadable or invalid JSON; left unchanged",
                        hooks_path.display()
                    ),
                    error,
                );
            }
        }
    } else {
        serde_json::json!({})
    };

    let outcome = merge_codex_memory_lifecycle(
        &mut value,
        &memory_start_command(),
        &codex_memory_capture_command(),
    );
    if outcome == MergeOutcome::AlreadyPresent {
        return crate::setup::SetupFinding::pass(
            check,
            format!(
                "Codex memory lifecycle already wired in {}",
                hooks_path.display()
            ),
        );
    }
    if let Some(parent) = hooks_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("cannot create {}", parent.display()),
                error.to_string(),
            );
        }
    }
    if hooks_path.exists() {
        let backup = hooks_path.with_extension(format!("json.bak.{backup_stamp}"));
        if let Err(error) = std::fs::copy(hooks_path, backup) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("backup failed for {}", hooks_path.display()),
                error.to_string(),
            );
        }
    }
    let mut body = serde_json::to_string_pretty(&value).unwrap_or_default();
    body.push('\n');
    match std::fs::write(hooks_path, body) {
        Ok(()) => crate::setup::SetupFinding::pass(
            check,
            format!(
                "wired Codex SessionStart + SessionEnd memory lifecycle in {}",
                hooks_path.display()
            ),
        ),
        Err(error) => crate::setup::SetupFinding::fail(
            check,
            format!("write failed: {}", hooks_path.display()),
            error.to_string(),
        ),
    }
}

pub(super) fn ensure_omp_memory_extension(
    home: &Path,
    backup_stamp: u64,
) -> crate::setup::SetupFinding {
    let check = "agents-omp-memory-lifecycle";
    let path = omp_memory_lifecycle_path(home);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(OMP_MEMORY_LIFECYCLE_JS) {
        return crate::setup::SetupFinding::pass(
            check,
            format!("OMP memory lifecycle extension ready at {}", path.display()),
        );
    }
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("cannot create {}", parent.display()),
                error.to_string(),
            );
        }
    }
    if path.exists() {
        let backup = path.with_extension(format!("js.bak.{backup_stamp}"));
        if let Err(error) = std::fs::copy(&path, backup) {
            return crate::setup::SetupFinding::fail(
                check,
                format!("backup failed for {}", path.display()),
                error.to_string(),
            );
        }
    }
    match std::fs::write(&path, OMP_MEMORY_LIFECYCLE_JS) {
        Ok(()) => crate::setup::SetupFinding::pass(
            check,
            format!(
                "installed OMP native memory lifecycle extension at {}",
                path.display()
            ),
        ),
        Err(error) => crate::setup::SetupFinding::fail(
            check,
            format!("write failed: {}", path.display()),
            error.to_string(),
        ),
    }
}
