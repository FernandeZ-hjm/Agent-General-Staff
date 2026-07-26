//! Private AGS runtime rollback planning (plan-only). Lives in the `setup`
//! lifecycle because it reasons about the private-install payload; the generic
//! rollback stub stays in `kernel::rollback`.

use super::memory::{claude_stop_memory_capture_path, context_memory_script_path};
use super::{claude_ags_command_path, PRIVATE_INSTALL_SCHEMA};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PrivateRollbackPresentation {
    pub json: String,
    pub text: String,
}

pub(in crate::setup) fn private_rollback_plan(
    target: &Path,
    home: &Path,
) -> PrivateRollbackPresentation {
    let files = [
        "install-manifest.json",
        "README.md",
        "mcp/ags.mcp.json",
        "hosts/codex.config.snippet.toml",
        "hosts/claude-code.mcp.snippet.json",
        "hosts/tencent-agent.mcp.snippet.json",
        "hosts/workbuddy.mcp.snippet.json",
        "hosts/codebuddy-code.mcp.snippet.json",
        "manifests/runtime-profiles.yaml",
        "hooks/claude-code-executor-stop.js",
        "hooks/codex-planner-recall.json",
        "bin/ags-mcp-stdio.sh",
        "secrets/README.md",
    ];
    let mut entries: Vec<_> = files
        .iter()
        .map(|rel| {
            let path = target.join(rel);
            serde_json::json!({
                "path": path.to_string_lossy(),
                "exists": path.exists(),
                "backup_candidates": backup_candidates(&path),
            })
        })
        .collect();
    let claude_command_path = claude_ags_command_path(home);
    entries.push(serde_json::json!({
        "path": claude_command_path.to_string_lossy(),
        "exists": claude_command_path.exists(),
        "backup_candidates": backup_candidates(&claude_command_path),
    }));
    // Project-memory capture scripts (installed under $HOME/.agents/scripts/).
    for script in [
        context_memory_script_path(home),
        claude_stop_memory_capture_path(home),
    ] {
        entries.push(serde_json::json!({
            "path": script.to_string_lossy(),
            "exists": script.exists(),
            "backup_candidates": backup_candidates(&script),
        }));
    }

    let plan = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": "private",
        "target": target.to_string_lossy(),
        "rollback_type": "plan-only",
        "applied": false,
        "note": "Rollback apply is intentionally not implemented. Review backup candidates and remove or restore files manually with explicit authorization.",
        "files": entries,
    });

    let mut lines = vec![
        "AGS Private Runtime Rollback Plan".to_string(),
        "=================================".to_string(),
        format!("Schema:  {PRIVATE_INSTALL_SCHEMA}"),
        "Profile: private".to_string(),
        format!("Target:  {}", target.display()),
        "Applied: false".to_string(),
        String::new(),
        "Files:".to_string(),
    ];
    if let Some(files) = plan["files"].as_array() {
        for file in files {
            lines.push(format!(
                "  - {} (exists: {})",
                file["path"].as_str().unwrap_or("?"),
                file["exists"]
            ));
            if let Some(backups) = file["backup_candidates"].as_array() {
                for backup in backups {
                    lines.push(format!("      backup: {}", backup.as_str().unwrap_or("?")));
                }
            }
        }
    }
    lines.push(String::new());
    lines.push("Verdict: PLAN-ONLY — no files modified.".to_string());
    PrivateRollbackPresentation {
        json: serde_json::to_string_pretty(&plan).unwrap_or_default(),
        text: lines.join("\n"),
    }
}
fn backup_candidates(path: &Path) -> Vec<String> {
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{file_name}.");
    let mut backups = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.contains(".bak.") {
                backups.push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    backups.sort();
    backups
}
