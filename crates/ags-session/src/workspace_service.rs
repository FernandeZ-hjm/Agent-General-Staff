//! Workspace-scoped AGS daemon and thin stdio adapter.
//!
//! This facade is the external seam. Registry ownership, immutable capability
//! loading, transport authentication, and upgrade/recycle mechanics remain
//! internal implementation modules.

use std::io::BufReader;
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

#[path = "workspace_service/capability_bundle.rs"]
mod capability_snapshot;
mod registry_ownership;
mod transport_handshake;
mod upgrade_recycle;

pub use capability_snapshot::WorkspaceState;

/// Callback implemented by the protocol adapter for one authenticated client.
pub trait WorkspaceSessionHandler: Send + Sync + 'static {
    fn run(
        &self,
        reader: BufReader<TcpStream>,
        writer: TcpStream,
        workspace: Arc<WorkspaceState>,
        session_id: String,
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

#[cfg(test)]
mod tests;
