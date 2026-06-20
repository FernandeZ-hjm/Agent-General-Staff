//! Private AGS runtime rollback planning (plan-only). Lives in the `setup`
//! lifecycle because it reasons about the private-install payload; the generic
//! rollback stub stays in `kernel::rollback`.

use super::{claude_ags_command_path, PRIVATE_INSTALL_SCHEMA};
use crate::context::private_install_target;
use std::path::{Path, PathBuf};

pub(crate) fn cmd_private_rollback_plan(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags rollback plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
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
    let claude_command_path = claude_ags_command_path();
    entries.push(serde_json::json!({
        "path": claude_command_path.to_string_lossy(),
        "exists": claude_command_path.exists(),
        "backup_candidates": backup_candidates(&claude_command_path),
    }));

    let plan = serde_json::json!({
        "schema_version": PRIVATE_INSTALL_SCHEMA,
        "profile": "private",
        "target": target.to_string_lossy(),
        "rollback_type": "plan-only",
        "applied": false,
        "note": "Rollback apply is intentionally not implemented. Review backup candidates and remove or restore files manually with explicit authorization.",
        "files": entries,
    });

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&plan).unwrap_or_default()
        ),
        _ => {
            println!("AGS Private Runtime Rollback Plan");
            println!("=================================");
            println!("Schema:  {}", PRIVATE_INSTALL_SCHEMA);
            println!("Profile: private");
            println!("Target:  {}", target.display());
            println!("Applied: false");
            println!();
            println!("Files:");
            if let Some(files) = plan["files"].as_array() {
                for file in files {
                    println!(
                        "  - {} (exists: {})",
                        file["path"].as_str().unwrap_or("?"),
                        file["exists"]
                    );
                    if let Some(backups) = file["backup_candidates"].as_array() {
                        for backup in backups {
                            println!("      backup: {}", backup.as_str().unwrap_or("?"));
                        }
                    }
                }
            }
            println!();
            println!("Verdict: PLAN-ONLY — no files modified.");
        }
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
