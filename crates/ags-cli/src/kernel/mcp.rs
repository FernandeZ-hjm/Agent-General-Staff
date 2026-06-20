use crate::cli::McpAction;
use crate::context::AGS_VERSION;

/// Start the AGS MCP server with the given transport.
///
/// V1 supports only stdio transport. The server reads line-delimited
/// JSON-RPC 2.0 messages from stdin and writes responses to stdout.
/// Stderr is reserved for server logging.
fn cmd_mcp_serve(transport: &str) {
    match transport {
        "stdio" => {
            eprintln!(
                "[ags-mcp] starting AGS MCP host initialization adapter v{} on stdio",
                AGS_VERSION
            );
            eprintln!("[ags-mcp] AGS MCP is the mandatory governance interface (NOT a governed third-party MCP).");
            ags_mcp::run_mcp_server();
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
    }
}
