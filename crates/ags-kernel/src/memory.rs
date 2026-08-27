//! Memory closure (contract v3 §7.9).
//!
//! Memory is verified continuity, never execution authority. Closure verifies
//! the evidence-chain interval for the task and appends one `closure` event;
//! the event id is the memory pointer. AGS never writes user memory,
//! capsules, or any file outside `.ags/evidence`.

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::evidence::{Event, EvidenceLog};

/// Close a task: verify the full evidence chain, require a successful verify
/// event for the task, then append the closure event. A task that was never
/// verified cannot be closed; a task with delegation evidence can only be
/// closed by the recorded owner instance (children RETURN, never CLOSE).
/// Returns the closure event.
pub fn close_task(
    evidence: &EvidenceLog,
    workspace: &str,
    task_card_hash: &str,
    report: Value,
    instance: Option<&str>,
) -> Result<Event> {
    let all = evidence.read_all()?;
    EvidenceLog::verify_chain(&all)?;
    // Owner gate: the task owner is the instance recorded on the first
    // delegation.issue event; without delegation, any caller may close.
    let owner = all
        .iter()
        .find(|e| {
            e.event_type == "delegation.issue"
                && e.task_card_hash.as_deref() == Some(task_card_hash)
        })
        .and_then(|e| e.payload.get("owner_instance").and_then(|v| v.as_str()));
    if let Some(owner) = owner {
        if instance != Some(owner) {
            return Err(Error::new(
                "closure_owner_required",
                format!("task {task_card_hash} is delegated; only instance `{owner}` may close it"),
            ));
        }
    }
    let verified: Vec<String> = all
        .iter()
        .filter(|e| e.task_card_hash.as_deref() == Some(task_card_hash))
        .filter(|e| {
            e.event_type == "test"
                && e.payload.get("all_succeeded").and_then(|v| v.as_bool()) == Some(true)
        })
        .map(|e| e.event_id.clone())
        .collect();
    if verified.is_empty() {
        return Err(Error::new(
            "closure_verify_missing",
            format!("no successful verify evidence found for task {task_card_hash}"),
        ));
    }
    evidence.append(
        "closure",
        workspace,
        Some(task_card_hash),
        "local",
        json!({
            "report": report,
            "closed_event_ids": verified,
        }),
    )
}

/// Memory pointer derived from the closure event — a link, not a new file.
/// `ags log` / `ags status` resolve it against the evidence log.
pub fn memory_pointer(slug: &str, task_card_hash: &str, closure_event_id: &str) -> String {
    format!("ags://memory/{slug}/tasks/{task_card_hash}#{closure_event_id}")
}

/// Byte-preservation boundary: returns the set of files the kernel is ever
/// allowed to write. Everything else — user memory, capsules, host stores —
/// is off-limits; `ags check` enforces this against the projection.
pub fn owned_write_roots(binding: &crate::workspace::WorkspaceBinding) -> Vec<std::path::PathBuf> {
    vec![binding.ags_dir.clone()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_shape_is_stable() {
        assert_eq!(
            memory_pointer("ws", "h1", "ev-abc"),
            "ags://memory/ws/tasks/h1#ev-abc"
        );
    }

    #[test]
    fn closure_requires_task_events_and_chain() {
        let dir = tempfile::tempdir().unwrap();
        let log = EvidenceLog::new(dir.path().join("evidence"));
        let err = close_task(&log, "ws", "unknown", json!({}), None).unwrap_err();
        assert_eq!(err.code, "closure_verify_missing");
        // A task with events but no successful verify cannot be closed.
        log.append("session", "ws", Some("t1"), "local", json!({}))
            .unwrap();
        let err = close_task(&log, "ws", "t1", json!({}), None).unwrap_err();
        assert_eq!(err.code, "closure_verify_missing");
        log.append(
            "test",
            "ws",
            Some("t1"),
            "local",
            json!({"phase": "verify", "all_succeeded": true}),
        )
        .unwrap();
        let event = close_task(&log, "ws", "t1", json!({"status": "succeeded"}), None).unwrap();
        assert_eq!(event.event_type, "closure");
        let all = log.read_all().unwrap();
        EvidenceLog::verify_chain(&all).unwrap();
        assert!(all.iter().any(|e| e.event_type == "closure"));
    }
}
