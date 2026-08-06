use crate::cli::McpAction;
use crate::context::AGS_VERSION;

/// Start the AGS MCP stdio adapter with the given transport.
///
/// The stdio process is a thin proxy. It connects to, or starts, the unique
/// daemon keyed by the canonical workspace path.
///
/// AGS MCP and EvoMap MCP are parallel peers — AGS MCP does NOT proxy,
/// wrap, or broker EvoMap MCP calls.
fn cmd_mcp_serve(transport: &str) {
    match transport {
        "stdio" => {
            eprintln!(
                "[ags-mcp] starting AGS MCP workspace adapter v{} on stdio",
                AGS_VERSION
            );
            eprintln!("[ags-mcp] AGS MCP is the mandatory governance interface (NOT a governed third-party MCP).");
            eprintln!("[ags-mcp] EvoMap boundary: AGS MCP and EvoMap MCP are parallel peers. AGS MCP does not proxy EvoMap MCP.");
            if let Err(error) = ags_mcp::run_stdio_adapter() {
                eprintln!("[ags-mcp] workspace adapter failed: {error}");
                std::process::exit(1);
            }
        }
        other => {
            eprintln!(
                "ags mcp serve: unsupported transport '{}' — only 'stdio' is supported in v1",
                other
            );
            std::process::exit(2);
        }
    }
}

pub(crate) fn run(action: McpAction) {
    match action {
        McpAction::Status { target } => {
            let status =
                ags_mcp::workspace_service_status(&target).unwrap_or_else(|error| fail(error));
            println!(
                "{}",
                serde_json::to_string_pretty(&status).expect("MCP status is serializable")
            );
        }
        McpAction::Restart { target } => {
            let status =
                ags_mcp::restart_workspace_service(&target).unwrap_or_else(|error| fail(error));
            println!(
                "{}",
                serde_json::to_string_pretty(&status).expect("MCP status is serializable")
            );
        }
        McpAction::Serve { transport } => cmd_mcp_serve(&transport),
        McpAction::WorkspaceDaemon { workspace } => {
            if let Err(error) = ags_mcp::run_workspace_daemon(&workspace) {
                eprintln!("[ags-mcp] workspace daemon failed: {error}");
                std::process::exit(1);
            }
        }
    }
}

fn fail(error: String) -> ! {
    eprintln!("ags mcp: {error}");
    std::process::exit(1)
}
