//! AGS context-memory product mechanism wiring for `ags setup`.
//!
//! Restores the project-memory capture chain as a first-class product:
//!   - installs the canonical guard/capture scripts to the host script dir,
//!   - merges the project-memory-capture step into the current AGS workspace's
//!     Claude Code Stop pipeline (raw guard → project memory capture), while
//!     preserving every existing hook,
//!   - bootstraps the current workspace's memory capsule via the installed
//!     `context-memory.sh` (create-if-missing; never overwrites the capsule).
//!
//! Command boundary: this lives in `ags setup` (host/workspace bootstrap).
//! `ags init` only creates per-project memory files and never installs a host
//! Stop hook — the installed capture bridge is cwd-aware and resolves each
//! project's memory by repository.

use crate::file_plan::InstallFile;
use std::path::{Path, PathBuf};

/// Canonical capture script bodies, embedded so the installed `ags` binary is
/// self-contained (no dependency on the suite checkout at install time).
pub(in crate::setup) const CONTEXT_MEMORY_SH: &str =
    include_str!("../../../../scripts/context-memory.sh");
pub(in crate::setup) const CLAUDE_STOP_MEMORY_CAPTURE_PY: &str =
    include_str!("../../../../scripts/claude-stop-memory-capture.py");
pub(in crate::setup) const RAW_TOOL_CALL_STOP_GUARD_JS: &str =
    include_str!("../../../../scripts/raw-tool-call-stop-guard.js");

/// Marker substrings used for idempotent, structure-preserving hook detection.
const MEMORY_CAPTURE_MARKER: &str = "claude-stop-memory-capture";
const RAW_GUARD_MARKER: &str = "raw-tool-call-stop-guard";

/// Host directory the capture scripts are installed into (fork decision:
/// `~/.agents/scripts/`, matching the layout the capture bridge already
/// resolves and the existing machine state).
pub(in crate::setup) fn host_scripts_dir(home: &Path) -> PathBuf {
    home.join(".agents").join("scripts")
}
pub(in crate::setup) fn context_memory_script_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("context-memory.sh")
}
pub(in crate::setup) fn claude_stop_memory_capture_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("claude-stop-memory-capture.py")
}
pub(in crate::setup) fn raw_tool_call_stop_guard_path(home: &Path) -> PathBuf {
    host_scripts_dir(home).join("raw-tool-call-stop-guard.js")
}

/// Stop-hook command that runs the project-memory capture bridge. Uses `$HOME`
/// (shell-expanded by the host) so the tracked workspace `settings.json` stays
/// machine-independent.
pub(in crate::setup) fn memory_capture_command() -> String {
    "python3 \"$HOME/.agents/scripts/claude-stop-memory-capture.py\"".to_string()
}
pub(in crate::setup) fn raw_guard_command() -> String {
    "node \"$HOME/.agents/scripts/raw-tool-call-stop-guard.js\"".to_string()
}

/// Install-file entries for the capture scripts. Added to the base install plan
/// so they appear in `ags setup` dry-run output and are written by the standard
/// install loop (which backs up changed files before overwriting).
pub(in crate::setup) fn memory_script_install_files(home: &Path) -> Vec<InstallFile> {
    vec![
        InstallFile {
            path: raw_tool_call_stop_guard_path(home),
            description: "AGS Claude Stop raw tool-call guard".to_string(),
            content: RAW_TOOL_CALL_STOP_GUARD_JS.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: context_memory_script_path(home),
            description: "AGS context-memory product script (status/init/capture)".to_string(),
            content: CONTEXT_MEMORY_SH.to_string(),
            mode: Some(0o755),
        },
        InstallFile {
            path: claude_stop_memory_capture_path(home),
            description: "AGS Claude Stop project-memory capture bridge".to_string(),
            content: CLAUDE_STOP_MEMORY_CAPTURE_PY.to_string(),
            mode: Some(0o755),
        },
    ]
}

/// Outcome of merging the memory-capture step into a Stop pipeline value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::setup) enum MergeOutcome {
    /// A memory-capture command already existed — nothing changed.
    AlreadyPresent,
    /// A memory-capture command was inserted into the pipeline.
    Wired,
}

fn command_str(hook: &serde_json::Value) -> Option<&str> {
    hook.get("command").and_then(|c| c.as_str())
}

