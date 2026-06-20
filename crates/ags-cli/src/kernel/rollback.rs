use std::path::PathBuf;

/// Shared dispatch: `rollback plan`
pub(crate) fn cmd_rollback_plan(format: &str) {
    let source_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let plan = serde_json::json!({
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
    });

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
        }
        _ => {
            println!("Rollback Plan");
            println!("=============");
            println!("Schema:        {}", plan["schema_version"]);
            println!("Source:        {}", plan["source_root"]);
            println!("Type:          {}", plan["rollback_type"]);
            println!("Applied:       {}", plan["applied"]);
            println!();
            println!("Note: {}", plan["note"]);
            println!();
            println!("Affected scope:");
            if let Some(scope) = plan["affected_scope"].as_object() {
                for (k, v) in scope {
                    println!("  {}: {}", k, v);
                }
            }
            println!();
            println!("Stopped because:");
            if let Some(reasons) = plan["stopped_because"].as_array() {
                for r in reasons {
                    println!("  - {}", r.as_str().unwrap_or("?"));
                }
            }
            println!();
            println!("Next steps:");
            if let Some(steps) = plan["next_steps"].as_array() {
                for s in steps {
                    println!("  - {}", s.as_str().unwrap_or("?"));
                }
            }
            println!();
            println!("Verdict: PLAN-ONLY — no rollback applied. Human confirmation required.");
        }
    }
}
