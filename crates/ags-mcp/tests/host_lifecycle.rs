use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use ags_kernel::evidence::EvidenceLog;
use serde_json::{json, Value};

fn invoke(root: &Path, event: &str, hook_event: &str) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ags-host"))
        .args(["--event", event, "--workspace"])
        .arg(root)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = json!({"hook_event_name": hook_event, "cwd": root});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "ags-host failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn workspace() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("ags.toml"),
        "[workspace]\nslug = \"ags-host-schema-test\"\nrole = \"A\"\n",
    )
    .unwrap();
    tmp
}

#[test]
fn stop_records_evidence_without_decision_or_continuation_feedback() {
    let tmp = workspace();
    assert_eq!(invoke(tmp.path(), "session-end", "Stop"), json!({}));
    let log = EvidenceLog::new(tmp.path().join(".ags/evidence"));
    let events = log.read_all().unwrap();
    EvidenceLog::verify_chain(&events).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "session");
    assert_eq!(events[0].payload["boundary"], "session-end");
}

#[test]
fn unbound_workspace_is_a_valid_noop_for_lifecycle_hooks() {
    let tmp = tempfile::tempdir().unwrap();
    for (event, hook) in [
        ("session-start", "SessionStart"),
        ("session-end", "Stop"),
        ("session-end", "SessionEnd"),
        ("stop-guard", "Stop"),
    ] {
        assert_eq!(invoke(tmp.path(), event, hook), json!({}));
    }
    assert!(!tmp.path().join(".ags").exists());
}

#[test]
fn session_start_emits_only_startup_context() {
    let tmp = workspace();
    let output = invoke(tmp.path(), "session-start", "SessionStart");
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("## AGS Project Memory Context"));
    assert_eq!(
        output,
        json!({"hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": context
        }})
    );
}

#[test]
fn session_end_and_stop_guard_emit_valid_noop_output() {
    let tmp = workspace();
    for (event, hook) in [("session-end", "SessionEnd"), ("stop-guard", "Stop")] {
        assert_eq!(invoke(tmp.path(), event, hook), json!({}));
    }
}
