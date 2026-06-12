//! AGS MCP Resources — governance protocol documents and boundary references.
//!
//! Resources expose AGS protocol files and governance reference material
//! as MCP resources. Each resource has a stable URI and returns markdown
//! text content.

use crate::protocol::{ResourceContent, ResourceDef, ResourceListResult, ResourceReadResult};

// ── Resource Definitions ─────────────────────────────────────────────────────

/// Generate MCP `resources/list` response with all available resources.
pub fn list_resources() -> ResourceListResult {
    ResourceListResult {
        resources: vec![
            ResourceDef {
                uri: "ags://global-kernel".to_string(),
                name: "AGS Global Kernel".to_string(),
                description: Some(
                    "AGS global governance kernel summary — lifecycle, rules, and EvoMap boundary. \
                     Applicable to any project; does NOT assume the target is an AGS repo. \
                     Includes: ambient preflight → solution formation → user confirmation → \
                     task-card request gate → execution contract → routing → gate/execution/receipt. \
                     Critical: do NOT jump from raw user requests to Light/Medium/Heavy; \
                     \"方案 OK\" does not equal task card approval."
                        .to_string(),
                ),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://protocol/agent-task-protocol".to_string(),
                name: "Agent Task Protocol".to_string(),
                description: Some(
                    "Canonical agent task protocol — complete lifecycle, roles, routing, \
                     review gates, verification, and delivery report format."
                        .to_string(),
                ),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://protocol/task-card-template".to_string(),
                name: "Task Card Template".to_string(),
                description: Some("Fixed project task-card skeleton with all required fields.".to_string()),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://protocol/runtime-adapters".to_string(),
                name: "Runtime Adapters".to_string(),
                description: Some(
                    "Executor, permission mode, parallelism, execution surface, and resolve rules."
                        .to_string(),
                ),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://protocol/task-routing".to_string(),
                name: "Task Routing".to_string(),
                description: Some("Light/Medium/Heavy task routing criteria and escalation rules.".to_string()),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://protocol/evolution-memory".to_string(),
                name: "Evolution Memory".to_string(),
                description: Some(
                    "Evolver advisory recall boundary, method capture rules, and recall documentation requirements."
                        .to_string(),
                ),
                mimeType: Some("text/markdown".to_string()),
            },
            ResourceDef {
                uri: "ags://evolver-boundary".to_string(),
                name: "EvoMap MCP Parallel-Call Boundary".to_string(),
                description: Some(
                    "Defines the parallel relationship between AGS MCP and EvoMap MCP. \
                     AGS is the governance authority; EvoMap provides advisory method \
                     recall during solution formation only. AGS MCP does NOT proxy, \
                     wrap, or broker EvoMap MCP."
                        .to_string(),
                ),
                mimeType: Some("text/markdown".to_string()),
            },
        ],
    }
}

/// Read a resource by URI. Returns structured content or an error.
pub fn read_resource(uri: &str) -> Result<ResourceReadResult, String> {
    match uri {
        "ags://global-kernel" => read_global_kernel(),
        "ags://protocol/agent-task-protocol" => {
            read_protocol_file("protocol/agent-task-protocol.md", "Agent Task Protocol")
        }
        "ags://protocol/task-card-template" => {
            read_protocol_file("protocol/task-card-template.md", "Task Card Template")
        }
        "ags://protocol/runtime-adapters" => {
            read_protocol_file("protocol/runtime-adapters.md", "Runtime Adapters")
        }
        "ags://protocol/task-routing" => {
            read_protocol_file("protocol/task-routing.md", "Task Routing")
        }
        "ags://protocol/evolution-memory" => {
            read_protocol_file("protocol/evolution-memory.md", "Evolution Memory")
        }
        "ags://evolver-boundary" => read_evolver_boundary(),
        other => Err(format!("Unknown resource URI: {}", other)),
    }
}

// ── Resource Content Providers ───────────────────────────────────────────────

fn read_global_kernel() -> Result<ResourceReadResult, String> {
    let text = include_str!("resources/global_kernel.md");
    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: "ags://global-kernel".to_string(),
            mimeType: Some("text/markdown".to_string()),
            text: text.to_string(),
        }],
    })
}

fn read_protocol_file(rel_path: &str, _display_name: &str) -> Result<ResourceReadResult, String> {
    // Try reading from the current working directory's protocol/ dir first,
    // then fall back to the repo root relative to the crate source.
    let cwd_path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(rel_path);

    let (text, source) = if cwd_path.exists() {
        (
            std::fs::read_to_string(&cwd_path)
                .unwrap_or_else(|e| format!("Error reading {}: {}", cwd_path.display(), e)),
            "current working directory",
        )
    } else {
        // Fall back to repo root lookup relative to the crate manifest dir
        let repo_rel = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(rel_path);

        if repo_rel.exists() {
            (
                std::fs::read_to_string(&repo_rel)
                    .unwrap_or_else(|e| format!("Error reading {}: {}", repo_rel.display(), e)),
                "repo root",
            )
        } else {
            return Err(format!(
                "Protocol file not found: {}. Looked at {} and {}.",
                rel_path,
                cwd_path.display(),
                repo_rel.display(),
            ));
        }
    };

    let uri = format!("ags://{}", rel_path.replace("protocol/", "protocol/"));
    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri,
            mimeType: Some("text/markdown".to_string()),
            text: format!(
                "<!-- Source: {} -->\n<!-- Protocol file: {} -->\n\n{}",
                source, rel_path, text
            ),
        }],
    })
}

fn read_evolver_boundary() -> Result<ResourceReadResult, String> {
    let text = include_str!("resources/evolver_boundary.md");
    Ok(ResourceReadResult {
        contents: vec![ResourceContent {
            uri: "ags://evolver-boundary".to_string(),
            mimeType: Some("text/markdown".to_string()),
            text: text.to_string(),
        }],
    })
}
