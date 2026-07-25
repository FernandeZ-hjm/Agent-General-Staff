use crate::cli::McpAction;
use crate::context::AGS_VERSION;

/// Start the AGS MCP stdio adapter with the given transport.
///
/// The stdio process is a thin proxy. It connects to, or starts, the unique
/// daemon keyed by the canonical workspace path.
fn cmd_mcp_serve(transport: &str) {
    match transport {
        "stdio" => {
            eprintln!(
                "[ags-mcp] starting AGS MCP workspace adapter v{} on stdio",
                AGS_VERSION
            );
            eprintln!("[ags-mcp] AGS MCP is the mandatory governance interface (NOT a governed third-party MCP).");
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
        McpAction::Serve { transport } => cmd_mcp_serve(&transport),
        McpAction::WorkspaceDaemon { workspace } => {
            if let Err(error) = ags_mcp::run_workspace_daemon(&workspace) {
                eprintln!("[ags-mcp] workspace daemon failed: {error}");
                std::process::exit(1);
            }
        }
    }
}
