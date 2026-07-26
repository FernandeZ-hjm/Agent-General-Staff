//! Plan-only kernel rollback model.

use std::path::Path;

pub fn kernel_plan(source_root: &Path) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "2.0-rollback",
        "source_root": source_root.to_string_lossy(),
        "rollback_type": "plan-only",
        "applied": false,
        "note": "Rollback plan is read-only. No files are modified. This is a planning stub — real rollback requires human confirmation and explicit task-card authorization.",
        "affected_scope": {
            "protocol_files": "Would revert to last known stable state",
            "scripts": "Would revert to last known stable state",
            "governance": "Would revert skill adoption/ignore lists to last checkpoint",
        },
        "stopped_because": [
            "rollback apply not yet implemented",
            "requires stable/public state synchronization",
            "requires human confirmation",
        ],
        "next_steps": [
            "Review this plan with Codex",
            "Confirm rollback scope with task-card authorization",
            "Run ags release verify --target stable to check current drift",
        ],
    })
}