fn hook_has_marker(hook: &serde_json::Value, marker: &str) -> bool {
    command_str(hook)
        .map(|c| c.contains(marker))
        .unwrap_or(false)
}

fn group_has_marker(group: &serde_json::Value, marker: &str) -> bool {
    let nested = group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| hooks.iter().any(|h| hook_has_marker(h, marker)))
        .unwrap_or(false);
    let flat = hook_has_marker(group, marker);
    nested || flat
}

fn hooks_contain(groups: &[serde_json::Value], marker: &str) -> bool {
    groups.iter().any(|group| group_has_marker(group, marker))
}

fn memory_hook_entry(command: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": 10,
    })
}

fn raw_guard_hook_entry() -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": raw_guard_command(),
        "timeout": 2,
    })
}

fn insert_raw_guard(stop_arr: &mut Vec<serde_json::Value>) {
    let mut first_nested_group: Option<usize> = None;
    let mut preferred_group: Option<usize> = None;
    for (idx, group) in stop_arr.iter().enumerate() {
        if group.get("hooks").and_then(|h| h.as_array()).is_some() {
            if first_nested_group.is_none() {
                first_nested_group = Some(idx);
            }
            if group_has_marker(group, MEMORY_CAPTURE_MARKER) {
                preferred_group = Some(idx);
                break;
            }
        }
    }

    if let Some(gi) = preferred_group.or(first_nested_group) {
        stop_arr[gi]
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array")
            .insert(0, raw_guard_hook_entry());
    } else {
        stop_arr.insert(0, serde_json::json!({ "hooks": [raw_guard_hook_entry()] }));
    }
}

/// Merge a project-memory-capture step into `value`'s `hooks.Stop` pipeline.
///
/// Ordering rule: setup restores the AGS raw-tool-call guard when missing, then
/// inserts memory capture immediately after that guard. Every existing hook is
/// preserved. Idempotent: if both raw guard and capture command are already
/// present anywhere in the Stop pipeline, the value is left unchanged.
pub(in crate::setup) fn merge_memory_capture(
    value: &mut serde_json::Value,
    command: &str,
) -> MergeOutcome {
    if !value.is_object() {
        *value = serde_json::json!({});
    }
    let root = value.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks_obj = hooks.as_object_mut().expect("hooks object");
    let stop = hooks_obj
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]));
    if !stop.is_array() {
        *stop = serde_json::json!([]);
    }
    let stop_arr = stop.as_array_mut().expect("stop array");

    let mut changed = false;
    if !hooks_contain(stop_arr, RAW_GUARD_MARKER) {
        insert_raw_guard(stop_arr);
        changed = true;
    }

    if hooks_contain(stop_arr, MEMORY_CAPTURE_MARKER) {
        return if changed {
            MergeOutcome::Wired
        } else {
            MergeOutcome::AlreadyPresent
        };
    }

    let entry = memory_hook_entry(command);

    // Insert immediately after the AGS raw guard. This keeps the guard first
    // while avoiding assumptions about any other user-owned Stop hooks.
    let mut raw_guard_slot: Option<(usize, usize)> = None;
    let mut first_nested_group: Option<usize> = None;
    for (idx, group) in stop_arr.iter().enumerate() {
        if let Some(arr) = group.get("hooks").and_then(|h| h.as_array()) {
            if first_nested_group.is_none() {
                first_nested_group = Some(idx);
            }
            if let Some(pos) = arr.iter().position(|h| {
                command_str(h)
                    .map(|c| c.contains(RAW_GUARD_MARKER))
                    .unwrap_or(false)
            }) {
                raw_guard_slot = Some((idx, pos));
                break;
            }
        }
    }

    if let Some((gi, pos)) = raw_guard_slot {
        let arr = stop_arr[gi]
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array");
        arr.insert(pos + 1, entry);
    } else if let Some(gi) = first_nested_group {
        stop_arr[gi]
            .get_mut("hooks")
            .and_then(|h| h.as_array_mut())
            .expect("nested hooks array")
            .push(entry);
    } else {
        stop_arr.push(serde_json::json!({ "hooks": [entry] }));
    }

    MergeOutcome::Wired
}

