//! Workspace-scoped AGS daemon and thin stdio adapter.
//!
//! This facade is the external seam. Registry ownership, immutable capability
//! loading, transport authentication, and upgrade/recycle mechanics remain
//! internal implementation modules.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::BufReader;
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

mod capability_snapshot;
mod registry_ownership;
mod transport_handshake;
mod upgrade_recycle;

pub use capability_snapshot::WorkspaceState;

pub const WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION: &str = "0.4.0-workspace-daemon-status";
pub const WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION: &str =
    "0.4.13-workspace-capability-activation";
pub const WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES: &str = "activate-capabilities";

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
        session_id: String,
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
    ) -> Result<serde_json::Value, String> {
        let _ = (payload, workspace);
        Err(format!("unsupported workspace command `{kind}`"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WorkspaceCommand {
    pub kind: String,
    pub payload: serde_json::Value,
}

pub fn run_stdio_adapter() -> Result<(), String> {
    transport_handshake::run_stdio_adapter_impl()
}

pub fn run_workspace_daemon(
    workspace: &Path,
    handler: Arc<dyn WorkspaceSessionHandler>,
) -> Result<(), String> {
    upgrade_recycle::run_workspace_daemon_impl(workspace, handler)
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

#[cfg(test)]
mod tests;
