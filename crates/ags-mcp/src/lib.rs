//! AGS MCP Server — host initialization adapter and mandatory governance
//! interface over MCP (Model Context Protocol).
//!
//! Exposes AGS governance tools, resources, and prompts through a thin stdio
//! JSON-RPC adapter backed by one daemon per canonical workspace, enabling
//! Tencent Agent (WorkBuddy, CodeBuddy-Code), Codex, OMP, Cursor, Claude Code
//! and other MCP hosts to call AGS governance gates as a global capability.
//!
//! # Initialization Gate
//!
//! `ags_preflight` is the **mandatory first call** for all AGS scenarios.
//! Hosts MUST complete preflight (MCP or CLI fallback `ags session preflight
//! --for <agent>`) before invoking any other AGS tool. `ags_route_request`
//! validates a typed host proposal read-only; it is NOT a preflight substitute
//! and never interprets raw natural language.
//!
//! # Identity
//!
//! AGS MCP is the suite's own host adapter — NOT a governed third-party MCP.
//! In `manifests/mcp-registry.yaml`, `ags` resides under `suite_interfaces:`,
//! not alongside governed third-party MCPs under `mcps:`.
//!
//! # Usage
//!
//! ```bash
//! ags mcp serve --transport stdio
//! ```

mod prompts;
mod protocol;
mod resources;
mod server;
mod tools;

pub use ags_session::{
    inspect_existing_workspace_service, restart_workspace_service, run_stdio_adapter,
    workspace_service_status, WorkspaceServiceInspection, WorkspaceServiceStatus,
    WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION,
};

use std::io::BufReader;
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

struct McpSessionHandler {
    lifecycle: ags_lifecycle::workspace_lifecycle::LifecycleKernel,
}

impl McpSessionHandler {
    fn new(workspace: &Path) -> Result<Self, String> {
        Ok(Self {
            lifecycle: ags_lifecycle::workspace_lifecycle::LifecycleKernel::new(
                workspace.to_path_buf(),
                ags_platform::home_dir_or_temp(),
            )?,
        })
    }
}

impl ags_session::WorkspaceSessionHandler for McpSessionHandler {
    fn run(
        &self,
        reader: BufReader<TcpStream>,
        writer: TcpStream,
        workspace: Arc<ags_session::WorkspaceState>,
        session_id: String,
        startup_executable_hash: String,
    ) {
        server::run_mcp_session(
            reader,
            writer,
            workspace,
            session_id,
            startup_executable_hash,
        );
    }

    fn run_workspace_command(
        &self,
        kind: &str,
        payload: serde_json::Value,
        workspace: Arc<ags_session::WorkspaceState>,
    ) -> Result<serde_json::Value, String> {
        if kind == "status" {
            return serde_json::to_value(ags_session::WorkspaceServiceInspection {
                schema_version: ags_session::WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION.to_string(),
                canonical_workspace: workspace.root().to_string_lossy().to_string(),
                workspace_identity: workspace.instance_key().to_string(),
                loaded_snapshot_hashes: workspace.loaded_snapshot_hashes()?,
            })
            .map_err(|error| format!("workspace daemon status encode failed: {error}"));
        }
        if kind != "lifecycle" {
            return Err(format!("unsupported workspace command `{kind}`"));
        }
        let envelope: ags_lifecycle::workspace_lifecycle::LifecycleEnvelope =
            serde_json::from_value(payload)
                .map_err(|error| format!("workspace lifecycle envelope invalid: {error}"))?;
        let decision = self.lifecycle.process(envelope)?;
        serde_json::to_value(decision)
            .map_err(|error| format!("workspace lifecycle decision encode failed: {error}"))
    }
}

pub fn run_workspace_daemon(workspace: &Path) -> Result<(), String> {
    ags_session::run_workspace_daemon(workspace, Arc::new(McpSessionHandler::new(workspace)?))
}
