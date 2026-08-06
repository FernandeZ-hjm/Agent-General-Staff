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
//! In `manifests/mcp-registry.yaml`, `ags` resides under `suite_interfaces:`.
//! Third-party MCP parents come from the canonical capability catalog and
//! become active only when the current Host probe confirms them.
//!
//! # Advisory-system boundary
//!
//! AGS MCP and third-party memory or advisory systems are **parallel peers**.
//! AGS MCP is the governance authority; advisory output may inform solution
//! formation only. AGS MCP does not proxy, wrap, or broker third-party MCP calls.
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

#[derive(Debug, Default)]
pub struct WorkspaceCapabilityRuntimeActivator;

impl ags_lifecycle::maintenance::CapabilityRuntimeActivator
    for WorkspaceCapabilityRuntimeActivator
{
    fn activate(
        &self,
        request: &ags_lifecycle::maintenance::CapabilityRuntimeActivationRequest,
    ) -> Result<ags_lifecycle::maintenance::CapabilityRuntimeActivationResult, String> {
        if ags_platform::normalize_path(&request.runtime_home)
            != ags_platform::normalize_path(&ags_platform::runtime_home())
            || ags_session::inspect_existing_workspace_service(&request.workspace)?.is_none()
        {
            return Ok(
                ags_lifecycle::maintenance::CapabilityRuntimeActivationResult {
                    activated_snapshot_hashes: request.active_snapshot_hashes.clone(),
                    loaded_snapshot_hashes: None,
                    runtime_identity: None,
                },
            );
        }
        let wire_request = ags_session::WorkspaceCapabilityActivationRequest {
            schema_version: ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION.to_string(),
            active_hosts: request.active_hosts(),
            retired_hosts: request.retired_hosts.clone(),
            replace_all: request.replace_all,
        };
        let value = ags_session::dispatch_workspace_command(
            &request.workspace,
            ags_session::WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES,
            serde_json::to_value(wire_request)
                .map_err(|error| format!("cannot encode capability activation: {error}"))?,
        )?;
        let wire_result: ags_session::WorkspaceCapabilityActivationResult =
            serde_json::from_value(value)
                .map_err(|error| format!("capability activation result invalid: {error}"))?;
        if wire_result.schema_version != ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION
        {
            return Err("capability activation result schema mismatch".to_string());
        }
        let inspection = ags_session::inspect_existing_workspace_service(&request.workspace)?
            .ok_or_else(|| {
                "workspace daemon disappeared after capability activation".to_string()
            })?;
        Ok(
            ags_lifecycle::maintenance::CapabilityRuntimeActivationResult {
                activated_snapshot_hashes: wire_result.activated_snapshot_hashes,
                loaded_snapshot_hashes: Some(inspection.loaded_snapshot_hashes),
                runtime_identity: Some(inspection.workspace_identity),
            },
        )
    }
}

pub fn workspace_capability_runtime_activator(
) -> std::sync::Arc<dyn ags_lifecycle::maintenance::CapabilityRuntimeActivator> {
    std::sync::Arc::new(WorkspaceCapabilityRuntimeActivator)
}

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
        if kind == ags_session::WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES {
            let request: ags_session::WorkspaceCapabilityActivationRequest =
                serde_json::from_value(payload)
                    .map_err(|error| format!("capability activation request invalid: {error}"))?;
            if request.schema_version != ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION
            {
                return Err("capability activation schema mismatch".to_string());
            }
            let active_hosts = request
                .active_hosts
                .iter()
                .map(|host| {
                    ags_host_integration::platform_spec(host).ok_or_else(|| {
                        format!("unsupported capability activation Host `{host}`")
                    })?;
                    Ok(host.clone())
                })
                .collect::<Result<Vec<_>, String>>()?;
            let retired_hosts = request
                .retired_hosts
                .iter()
                .map(|host| {
                    ags_host_integration::platform_spec(host)
                        .ok_or_else(|| format!("unsupported retired capability Host `{host}`"))?;
                    Ok(host.clone())
                })
                .collect::<Result<Vec<_>, String>>()?;
            let activated_snapshot_hashes = workspace.activate_host_snapshots(
                &active_hosts,
                &retired_hosts,
                request.replace_all,
            )?;
            let result = ags_session::WorkspaceCapabilityActivationResult {
                schema_version: ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION
                    .to_string(),
                activated_snapshot_hashes,
                loaded_snapshot_hashes: workspace.loaded_snapshot_hashes()?,
            };
            return serde_json::to_value(result)
                .map_err(|error| format!("capability activation encode failed: {error}"));
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