/// Describe the ordering of the relevant Stop steps for diagnostics.
fn describe_order(value: &serde_json::Value) -> String {
    let mut seen: Vec<&str> = Vec::new();
    if let Some(stop) = value
        .get("hooks")
        .and_then(|h| h.get("Stop"))
        .and_then(|s| s.as_array())
    {
        for group in stop {
            if let Some(arr) = group.get("hooks").and_then(|h| h.as_array()) {
                for h in arr {
                    if let Some(c) = command_str(h) {
                        if c.contains(RAW_GUARD_MARKER) && !seen.contains(&"raw-guard") {
                            seen.push("raw-guard");
                        } else if c.contains(MEMORY_CAPTURE_MARKER)
                            && !seen.contains(&"memory-capture")
                        {
                            seen.push("memory-capture");
                        }
                    }
                }
            }
        }
    }
    seen.join(" → ")
}

/// Wire the project-memory-capture step into a workspace `settings.json`.
///
/// Reads, merges (preserving existing hooks), backs up the prior file to
/// `.bak.<stamp>` on change, and writes pretty JSON. Returns a diagnostic
/// `Finding`. Never deletes user hooks, and never clobbers the file on
/// unreadable / invalid JSON. Called only on the `--register-claude` apply path,
/// so any failure to wire is a blocking `fail` (the operator requested wiring),
/// while leaving the existing file untouched.
pub(in crate::setup) fn wire_workspace_memory_capture(
    settings_path: &Path,
    command: &str,
    backup_stamp: u64,
) -> suite_doctor::Finding {
    let check = "setup-memory-capture-hook";
    let mut value: serde_json::Value = if settings_path.exists() {
        match std::fs::read_to_string(settings_path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    return suite_doctor::Finding::fail(
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
                return suite_doctor::Finding::fail(
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
        return suite_doctor::Finding::pass(
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
            return suite_doctor::Finding::fail(
                check,
                format!("cannot create {}", parent.display()),
                e.to_string(),
            );
        }
    }
    if settings_path.exists() {
        let backup = settings_path.with_extension(format!("json.bak.{backup_stamp}"));
        if let Err(e) = std::fs::copy(settings_path, &backup) {
            return suite_doctor::Finding::fail(
                check,
                format!("backup failed for {}", settings_path.display()),
                e.to_string(),
            );
        }
    }
    let mut serialized = serde_json::to_string_pretty(&value).unwrap_or_default();
    serialized.push('\n');
    if let Err(e) = std::fs::write(settings_path, serialized) {
        return suite_doctor::Finding::fail(
            check,
            format!("write failed: {}", settings_path.display()),
            e.to_string(),
        );
    }
    suite_doctor::Finding::pass(
        check,
        format!(
            "wired project memory capture into {} (order: {})",
            settings_path.display(),
            describe_order(&value)
        ),
    )
}

/// Bootstrap the current workspace's memory capsule by invoking the installed
/// `context-memory.sh init`. Create-if-missing; the script never overwrites the
/// capsule. Fail-closed on the `--register-claude` apply path: a missing script
/// or shell failure is a blocking `fail` (the operator asked to wire the chain),
/// not an advisory warn. `memory_root` overrides the default store (tests only).
pub(in crate::setup) fn bootstrap_workspace_memory_with(
    script_path: &Path,
    workspace_root: &Path,
    memory_root: Option<&Path>,
) -> suite_doctor::Finding {
    let check = "setup-memory-capsule-bootstrap";
    if !script_path.is_file() {
        return suite_doctor::Finding::fail(
            check,
            "context-memory.sh not installed — capsule bootstrap skipped",
            format!("expected installed script at {}", script_path.display()),
        );
    }
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(script_path)
        .arg("init")
        .arg("--repo")
        .arg(workspace_root);
    if let Some(root) = memory_root {
        cmd.env("MEMORY_ROOT", root);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => suite_doctor::Finding::pass(
            check,
            format!(
                "workspace memory capsule ready for {} (capsule never overwritten)",
                workspace_root.display()
            ),
        ),
        Ok(out) => suite_doctor::Finding::fail(
            check,
            "context-memory.sh init reported a problem",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => suite_doctor::Finding::fail(
            check,
            "could not run context-memory.sh init",
            e.to_string(),
        ),
    }
}

/// Register-claude apply step: wire the workspace Stop pipeline and bootstrap
/// the workspace memory capsule. `home` resolves the installed script path;
/// `workspace_root` is the current AGS suite/workspace whose `.claude` config
/// and memory are bootstrapped.
pub(in crate::setup) fn add_workspace_memory_capture(
    report: &mut suite_doctor::HealthReport,
    home: &Path,
    workspace_root: &Path,
    backup_stamp: u64,
) {
    add_workspace_memory_capture_inner(report, home, workspace_root, backup_stamp, None);
}

fn add_workspace_memory_capture_inner(
    report: &mut suite_doctor::HealthReport,
    home: &Path,
    workspace_root: &Path,
    backup_stamp: u64,
    memory_root: Option<&Path>,
) {
    let settings_path = workspace_root.join(".claude").join("settings.json");
    report.add(wire_workspace_memory_capture(
        &settings_path,
        &memory_capture_command(),
        backup_stamp,
    ));
    let script_path = context_memory_script_path(home);
    report.add(bootstrap_workspace_memory_with(
        &script_path,
        workspace_root,
        memory_root,
    ));
}

/// Read-only preview of what `ags setup --yes --register-claude` will do to the
/// workspace memory-capture chain. Rendered in the setup plan / dry-run so the
/// operator can see the hook install/repair before applying.
pub(in crate::setup) fn render_memory_capture_plan(
    home: &Path,
    workspace_root: &Path,
    register_claude: bool,
) -> String {
    let settings_path = workspace_root.join(".claude").join("settings.json");
    let (raw_wired, memory_wired) = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .map(|v| {
            v.get("hooks")
                .and_then(|h| h.get("Stop"))
                .and_then(|s| s.as_array())
                .map(|stop| {
                    (
                        hooks_contain(stop, RAW_GUARD_MARKER),
                        hooks_contain(stop, MEMORY_CAPTURE_MARKER),
                    )
                })
                .unwrap_or((false, false))
        })
        .unwrap_or((false, false));

    let mut lines = vec!["Memory capture chain (project memory):".to_string()];
    lines.push(format!(
        "  - Scripts: {} , {} , {}",
        raw_tool_call_stop_guard_path(home).display(),
        context_memory_script_path(home).display(),
        claude_stop_memory_capture_path(home).display()
    ));
    lines.push(format!(
        "  - Workspace Stop config: {}",
        settings_path.display()
    ));
    lines.push(format!(
        "  - Current state: raw guard {}",
        if raw_wired { "WIRED" } else { "MISSING" }
    ));
    lines.push(format!(
        "  - Current state: project memory capture {}",
        if memory_wired { "WIRED" } else { "MISSING" }
    ));
    if register_claude {
        if raw_wired && memory_wired {
            lines.push(
                "  - Action: scripts refreshed; Stop pipeline already wired (idempotent)."
                    .to_string(),
            );
        } else {
            lines.push(
                "  - Action: install scripts + repair Stop pipeline (raw guard → project memory capture), backing up the prior settings.json."
                    .to_string(),
            );
        }
        lines.push(
            "  - Capsule: bootstrapped via context-memory.sh init (create-if-missing; never overwrites context-capsule.md)."
                .to_string(),
        );
    } else {
        lines.push(
            "  - Action: pass --register-claude to install scripts and wire/repair the Stop pipeline + bootstrap the workspace capsule."
                .to_string(),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("ags-setup-memory-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn raw_existing_settings() -> serde_json::Value {
        serde_json::json!({
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            { "type": "command", "command": "node /x/raw-tool-call-stop-guard.js", "timeout": 2 },
                            { "type": "command", "command": "node /x/user-stop-hook.js", "timeout": 8 }
                        ]
                    }
                ]
            },
            "_keep": true
        })
    }

    fn user_only_settings() -> serde_json::Value {
        serde_json::json!({
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            { "type": "command", "command": "node /x/user-stop-hook.js", "timeout": 8 }
                        ]
                    }
                ]
            },
            "_keep": true
        })
    }

    fn commands(value: &serde_json::Value) -> Vec<String> {
        value["hooks"]["Stop"][0]["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["command"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn merge_inserts_memory_after_raw_guard() {
        let mut v = raw_existing_settings();
        let outcome = merge_memory_capture(&mut v, &memory_capture_command());
        assert_eq!(outcome, MergeOutcome::Wired);
        let cmds = commands(&v);
        assert_eq!(cmds.len(), 3);
        assert!(cmds[0].contains(RAW_GUARD_MARKER));
        assert!(cmds[1].contains(MEMORY_CAPTURE_MARKER));
        assert!(cmds[2].contains("user-stop-hook.js"));
        // Unknown top-level keys preserved.
        assert_eq!(v["_keep"], serde_json::json!(true));
        assert_eq!(describe_order(&v), "raw-guard → memory-capture");
    }

    #[test]
    fn merge_is_idempotent() {
        let mut v = raw_existing_settings();
        assert_eq!(
            merge_memory_capture(&mut v, &memory_capture_command()),
            MergeOutcome::Wired
        );
        assert_eq!(
            merge_memory_capture(&mut v, &memory_capture_command()),
            MergeOutcome::AlreadyPresent
        );
        let count = commands(&v)
            .iter()
            .filter(|c| c.contains(MEMORY_CAPTURE_MARKER))
            .count();
        assert_eq!(count, 1, "no duplicate capture step");
    }

    #[test]
    fn merge_preserves_unknown_hooks() {
        let mut v = serde_json::json!({
            "hooks": { "Stop": [ { "hooks": [
                { "type": "command", "command": "node /x/raw-tool-call-stop-guard.js" },
                { "type": "command", "command": "node /x/user-custom-hook.js" },
                { "type": "command", "command": "node /x/user-late-hook.js" }
            ] } ] }
        });
        merge_memory_capture(&mut v, &memory_capture_command());
        let cmds = commands(&v);
        assert!(cmds.iter().any(|c| c.contains("user-custom-hook.js")));
        assert!(cmds.iter().any(|c| c.contains("user-late-hook.js")));
        // capture lands immediately after the AGS raw guard.
        let raw = cmds
            .iter()
            .position(|c| c.contains(RAW_GUARD_MARKER))
            .unwrap();
        let mem = cmds
            .iter()
            .position(|c| c.contains(MEMORY_CAPTURE_MARKER))
            .unwrap();
        assert_eq!(mem, raw + 1);
    }

    #[test]
    fn merge_restores_raw_guard_when_absent() {
        let mut v = user_only_settings();
        merge_memory_capture(&mut v, &memory_capture_command());
        let cmds = commands(&v);
        assert_eq!(cmds.len(), 3);
        assert!(cmds[0].contains(RAW_GUARD_MARKER));
        assert!(cmds[1].contains(MEMORY_CAPTURE_MARKER));
        assert!(cmds[2].contains("user-stop-hook.js"));
        assert_eq!(describe_order(&v), "raw-guard → memory-capture");
    }

    #[test]
    fn merge_creates_stop_when_absent() {
        let mut v = serde_json::json!({});
        merge_memory_capture(&mut v, &memory_capture_command());
        assert!(hooks_contain(
            v["hooks"]["Stop"].as_array().unwrap(),
            MEMORY_CAPTURE_MARKER
        ));
    }

    #[test]
    fn install_files_target_agents_scripts_with_mode() {
        let home = tmp("install-files");
        let files = memory_script_install_files(&home);
        assert_eq!(files.len(), 3);
        assert!(files[0]
            .path
            .ends_with(".agents/scripts/raw-tool-call-stop-guard.js"));
        assert!(files[1].path.ends_with(".agents/scripts/context-memory.sh"));
        assert!(files[2]
            .path
            .ends_with(".agents/scripts/claude-stop-memory-capture.py"));
        assert_eq!(files[0].mode, Some(0o755));
        assert_eq!(files[1].mode, Some(0o755));
        assert_eq!(files[2].mode, Some(0o755));
        assert!(files[0].content.contains("hasRawToolCallLeak"));
        assert!(files[1].content.contains("context-memory.sh"));
        assert!(files[2].content.contains("claude-stop-memory-capture"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn wire_writes_backs_up_then_is_idempotent() {
        let dir = tmp("wire");
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let settings = claude.join("settings.json");
        std::fs::write(
            &settings,
            serde_json::to_string_pretty(&raw_existing_settings()).unwrap(),
        )
        .unwrap();

        let f1 = wire_workspace_memory_capture(&settings, &memory_capture_command(), 1234);
        assert_eq!(f1.status, suite_doctor::CheckStatus::Pass);
        assert!(f1.message.contains("wired"));
        // backup written
        assert!(claude.join("settings.json.bak.1234").exists());
        // resulting file has the ordered pipeline
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(describe_order(&written), "raw-guard → memory-capture");

        // second run is idempotent: unchanged, no new write needed
        let f2 = wire_workspace_memory_capture(&settings, &memory_capture_command(), 5678);
        assert_eq!(f2.status, suite_doctor::CheckStatus::Pass);
        assert!(f2.message.contains("already wired"));
        assert!(!claude.join("settings.json.bak.5678").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Adversarial-review fix: on the --register-claude apply path, invalid
    /// settings.json must FAIL (block setup) — not warn — while never clobbering
    /// the user's file.
    #[test]
    fn wire_fails_on_invalid_json_without_clobbering() {
        let dir = tmp("wire-bad");
        let claude = dir.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let settings = claude.join("settings.json");
        std::fs::write(&settings, "{ not json").unwrap();
        let f = wire_workspace_memory_capture(&settings, &memory_capture_command(), 1);
        assert_eq!(f.status, suite_doctor::CheckStatus::Fail);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// G8: `context-memory.sh capture` archives a receipt and refreshes
    /// task-memory.md but NEVER overwrites the manual context-capsule.md.
    #[cfg(unix)]
    #[test]
    fn context_memory_capture_preserves_capsule() {
        let base = tmp("capture");
        let scripts = base.join("scripts");
        let repo = base.join("repo");
        let mroot = base.join("memory/projects");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::create_dir_all(repo.join("config")).unwrap();
        std::fs::create_dir_all(&mroot).unwrap();

        let script = scripts.join("context-memory.sh");
        std::fs::write(&script, CONTEXT_MEMORY_SH).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&script).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&script, p).unwrap();
        }
        std::fs::write(
            repo.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: capture-fixture\n",
        )
        .unwrap();

        let run = |args: &[&str]| {
            std::process::Command::new("bash")
                .arg(&script)
                .args(args)
                .arg("--repo")
                .arg(&repo)
                .arg("--memory-root")
                .arg(&mroot)
                .output()
                .expect("run context-memory.sh")
        };

        // init creates the capsule
        let init = run(&["init"]);
        assert!(
            init.status.success(),
            "init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let capsule = mroot.join("capture-fixture/context-capsule.md");
        assert!(capsule.exists());

        // simulate human edit of the manual design-purpose block
        std::fs::write(&capsule, "## 项目设计目的\nHUMAN-ONLY-SENTINEL\n").unwrap();

        // build a fake receipt and capture it
        let receipt = base.join("receipt");
        std::fs::create_dir_all(&receipt).unwrap();
        std::fs::write(receipt.join("task-card.md"), "任务：\nfixture capture\n").unwrap();
        std::fs::write(
            receipt.join("delivery-report.md"),
            "# 任务交付报告\n\n## 任务状态\n完成\n\n一句话结论：ok\n",
        )
        .unwrap();
        let cap = run(&["capture", receipt.to_str().unwrap()]);
        assert!(
            cap.status.success(),
            "capture failed: {}",
            String::from_utf8_lossy(&cap.stderr)
        );

        // capsule must be byte-identical (NOT overwritten)
        assert_eq!(
            std::fs::read_to_string(&capsule).unwrap(),
            "## 项目设计目的\nHUMAN-ONLY-SENTINEL\n"
        );
        // task-memory.md refreshed from the archive
        let task_memory =
            std::fs::read_to_string(mroot.join("capture-fixture/task-memory.md")).unwrap();
        assert!(task_memory.contains("fixture capture"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Bootstrap is create-if-missing and never overwrites an existing capsule.
    #[cfg(unix)]
    #[test]
    fn bootstrap_creates_then_preserves_capsule() {
        let base = tmp("bootstrap");
        let scripts = base.join("scripts");
        let repo = base.join("repo");
        let mroot = base.join("memory/projects");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::create_dir_all(repo.join("config")).unwrap();
        let script = scripts.join("context-memory.sh");
        std::fs::write(&script, CONTEXT_MEMORY_SH).unwrap();
        std::fs::write(
            repo.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: boot-fixture\n",
        )
        .unwrap();

        let f1 = bootstrap_workspace_memory_with(&script, &repo, Some(&mroot));
        assert_eq!(f1.status, suite_doctor::CheckStatus::Pass);
        let capsule = mroot.join("boot-fixture/context-capsule.md");
        assert!(capsule.exists());

        std::fs::write(&capsule, "## 项目设计目的\nKEEP-ME\n").unwrap();
        let f2 = bootstrap_workspace_memory_with(&script, &repo, Some(&mroot));
        assert_eq!(f2.status, suite_doctor::CheckStatus::Pass);
        assert_eq!(
            std::fs::read_to_string(&capsule).unwrap(),
            "## 项目设计目的\nKEEP-ME\n"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Adversarial-review fix: a missing capture script on the apply path is a
    /// blocking FAIL, so setup cannot report success with the chain unwired.
    #[test]
    fn bootstrap_fails_when_script_missing() {
        let base = tmp("noscript");
        let f =
            bootstrap_workspace_memory_with(&base.join("nope.sh"), &base, Some(&base.join("m")));
        assert_eq!(f.status, suite_doctor::CheckStatus::Fail);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Adversarial-review fix (finding 2): the register-claude apply path fails
    /// CLOSED — an invalid workspace settings.json yields a non-passing report,
    /// so `ags setup --yes --register-claude` exits non-zero instead of emitting
    /// an "applied" receipt with the memory chain unwired.
    #[cfg(unix)]
    #[test]
    fn register_claude_apply_fails_closed_on_invalid_settings() {
        let base = tmp("failclosed");
        let home = base.join("home");
        let workspace = base.join("ws");
        std::fs::create_dir_all(host_scripts_dir(&home)).unwrap();
        std::fs::create_dir_all(workspace.join(".claude")).unwrap();
        std::fs::create_dir_all(workspace.join("config")).unwrap();
        // capture script installed so bootstrap could run…
        std::fs::write(context_memory_script_path(&home), CONTEXT_MEMORY_SH).unwrap();
        std::fs::write(
            workspace.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: failclosed-fixture\n",
        )
        .unwrap();
        // …but settings.json is broken → wiring must fail closed.
        std::fs::write(workspace.join(".claude/settings.json"), "{ broken").unwrap();

        let mut report = suite_doctor::HealthReport::new("t");
        add_workspace_memory_capture_inner(
            &mut report,
            &home,
            &workspace,
            7,
            Some(&base.join("mem")),
        );
        assert!(
            !report.passed(),
            "invalid settings.json must make the register-claude apply fail closed"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Happy-path counterpart: valid settings → report passes and the capture
    /// step is wired in order.
    #[cfg(unix)]
    #[test]
    fn register_claude_apply_wires_and_passes_on_valid_settings() {
        let base = tmp("passes");
        let home = base.join("home");
        let workspace = base.join("ws");
        std::fs::create_dir_all(host_scripts_dir(&home)).unwrap();
        std::fs::create_dir_all(workspace.join(".claude")).unwrap();
        std::fs::create_dir_all(workspace.join("config")).unwrap();
        std::fs::write(context_memory_script_path(&home), CONTEXT_MEMORY_SH).unwrap();
        std::fs::write(
            workspace.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: passes-fixture\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join(".claude/settings.json"),
            serde_json::to_string_pretty(&raw_existing_settings()).unwrap(),
        )
        .unwrap();

        let mut report = suite_doctor::HealthReport::new("t");
        add_workspace_memory_capture_inner(
            &mut report,
            &home,
            &workspace,
            7,
            Some(&base.join("mem")),
        );
        assert!(
            report.passed(),
            "valid settings must wire + bootstrap cleanly"
        );
        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(workspace.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(describe_order(&written), "raw-guard → memory-capture");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn register_claude_apply_repairs_missing_raw_guard() {
        let base = tmp("repairs");
        let home = base.join("home");
        let workspace = base.join("ws");
        std::fs::create_dir_all(host_scripts_dir(&home)).unwrap();
        std::fs::create_dir_all(workspace.join(".claude")).unwrap();
        std::fs::create_dir_all(workspace.join("config")).unwrap();
        std::fs::write(context_memory_script_path(&home), CONTEXT_MEMORY_SH).unwrap();
        std::fs::write(
            workspace.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: repairs-fixture\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join(".claude/settings.json"),
            serde_json::to_string_pretty(&user_only_settings()).unwrap(),
        )
        .unwrap();

        let mut report = suite_doctor::HealthReport::new("t");
        add_workspace_memory_capture_inner(
            &mut report,
            &home,
            &workspace,
            7,
            Some(&base.join("mem")),
        );
        assert!(
            report.passed(),
            "settings without raw guard must be repaired by register-claude apply"
        );
        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(workspace.join(".claude/settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(describe_order(&written), "raw-guard → memory-capture");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Adversarial-review fix (finding 1): the Stop bridge must never pair a new
    /// task card with a stale delivery report from an earlier task. Drives the
    /// real `claude-stop-memory-capture.py` end-to-end over a crafted transcript.
    #[cfg(unix)]
    #[test]
    fn capture_does_not_pair_new_card_with_stale_report() {
        let base = tmp("pairing");
        let scripts = base.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let ctx = scripts.join("context-memory.sh");
        let py = scripts.join("claude-stop-memory-capture.py");
        std::fs::write(&ctx, CONTEXT_MEMORY_SH).unwrap();
        std::fs::write(&py, CLAUDE_STOP_MEMORY_CAPTURE_PY).unwrap();

        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join("config")).unwrap();
        std::fs::write(
            repo.join("config/agent-project-profile.yaml"),
            "schema_version: 1\nproject:\n  slug: pairing-fixture\n",
        )
        .unwrap();

        let card = |name: &str| {
            format!(
                "## 任务卡\n\n任务：\n{name}\nExecutor: Claude Code\nRuntime adapter: claude-code\nVerification gate:\n- 验证\n"
            )
        };
        let report_txt =
            |c: &str| format!("# 任务交付报告\n\n## 任务状态\n完成\n\n一句话结论：{c}\n");
        let line = |role: &str, text: &str| {
            serde_json::json!({ "message": { "role": role, "content": text } }).to_string()
        };

        let transcript = base.join("t.jsonl");
        // OLD card + OLD report, then a NEW card with NO following report.
        std::fs::write(
            &transcript,
            format!(
                "{}\n{}\n{}\n",
                line("user", &card("OLD-TASK")),
                line("assistant", &report_txt("old done")),
                line("user", &card("NEW-TASK")),
            ),
        )
        .unwrap();

        let memroot = base.join("mem");
        let hook_input = serde_json::json!({
            "hook_event_name": "Stop",
            "transcript_path": transcript.to_str().unwrap(),
            "cwd": repo.to_str().unwrap(),
        })
        .to_string();

        let run = |stdin: &str| {
            use std::io::Write;
            use std::process::{Command, Stdio};
            let mut child = Command::new("python3")
                .arg(&py)
                .env("AGENT_CONTEXT_MEMORY_SH", &ctx)
                .env("MEMORY_ROOT", &memroot)
                .env("CLAUDE_STOP_MEMORY_RECEIPT_ROOT", base.join("receipts"))
                .env("CLAUDE_STOP_MEMORY_STATE_DIR", base.join("state"))
                .env("CLAUDE_STOP_MEMORY_LOG_DIR", base.join("logs"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn python3");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(stdin.as_bytes())
                .unwrap();
            child.wait_with_output().expect("python3 output")
        };

        let task_memory = memroot.join("pairing-fixture/task-memory.md");

        // New card, no following report → NOTHING captured (no stale pairing).
        let out = run(&hook_input);
        assert!(out.status.success());
        assert!(
            !task_memory.exists(),
            "a new unfinished card must not be paired with a stale report"
        );

        // Append the NEW report → now the NEW card pairs with the NEW report.
        let prev = std::fs::read_to_string(&transcript).unwrap();
        std::fs::write(
            &transcript,
            format!("{prev}{}\n", line("assistant", &report_txt("new done"))),
        )
        .unwrap();
        let out2 = run(&hook_input);
        assert!(out2.status.success());
        assert!(
            task_memory.exists(),
            "new card + following report is captured"
        );
        let tm = std::fs::read_to_string(&task_memory).unwrap();
        assert!(tm.contains("NEW-TASK"), "captured the NEW task; got: {tm}");
        assert!(!tm.contains("OLD-TASK"), "must not capture the OLD task");
        let _ = std::fs::remove_dir_all(&base);
    }
}
