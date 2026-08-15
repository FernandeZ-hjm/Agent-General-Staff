//! Contract-v2 workspace routing and authenticated per-workspace service.
//!
//! The public surface contains only request-scoped resolver/router types and
//! the daemon control protocol. Contract-v1 preflight bindings, client
//! sessions, and lease stores were removed by the v0.4.20 hard cut.

mod workspace_router;
mod workspace_service;

pub use workspace_router::{
    AuthenticatedWorkspaceSession, WorkspaceBinding, WorkspaceContext, WorkspaceResolutionError,
    WorkspaceResolver, WorkspaceRouter, MAX_WORKSPACE_SESSIONS,
};
pub use workspace_service::{
    connect_workspace_control_client, dispatch_workspace_command, dispatch_workspace_control,
    inspect_existing_workspace_service, read_workspace_wire_frame, restart_workspace_service,
    run_workspace_daemon, workspace_service_status, WorkspaceCapabilityActivationRequest,
    WorkspaceCapabilityActivationResult, WorkspaceClientIdentity, WorkspaceCommandContext,
    WorkspaceControlClient, WorkspaceControlRequest, WorkspaceControlResponse,
    WorkspaceControlSurface, WorkspaceServiceInspection, WorkspaceServiceStatus,
    WorkspaceSessionContext, WorkspaceSessionHandler, WorkspaceState,
    MAX_WORKSPACE_WIRE_FRAME_BYTES, WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION,
    WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES, WORKSPACE_COMMAND_CONTROL_PLANE,
    WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION,
};
