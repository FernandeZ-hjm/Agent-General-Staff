//! Workspace-scoped AGS daemon and thin stdio adapter.
//!
//! This facade is the external seam. Registry ownership, immutable capability
//! loading, transport authentication, and upgrade/recycle mechanics remain
//! internal implementation modules.

use serde::Serialize;
use std::io::BufReader;
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

mod capability_snapshot;
mod registry_ownership;
mod transport_handshake;
mod upgrade_recycle;

pub use capability_snapshot::WorkspaceState;

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

pub fn restart_workspace_service(workspace: &Path) -> Result<WorkspaceServiceStatus, String> {
    upgrade_recycle::restart_workspace_service_impl(workspace)
}

#[cfg(test)]
mod tests;
