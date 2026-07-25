//! Workspace-scoped AGS service and session authority.
//!
//! The instance key is derived only from the canonical workspace path. Hosts
//! are client attributes; preflight bindings and one-shot action leases remain
//! isolated per client session.

mod action_store;
mod client_session;
mod workspace_service;

use std::path::{Path, PathBuf};

pub use action_store::SessionActionStore;
pub use client_session::WorkspaceClientSession;
pub use workspace_service::{
    run_stdio_adapter, run_workspace_daemon, WorkspaceSessionHandler, WorkspaceState,
};

/// Host and target accepted by a successful preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightBinding {
    pub host: String,
    pub target: PathBuf,
    pub host_home: PathBuf,
}

/// Capability access supplied by the workspace service to a client session.
pub trait CapabilityCatalogSource {
    fn capability_reference(&self, target: &Path, host: &str) -> serde_json::Value;
    fn load_validated_snapshot(
        &self,
        binding: &PreflightBinding,
    ) -> Result<
        (
            skill_resolver::HostCapabilitySnapshot,
            skill_resolver::ActiveSkillTable,
        ),
        String,
    >;
}
