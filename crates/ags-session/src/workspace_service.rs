//! Workspace-scoped AGS daemon and thin stdio adapter.
//!
//! This facade is the external seam. Registry ownership, immutable capability
//! loading, transport authentication, and upgrade/recycle mechanics remain
//! internal implementation modules.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

mod capability_snapshot;
mod registry_ownership;
mod transport_handshake;
mod upgrade_recycle;

pub(crate) use capability_snapshot::project_facts_hash_at;
pub use capability_snapshot::WorkspaceState;
pub use transport_handshake::{read_workspace_wire_frame, MAX_WORKSPACE_WIRE_FRAME_BYTES};

pub const WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION: &str =
    "ags://schema/contract/v2/workspace-daemon-status";
pub const WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION: &str =
    "ags://schema/contract/v2/workspace-capability-activation";
pub const WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES: &str = "activate-capabilities";
pub const WORKSPACE_COMMAND_CONTROL_PLANE: &str = "control-plane-v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceControlSurface {
    Cli,
    Mcp,
}

/// Host and transport identity established during the authenticated daemon
/// handshake. These fields never appear in an Operation or apply payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceClientIdentity {
    pub connection_id: String,
    pub host_id: String,
}

/// Typed control request sent to the authenticated workspace daemon. Generic
/// parameters are concrete contract-v2 types at the adapter seam; no free-form
/// operation payload is accepted by this Interface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkspaceControlRequest<O, H> {
    Open {
        surface: WorkspaceControlSurface,
    },
    Decide {
        operation: O,
    },
    Apply {
        action_ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<H>,
    },
}

/// Identity facts established by the workspace-service handshake, never
/// accepted from a command payload or CLI argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCommandContext {
    pub canonical_workspace: std::path::PathBuf,
    pub workspace_service_identity: String,
    pub authenticated_session: String,
}

/// Immutable identity for one authenticated long-lived workspace session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSessionContext {
    pub canonical_workspace: std::path::PathBuf,
    pub workspace_service_identity: String,
    pub workspace_identity: String,
    pub project_facts_hash: String,
    pub registry_key: String,
    pub authenticated_session: String,
    pub connection_id: String,
    pub host_id: String,
}

/// Persistent authenticated control connection. One instance is retained by
/// `WorkspaceRouter` for each canonical workspace on an MCP connection.
#[derive(Debug)]
pub struct WorkspaceControlClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    context: WorkspaceSessionContext,
}

impl WorkspaceControlClient {
    pub fn context(&self) -> &WorkspaceSessionContext {
        &self.context
    }

    pub fn request<Req, Res>(&mut self, request: &Req) -> Result<Res, String>
    where
        Req: Serialize,
        Res: for<'de> Deserialize<'de>,
    {
        transport_handshake::write_json_line(&mut self.writer, request)?;
        let response = transport_handshake::read_json_line(&mut self.reader);
        if response
            .as_ref()
            .is_err_and(|error| error.starts_with("workspace_wire_frame_too_large:"))
        {
            let _ = self.writer.shutdown(std::net::Shutdown::Both);
        }
        response
    }

    pub fn shutdown_write(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .and_then(|_| self.writer.shutdown(std::net::Shutdown::Write))
            .map_err(|error| format!("workspace control shutdown failed: {error}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "result",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WorkspaceControlResponse<S, D, R> {
    Opened(S),
    Decided(D),
    Applied(R),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCapabilityActivationRequest {
    pub schema_version: String,
    pub active_hosts: Vec<String>,
    pub retired_hosts: Vec<String>,
    pub replace_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCapabilityActivationResult {
    pub schema_version: String,
    pub activated_snapshot_hashes: BTreeMap<String, String>,
    pub loaded_snapshot_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceServiceStatus {
    pub schema_version: String,
    pub workspace: String,
    pub state: String,
    pub pid: Option<u32>,
    pub endpoint: Option<String>,
    pub executable_hash: Option<String>,
    pub current_executable_hash: String,
    pub current_binary: bool,
}

/// Authenticated, read-only state reported by an already-running workspace
/// daemon. Unlike normal command dispatch, inspection never starts, upgrades,
/// retires, or recycles a daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceServiceInspection {
    pub schema_version: String,
    pub canonical_workspace: String,
    pub workspace_identity: String,
    pub loaded_snapshot_hashes: BTreeMap<String, String>,
}

/// Callback implemented by the protocol adapter for one authenticated client.
pub trait WorkspaceSessionHandler: Send + Sync + 'static {
    fn run(
        &self,
        reader: BufReader<TcpStream>,
        writer: TcpStream,
        workspace: Arc<WorkspaceState>,
        context: WorkspaceSessionContext,
        startup_executable_hash: String,
    );

    /// Handle one authenticated, workspace-scoped command without opening an
    /// MCP client session. Domain adapters own command semantics and state;
    /// `ags-session` owns only authentication and transport.
    fn run_workspace_command(
        &self,
        kind: &str,
        payload: serde_json::Value,
        workspace: Arc<WorkspaceState>,
        context: WorkspaceCommandContext,
    ) -> Result<serde_json::Value, String> {
        let _ = (payload, workspace, context);
        Err(format!("unsupported workspace command `{kind}`"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WorkspaceCommand {
    pub kind: String,
    pub payload: serde_json::Value,
}

pub fn run_workspace_daemon(
    workspace: &Path,
    handler: Arc<dyn WorkspaceSessionHandler>,
) -> Result<(), String> {
    upgrade_recycle::run_workspace_daemon_impl(workspace, handler)
}

pub fn connect_workspace_control_client(
    workspace: &Path,
    connection_id: &str,
    host_id: &str,
) -> Result<WorkspaceControlClient, String> {
    transport_handshake::connect_workspace_control_client_impl(
        workspace,
        WorkspaceClientIdentity {
            connection_id: connection_id.to_string(),
            host_id: host_id.to_string(),
        },
    )
}

pub fn workspace_service_status(workspace: &Path) -> Result<WorkspaceServiceStatus, String> {
    upgrade_recycle::workspace_service_status_impl(workspace)
}

pub fn inspect_existing_workspace_service(
    workspace: &Path,
) -> Result<Option<WorkspaceServiceInspection>, String> {
    transport_handshake::inspect_existing_workspace_service_impl(workspace)
}

pub fn restart_workspace_service(workspace: &Path) -> Result<WorkspaceServiceStatus, String> {
    upgrade_recycle::restart_workspace_service_impl(workspace)
}

pub fn dispatch_workspace_command(
    workspace: &Path,
    kind: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, String> {
    transport_handshake::dispatch_workspace_command_impl(workspace, kind, payload)
}

pub fn dispatch_workspace_control<Req, Res>(workspace: &Path, request: &Req) -> Result<Res, String>
where
    Req: Serialize,
    Res: for<'de> Deserialize<'de>,
{
    let payload = serde_json::to_value(request)
        .map_err(|error| format!("control-plane request encode failed: {error}"))?;
    let response = dispatch_workspace_command(workspace, WORKSPACE_COMMAND_CONTROL_PLANE, payload)?;
    serde_json::from_value(response)
        .map_err(|error| format!("control-plane response decode failed: {error}"))
}

#[cfg(test)]
mod tests;
