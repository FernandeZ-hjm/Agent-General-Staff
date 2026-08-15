//! Contract-v2 MCP transport: JSON-RPC framing plus request-scoped routing.

use crate::protocol::{ToolDef, ToolListResult};
use ags_control_plane::{
    host_outcome_input_schema, operation_request_schema, AdapterSurface,
    ContentAddressedArtifactRef, DetailsReadRequest, HostOutcomeInput, OperationContext,
    OperationRequest, DETAILS_CHUNK_LIMIT,
};
use ags_host_integration::HostId;
use ags_session::{
    connect_workspace_control_client, AuthenticatedWorkspaceSession, WorkspaceBinding,
    WorkspaceContext, WorkspaceControlClient, WorkspaceControlRequest, WorkspaceControlResponse,
    WorkspaceControlSurface, WorkspaceResolutionError, WorkspaceRouter,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub const TOOL_DECIDE: &str = "ags_decide";
pub const TOOL_APPLY: &str = "ags_apply";
const TOOL_SCHEMA_BUDGET: usize = 8 * 1024;
pub const MAX_MCP_RESULT_BYTES: usize = 16 * 1024;
pub const MAX_ROUTED_ACTIONS: usize = 128;
const MAX_INLINE_STRUCTURED_BYTES: usize = 12 * 1024;
const MAX_ERROR_TEXT_BYTES: usize = 2 * 1024;
static CONNECTION_COUNTER: AtomicU64 = AtomicU64::new(1);
const MAX_MCP_ROOTS: usize = 64;
const MAX_ROOT_URI_BYTES: usize = 4 * 1024;
const MAX_MCP_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 4 * 1024;
const MAX_DETAILS_RESOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DETAILS_CHUNKS: usize = MAX_DETAILS_RESOURCE_BYTES / DETAILS_CHUNK_LIMIT as usize + 1;
const MAX_DETAILS_REFERENCE_NODES: usize = 512;
const MAX_DETAILS_REFERENCE_DEPTH: usize = 32;
const ROOTS_REQUEST_ID_PREFIX: &str = "ags-roots-";

#[derive(Debug, Clone, PartialEq)]
enum RequestId {
    String(String),
    Number(serde_json::Number),
}

impl RequestId {
    fn parse(value: &Value) -> Result<Self, &'static str> {
        let id = match value {
            Value::String(value) if value.len() <= MAX_REQUEST_ID_BYTES => {
                Self::String(value.clone())
            }
            Value::Number(value) if value.to_string().len() <= MAX_REQUEST_ID_BYTES => {
                Self::Number(value.clone())
            }
            Value::String(_) | Value::Number(_) => return Err("request id exceeds 4 KiB"),
            _ => return Err("request id must be a string or number"),
        };
        Ok(id)
    }

    fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Number(_) => None,
        }
    }

    fn into_value(self) -> Value {
        match self {
            Self::String(value) => Value::String(value),
            Self::Number(value) => Value::Number(value),
        }
    }
}

#[derive(Debug)]
struct Outbound {
    envelope: Value,
    terminal_delivery: Option<String>,
}

impl Outbound {
    fn new(envelope: Value) -> Self {
        Self::with_terminal_delivery(envelope, None)
    }

    fn with_terminal_delivery(envelope: Value, terminal_delivery: Option<String>) -> Self {
        if serde_json::to_vec(&envelope).is_ok_and(|bytes| bytes.len() <= MAX_MCP_RESULT_BYTES) {
            return Self {
                envelope,
                terminal_delivery,
            };
        }
        let response_id = envelope.get("id").and_then(|id| RequestId::parse(id).ok());
        let envelope = error_response(
            response_id,
            -32603,
            "response_too_large: outer JSON-RPC envelope exceeds 16 KiB",
        );
        debug_assert!(
            serde_json::to_vec(&envelope).is_ok_and(|bytes| bytes.len() <= MAX_MCP_RESULT_BYTES)
        );
        Self {
            envelope,
            terminal_delivery: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootsDiscovery {
    NotNegotiated,
    Pending,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    pub connection_id: String,
    pub host_id: HostId,
    pub workspace: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideArguments {
    pub operation: OperationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyArguments {
    pub action_ref: String,
    #[serde(default)]
    pub outcome: Option<HostOutcomeInput>,
}

/// The only transport-to-workspace-service seam.
pub trait ControlPlaneSeam {
    type Session;

    fn open(
        &mut self,
        context: &RequestContext,
    ) -> Result<AuthenticatedWorkspaceSession<Self::Session>, String>;

    fn decide(
        &mut self,
        context: &RequestContext,
        session: &mut Self::Session,
        operation: OperationRequest,
    ) -> Result<Value, String>;

    fn apply(
        &mut self,
        context: &RequestContext,
        session: &mut Self::Session,
        action_ref: String,
        outcome: Option<HostOutcomeInput>,
    ) -> Result<Value, String>;
}

pub struct Connection<P: ControlPlaneSeam> {
    connection_id: String,
    host_id: Option<HostId>,
    roots: Vec<PathBuf>,
    roots_discovery: RootsDiscovery,
    adapter_cwd: PathBuf,
    router: Option<WorkspaceRouter<P::Session>>,
    action_routes: HashMap<String, ActionRoute>,
    action_order: VecDeque<String>,
    action_eviction_count: usize,
    details_routes: HashMap<String, DetailsRoute>,
    details_order: VecDeque<String>,
    port: P,
}

#[derive(Debug, Clone)]
struct ActionRoute {
    binding: WorkspaceBinding,
    terminal_response: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetailsRoute {
    binding: WorkspaceBinding,
    sha256: String,
}

impl<P: ControlPlaneSeam> Connection<P> {
    pub fn new(adapter_cwd: PathBuf, port: P) -> Self {
        let sequence = CONNECTION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            connection_id: format!("mcp-{}-{sequence}", std::process::id()),
            host_id: None,
            roots: Vec::new(),
            roots_discovery: RootsDiscovery::NotNegotiated,
            adapter_cwd,
            router: None,
            action_routes: HashMap::new(),
            action_order: VecDeque::new(),
            action_eviction_count: 0,
            details_routes: HashMap::new(),
            details_order: VecDeque::new(),
            port,
        }
    }

    pub fn initialize(&mut self, host: &str) -> Result<(), String> {
        let normalized = HostId::new(host)?;
        if let Some(bound) = &self.host_id {
            if bound != &normalized {
                return Err("connection_host_mismatch".to_string());
            }
        } else {
            self.router = Some(
                WorkspaceRouter::new(&self.connection_id, normalized.as_str())
                    .map_err(|error| error.to_string())?,
            );
            self.host_id = Some(normalized);
        }
        Ok(())
    }

    fn update_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
        self.roots_discovery = RootsDiscovery::Available;
    }

    fn mark_roots_pending(&mut self) {
        self.roots_discovery = RootsDiscovery::Pending;
    }

    fn mark_roots_unavailable(&mut self) {
        self.roots.clear();
        self.roots_discovery = RootsDiscovery::Unavailable;
    }

    pub fn decide(&mut self, mut arguments: DecideArguments) -> Result<Value, String> {
        if arguments.operation.spec().adapter_surface != AdapterSurface::ProductCli {
            return Err(format!(
                "operation_surface_forbidden: {} is not an MCP product operation",
                arguments.operation.name().as_str()
            ));
        }
        let explicit = arguments
            .operation
            .context()
            .workspace
            .as_ref()
            .map(PathBuf::from);
        let (context, workspace_context) = self.request_context(explicit)?;
        let router = self
            .router
            .as_mut()
            .ok_or_else(|| "connection_not_initialized".to_string())?;
        let port = &mut self.port;
        let binding = router
            .open_workspace(&workspace_context, |path| {
                port.open(&context)
                    .map_err(|detail| WorkspaceResolutionError {
                        code: "workspace_session_open_failed",
                        detail,
                        candidates: vec![path.to_path_buf()],
                    })
            })
            .map_err(|error| error.to_string())?;
        arguments.operation.context_mut().workspace =
            Some(binding.canonical_workspace().to_string_lossy().into_owned());
        let value = {
            let session = router
                .session_mut(&binding)
                .map_err(|error| error.to_string())?;
            port.decide(&context, session, arguments.operation)?
        };
        self.route_details(&value, binding.clone())?;
        if let Some(action_ref) = value.get("action_ref").and_then(Value::as_str) {
            if !action_ref.is_empty() {
                self.route_action(action_ref.to_string(), binding)?;
            }
        }
        Ok(value)
    }

    pub fn apply(&mut self, arguments: ApplyArguments) -> Result<Value, String> {
        let route = self
            .action_routes
            .get(&arguments.action_ref)
            .cloned()
            .ok_or_else(|| "action_ref_invalid".to_string())?;
        if let Some(terminal) = route.terminal_response {
            return Ok(terminal);
        }
        let binding = route.binding;
        let context = RequestContext {
            connection_id: binding.connection_id().to_string(),
            host_id: HostId::new(binding.host_id())?,
            workspace: binding.canonical_workspace().to_path_buf(),
        };
        let router = self
            .router
            .as_mut()
            .ok_or_else(|| "connection_not_initialized".to_string())?;
        let port = &mut self.port;
        let result = (|| {
            let session = router
                .session_mut(&binding)
                .map_err(|error| error.to_string())?;
            let value = port.apply(
                &context,
                session,
                arguments.action_ref.clone(),
                arguments.outcome,
            )?;
            Ok(value)
        })();
        match result {
            Ok(value) => {
                self.route_details(&value, binding.clone())?;
                if apply_response_disposition(&value) == ApplyResponseDisposition::Terminal {
                    if let Some(route) = self.action_routes.get_mut(&arguments.action_ref) {
                        route.terminal_response = Some(value.clone());
                    }
                }
                Ok(value)
            }
            Err(error) => Err(error),
        }
    }

    pub fn open_count(&self) -> usize {
        self.router.as_ref().map_or(0, WorkspaceRouter::open_count)
    }

    pub fn handshake_count(&self) -> usize {
        self.router
            .as_ref()
            .map_or(0, WorkspaceRouter::handshake_count)
    }

    pub fn warm_hit_count(&self) -> usize {
        self.router
            .as_ref()
            .map_or(0, WorkspaceRouter::warm_hit_count)
    }

    pub fn stale_reopen_count(&self) -> usize {
        self.router
            .as_ref()
            .map_or(0, WorkspaceRouter::stale_reopen_count)
    }

    pub fn routed_action_count(&self) -> usize {
        self.action_routes.len()
    }

    pub fn action_eviction_count(&self) -> usize {
        self.action_eviction_count
    }

    fn listed_resources(&self) -> Value {
        let resources = self
            .details_order
            .iter()
            .filter_map(|uri| {
                self.details_routes.get(uri).map(|route| {
                    json!({
                        "uri": uri,
                        "name": "AGS immutable operation details",
                        "mimeType": "application/json",
                        "_meta": {"sha256": route.sha256},
                    })
                })
            })
            .collect::<Vec<_>>();
        json!({"resources": resources})
    }

    fn route_action(
        &mut self,
        action_ref: String,
        binding: WorkspaceBinding,
    ) -> Result<(), String> {
        if self.action_routes.contains_key(&action_ref) {
            return Err("action_ref_collision".to_string());
        }
        if self.action_routes.len() == MAX_ROUTED_ACTIONS {
            let oldest = self
                .action_order
                .pop_front()
                .expect("a full action route cache has an oldest action");
            self.action_routes.remove(&oldest);
            self.action_eviction_count = self.action_eviction_count.saturating_add(1);
        }
        self.action_order.push_back(action_ref.clone());
        self.action_routes.insert(
            action_ref,
            ActionRoute {
                binding,
                terminal_response: None,
            },
        );
        Ok(())
    }

    fn remove_action_route(&mut self, action_ref: &str) {
        if self.action_routes.remove(action_ref).is_some() {
            self.action_order
                .retain(|candidate| candidate != action_ref);
        }
    }

    fn route_details(&mut self, value: &Value, binding: WorkspaceBinding) -> Result<(), String> {
        let mut pending = vec![(value, 0_usize)];
        let mut visited = 0_usize;
        while let Some((candidate, depth)) = pending.pop() {
            visited = visited.saturating_add(1);
            if visited > MAX_DETAILS_REFERENCE_NODES || depth > MAX_DETAILS_REFERENCE_DEPTH {
                return Err("details_reference_budget_exceeded".to_string());
            }
            match candidate {
                Value::Object(object) => {
                    if let Some(uri_value) = object.get("details_uri") {
                        let uri = uri_value
                            .as_str()
                            .ok_or_else(|| "details_reference_invalid".to_string())?;
                        let sha256 = object
                            .get("sha256")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "details_reference_missing_sha256".to_string())?;
                        if !uri.starts_with("ags://details/")
                            || uri.len() <= "ags://details/".len()
                            || !ags_platform::is_sha256(sha256)
                        {
                            return Err("details_reference_invalid".to_string());
                        }
                        self.route_one_details(uri, sha256, binding.clone())?;
                    }
                    pending.extend(object.values().map(|value| (value, depth + 1)));
                }
                Value::Array(values) => {
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn route_one_details(
        &mut self,
        uri: &str,
        sha256: &str,
        binding: WorkspaceBinding,
    ) -> Result<(), String> {
        let route = DetailsRoute {
            binding,
            sha256: sha256.to_string(),
        };
        if let Some(existing) = self.details_routes.get(uri) {
            return (existing == &route)
                .then_some(())
                .ok_or_else(|| "details_uri_collision".to_string());
        }
        if self.details_routes.len() == MAX_ROUTED_ACTIONS {
            let oldest = self
                .details_order
                .pop_front()
                .expect("a full details route cache has an oldest route");
            self.details_routes.remove(&oldest);
        }
        self.details_order.push_back(uri.to_string());
        self.details_routes.insert(uri.to_string(), route);
        Ok(())
    }

    fn read_resource(&mut self, uri: &str) -> Result<Value, String> {
        let route = self
            .details_routes
            .get(uri)
            .cloned()
            .ok_or_else(|| "details_uri_unauthorized".to_string())?;
        let context = RequestContext {
            connection_id: route.binding.connection_id().to_string(),
            host_id: HostId::new(route.binding.host_id())?,
            workspace: route.binding.canonical_workspace().to_path_buf(),
        };
        let router = self
            .router
            .as_mut()
            .ok_or_else(|| "connection_not_initialized".to_string())?;
        let session = router
            .session_mut(&route.binding)
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut offset = 0_u64;
        for _ in 0..MAX_DETAILS_CHUNKS {
            let operation = OperationRequest::DetailsRead(DetailsReadRequest {
                context: OperationContext {
                    workspace: Some(
                        route
                            .binding
                            .canonical_workspace()
                            .to_string_lossy()
                            .into_owned(),
                    ),
                },
                artifact: ContentAddressedArtifactRef {
                    uri: uri.to_string(),
                    sha256: route.sha256.clone(),
                },
                offset,
                max_bytes: DETAILS_CHUNK_LIMIT,
            });
            let response = self.port.decide(&context, session, operation)?;
            let chunk = response
                .get("result")
                .and_then(Value::as_object)
                .ok_or_else(|| "details_chunk_missing".to_string())?;
            let data = chunk
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| "details_chunk_data_missing".to_string())?;
            decode_hex_into(data, &mut bytes)?;
            if bytes.len() > MAX_DETAILS_RESOURCE_BYTES {
                return Err("details_resource_exceeds_8_mib".to_string());
            }
            let next = chunk
                .get("next_offset")
                .and_then(Value::as_u64)
                .ok_or_else(|| "details_chunk_next_offset_missing".to_string())?;
            if next <= offset && !chunk.get("eof").and_then(Value::as_bool).unwrap_or(false) {
                return Err("details_chunk_did_not_advance".to_string());
            }
            offset = next;
            if chunk.get("eof").and_then(Value::as_bool) == Some(true) {
                let text = String::from_utf8(bytes)
                    .map_err(|_| "details_resource_is_not_utf8".to_string())?;
                return Ok(json!({
                    "contents": [{"uri": uri, "mimeType": "application/json", "text": text}]
                }));
            }
        }
        Err("details_resource_chunk_limit_exceeded".to_string())
    }

    fn complete_terminal_delivery(&mut self, action_ref: &str) {
        if self
            .action_routes
            .get(action_ref)
            .and_then(|route| route.terminal_response.as_ref())
            .is_some_and(|value| {
                apply_response_disposition(value) == ApplyResponseDisposition::Terminal
            })
        {
            self.remove_action_route(action_ref);
        }
    }

    fn request_context(
        &self,
        explicit: Option<PathBuf>,
    ) -> Result<(RequestContext, WorkspaceContext), String> {
        let host_id = self
            .host_id
            .clone()
            .ok_or_else(|| "connection_not_initialized".to_string())?;
        if explicit.is_none() && self.roots_discovery == RootsDiscovery::Pending {
            return Err("workspace_roots_pending".to_string());
        }
        let omitted = explicit.is_none();
        let workspace_context = WorkspaceContext {
            workspace: explicit,
            mcp_roots: self.roots.clone(),
            adapter_cwd: self.adapter_cwd.clone(),
        };
        let workspace = self
            .router
            .as_ref()
            .ok_or_else(|| "connection_not_initialized".to_string())?
            .resolve(&workspace_context)
            .map_err(|error| {
                if omitted
                    && self.roots_discovery == RootsDiscovery::Unavailable
                    && error.code == "workspace_required"
                {
                    "workspace_roots_unavailable".to_string()
                } else {
                    error.to_string()
                }
            })?;
        Ok((
            RequestContext {
                connection_id: self.connection_id.clone(),
                host_id,
                workspace,
            },
            workspace_context,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyResponseDisposition {
    AwaitingOutcome,
    Terminal,
    Retain,
}

fn apply_response_disposition(value: &Value) -> ApplyResponseDisposition {
    let state = value.get("state").and_then(Value::as_str);
    let reason = value.get("reason_code").and_then(Value::as_str);
    if state == Some("awaiting-outcome") && reason == Some("host_outcome_required") {
        return ApplyResponseDisposition::AwaitingOutcome;
    }
    if matches!(state, Some("receipted" | "risk-escalated")) {
        return ApplyResponseDisposition::Terminal;
    }
    ApplyResponseDisposition::Retain
}

#[derive(Debug, Default)]
pub struct WorkspaceRpcPort;

impl ControlPlaneSeam for WorkspaceRpcPort {
    type Session = WorkspaceControlClient;

    fn open(
        &mut self,
        context: &RequestContext,
    ) -> Result<AuthenticatedWorkspaceSession<Self::Session>, String> {
        let mut client = connect_workspace_control_client(
            &context.workspace,
            &context.connection_id,
            context.host_id.as_str(),
        )?;
        let reply: crate::ControlReply = client.request(&WorkspaceControlRequest::<
            OperationRequest,
            HostOutcomeInput,
        >::Open {
            surface: WorkspaceControlSurface::Mcp,
        })?;
        match reply {
            crate::ControlReply::Ok(response) => match *response {
                WorkspaceControlResponse::Opened(_) => {}
                _ => return Err("control_open_response_mismatch".to_string()),
            },
            crate::ControlReply::Error(error) => {
                return Err(format!("{}: {}", error.code, error.detail))
            }
        }
        let metadata = client.context().clone();
        AuthenticatedWorkspaceSession::new(
            metadata.canonical_workspace,
            metadata.workspace_identity,
            metadata.project_facts_hash,
            metadata.registry_key,
            metadata.authenticated_session,
            client,
        )
        .map_err(|error| error.to_string())
    }

    fn decide(
        &mut self,
        _context: &RequestContext,
        session: &mut Self::Session,
        operation: OperationRequest,
    ) -> Result<Value, String> {
        let reply: crate::ControlReply = session.request(&WorkspaceControlRequest::<
            OperationRequest,
            HostOutcomeInput,
        >::Decide {
            operation,
        })?;
        reply_value(reply, "decided")
    }

    fn apply(
        &mut self,
        _context: &RequestContext,
        session: &mut Self::Session,
        action_ref: String,
        outcome: Option<HostOutcomeInput>,
    ) -> Result<Value, String> {
        let reply: crate::ControlReply = session.request(&WorkspaceControlRequest::<
            OperationRequest,
            HostOutcomeInput,
        >::Apply {
            action_ref,
            outcome,
        })?;
        reply_value(reply, "applied")
    }
}

fn reply_value(reply: crate::ControlReply, expected: &str) -> Result<Value, String> {
    match reply {
        crate::ControlReply::Ok(response) => match (*response, expected) {
            (WorkspaceControlResponse::Decided(value), "decided") => {
                serde_json::to_value(value).map_err(|error| error.to_string())
            }
            (WorkspaceControlResponse::Applied(value), "applied") => {
                serde_json::to_value(value).map_err(|error| error.to_string())
            }
            _ => Err("control_response_mismatch".to_string()),
        },
        crate::ControlReply::Error(error) => Err(format!("{}: {}", error.code, error.detail)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolPhase {
    AwaitInitialize,
    AwaitInitialized,
    Ready,
}

#[derive(Debug)]
struct McpProtocolState {
    phase: ProtocolPhase,
    roots_supported: bool,
    roots_list_changed: bool,
    pending_roots_request: Option<String>,
    refresh_roots_after_pending: bool,
    next_roots_request: u64,
}

impl Default for McpProtocolState {
    fn default() -> Self {
        Self {
            phase: ProtocolPhase::AwaitInitialize,
            roots_supported: false,
            roots_list_changed: false,
            pending_roots_request: None,
            refresh_roots_after_pending: false,
            next_roots_request: 1,
        }
    }
}

impl McpProtocolState {
    fn handle<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
        message: &Value,
    ) -> Result<Vec<Outbound>, String> {
        let Some(object) = message.as_object() else {
            return Ok(vec![Outbound::new(error_response(
                None,
                -32600,
                "Invalid Request",
            ))]);
        };
        let method_field = object.get("method");
        let id_field = object.get("id");
        let is_notification = method_field.is_some() && id_field.is_none();
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return if is_notification {
                Ok(Vec::new())
            } else {
                let response_id = id_field.and_then(|id| RequestId::parse(id).ok());
                Ok(vec![Outbound::new(error_response(
                    response_id,
                    -32600,
                    "Invalid Request",
                ))])
            };
        }
        let method = match method_field {
            Some(Value::String(method)) => Some(method.as_str()),
            Some(_) if id_field.is_none() => return Ok(Vec::new()),
            Some(_) => {
                return Ok(vec![Outbound::new(error_response(
                    None,
                    -32600,
                    "Invalid Request",
                ))])
            }
            None => None,
        };
        if method.is_none() {
            if let Some(id_field) = id_field {
                let Ok(id) = RequestId::parse(id_field) else {
                    return Ok(Vec::new());
                };
                return self.handle_roots_response(connection, message, &id);
            }
        }
        let Some(method) = method else {
            return Ok(vec![Outbound::new(error_response(
                None,
                -32600,
                "Invalid Request",
            ))]);
        };
        let Some(id_field) = id_field else {
            return self.handle_notification(connection, method);
        };
        let Ok(id) = RequestId::parse(id_field) else {
            return Ok(vec![Outbound::new(error_response(
                None,
                -32600,
                "Invalid Request",
            ))]);
        };
        if method == "initialize" {
            return Ok(vec![Outbound::new(
                self.handle_initialize(connection, message, id),
            )]);
        }
        if method == "ping" {
            return Ok(vec![Outbound::new(ok_response(id, json!({})))]);
        }
        if self.phase != ProtocolPhase::Ready {
            return Ok(vec![Outbound::new(error_response(
                Some(id),
                -32002,
                "Server not initialized",
            ))]);
        }
        let response = match method {
            "tools/list" => ok_response(id, serde_json::to_value(list_tools()).unwrap()),
            "tools/call" => return Ok(vec![tool_call_response(connection, message, id)]),
            "resources/list" => ok_response(id, connection.listed_resources()),
            "resources/read" => {
                let Some(uri) = message
                    .get("params")
                    .and_then(Value::as_object)
                    .and_then(|params| params.get("uri"))
                    .and_then(Value::as_str)
                else {
                    return Ok(vec![Outbound::new(error_response(
                        Some(id),
                        -32602,
                        "params.uri required",
                    ))]);
                };
                match connection.read_resource(uri) {
                    Ok(resource) => ok_response(id, resource),
                    Err(error) => error_response(Some(id), -32001, &error),
                }
            }
            _ => error_response(Some(id), -32601, "Method not found"),
        };
        Ok(vec![Outbound::new(response)])
    }

    fn handle_initialize<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
        request: &Value,
        id: RequestId,
    ) -> Value {
        if self.phase != ProtocolPhase::AwaitInitialize {
            return error_response(Some(id), -32600, "initialize must be the first request");
        }
        let Some(params) = request.get("params").and_then(Value::as_object) else {
            return error_response(Some(id), -32602, "initialize params required");
        };
        if params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return error_response(Some(id), -32602, "params.protocolVersion required");
        }
        let Some(capabilities) = params.get("capabilities").and_then(Value::as_object) else {
            return error_response(Some(id), -32602, "params.capabilities required");
        };
        let Some(client_info) = params.get("clientInfo").and_then(Value::as_object) else {
            return error_response(Some(id), -32602, "params.clientInfo required");
        };
        let Some(host) = client_info.get("name").and_then(Value::as_str) else {
            return error_response(Some(id), -32602, "params.clientInfo.name required");
        };
        if client_info
            .get("version")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return error_response(Some(id), -32602, "params.clientInfo.version required");
        }
        let roots = capabilities.get("roots").and_then(Value::as_object);
        self.roots_supported = roots.is_some();
        self.roots_list_changed = roots
            .and_then(|value| value.get("listChanged"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        match connection.initialize(host) {
            Ok(()) => {
                self.phase = ProtocolPhase::AwaitInitialized;
                ok_response(
                    id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {
                            "tools": {"listChanged": false},
                            "resources": {"listChanged": false, "subscribe": false}
                        },
                        "serverInfo": {"name": "ags-mcp", "version": env!("CARGO_PKG_VERSION")}
                    }),
                )
            }
            Err(error) => error_response(Some(id), -32001, &error),
        }
    }

    fn handle_notification<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
        method: &str,
    ) -> Result<Vec<Outbound>, String> {
        match method {
            "notifications/initialized" => {
                if self.phase != ProtocolPhase::AwaitInitialized {
                    return Ok(Vec::new());
                }
                self.phase = ProtocolPhase::Ready;
                if self.roots_supported {
                    Ok(vec![Outbound::new(self.roots_request(connection)?)])
                } else {
                    Ok(Vec::new())
                }
            }
            "notifications/roots/list_changed" => {
                if self.phase != ProtocolPhase::Ready
                    || !self.roots_supported
                    || !self.roots_list_changed
                {
                    return Ok(Vec::new());
                }
                if self.pending_roots_request.is_some() {
                    connection.mark_roots_pending();
                    self.refresh_roots_after_pending = true;
                    Ok(Vec::new())
                } else {
                    Ok(vec![Outbound::new(self.roots_request(connection)?)])
                }
            }
            _ => Ok(Vec::new()),
        }
    }

    fn roots_request<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
    ) -> Result<Value, String> {
        if self.pending_roots_request.is_some() {
            return Err("roots_request_already_pending".to_string());
        }
        let id = format!("{ROOTS_REQUEST_ID_PREFIX}{}", self.next_roots_request);
        self.next_roots_request = self.next_roots_request.saturating_add(1);
        self.pending_roots_request = Some(id.clone());
        connection.mark_roots_pending();
        Ok(json!({"jsonrpc": "2.0", "id": id, "method": "roots/list"}))
    }

    fn handle_roots_response<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
        response: &Value,
        id: &RequestId,
    ) -> Result<Vec<Outbound>, String> {
        let Some(id) = id.as_string() else {
            return Ok(Vec::new());
        };
        if self.pending_roots_request.as_deref() != Some(id) {
            return Ok(Vec::new());
        }
        match (response.get("result"), response.get("error")) {
            (Some(_), None) => match parse_roots_result(response) {
                Ok(roots) => connection.update_roots(roots),
                Err(_) => return Ok(Vec::new()),
            },
            (None, Some(error)) if is_typed_json_rpc_error(error) => {
                return self.roots_unavailable(connection);
            }
            _ => return Ok(Vec::new()),
        }
        self.pending_roots_request = None;
        if self.refresh_roots_after_pending {
            self.refresh_roots_after_pending = false;
            Ok(vec![Outbound::new(self.roots_request(connection)?)])
        } else {
            Ok(Vec::new())
        }
    }

    fn roots_unavailable<P: ControlPlaneSeam>(
        &mut self,
        connection: &mut Connection<P>,
    ) -> Result<Vec<Outbound>, String> {
        self.pending_roots_request = None;
        connection.mark_roots_unavailable();
        if self.refresh_roots_after_pending {
            self.refresh_roots_after_pending = false;
            Ok(vec![Outbound::new(self.roots_request(connection)?)])
        } else {
            Ok(Vec::new())
        }
    }
}

fn is_typed_json_rpc_error(error: &Value) -> bool {
    let Some(object) = error.as_object() else {
        return false;
    };
    object
        .keys()
        .all(|key| matches!(key.as_str(), "code" | "message" | "data"))
        && object.get("code").and_then(Value::as_i64).is_some()
        && object.get("message").and_then(Value::as_str).is_some()
}

enum BoundedLine {
    Eof,
    Line(String),
    Rejected(&'static str),
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> std::io::Result<BoundedLine> {
    buffer.clear();
    let mut rejected = None;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if buffer.is_empty() && rejected.is_none() {
                return Ok(BoundedLine::Eof);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let content = newline.map_or(available, |index| &available[..index]);
        if rejected.is_none() {
            if buffer.len().saturating_add(content.len()) > MAX_MCP_MESSAGE_BYTES {
                buffer.clear();
                rejected = Some("JSON-RPC message exceeds 1 MiB");
            } else {
                buffer.extend_from_slice(content);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if let Some(reason) = rejected {
        return Ok(BoundedLine::Rejected(reason));
    }
    if buffer.last() == Some(&b'\r') {
        buffer.pop();
    }
    match String::from_utf8(std::mem::take(buffer)) {
        Ok(line) => Ok(BoundedLine::Line(line)),
        Err(_) => Ok(BoundedLine::Rejected("JSON-RPC message is not UTF-8")),
    }
}

pub fn serve<R: BufRead, W: Write, P: ControlPlaneSeam>(
    reader: R,
    writer: W,
    adapter_cwd: PathBuf,
    port: P,
) -> Result<(), String> {
    let mut connection = Connection::new(adapter_cwd, port);
    serve_connection(reader, writer, &mut connection)
}

fn serve_connection<R: BufRead, W: Write, P: ControlPlaneSeam>(
    mut reader: R,
    mut writer: W,
    connection: &mut Connection<P>,
) -> Result<(), String> {
    let mut state = McpProtocolState::default();
    let mut line_buffer = Vec::with_capacity(8 * 1024);
    loop {
        let line = match read_bounded_line(&mut reader, &mut line_buffer)
            .map_err(|error| format!("stdio read failed: {error}"))?
        {
            BoundedLine::Eof => break,
            BoundedLine::Line(line) => line,
            BoundedLine::Rejected(reason) => {
                deliver_outbound(
                    &mut writer,
                    connection,
                    Outbound::new(error_response(None, -32700, reason)),
                )?;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = Outbound::new(error_response(
                    None,
                    -32700,
                    &format!("invalid JSON-RPC request: {error}"),
                ));
                deliver_outbound(&mut writer, connection, response)?;
                continue;
            }
        };
        for response in state.handle(connection, &request)? {
            deliver_outbound(&mut writer, connection, response)?;
        }
    }
    Ok(())
}

fn deliver_outbound<W: Write, P: ControlPlaneSeam>(
    writer: &mut W,
    connection: &mut Connection<P>,
    outbound: Outbound,
) -> Result<(), String> {
    writeln!(writer, "{}", outbound.envelope)
        .and_then(|_| writer.flush())
        .map_err(|error| format!("stdio write failed: {error}"))?;
    if let Some(action_ref) = outbound.terminal_delivery {
        connection.complete_terminal_delivery(&action_ref);
    }
    Ok(())
}

fn parse_roots_result(response: &Value) -> Result<Vec<PathBuf>, String> {
    let roots = response
        .pointer("/result/roots")
        .and_then(Value::as_array)
        .ok_or_else(|| "roots_result_invalid".to_string())?;
    if roots.len() > MAX_MCP_ROOTS {
        return Err("roots_limit_exceeded".to_string());
    }
    let mut paths = Vec::with_capacity(roots.len());
    let mut unique = HashSet::with_capacity(roots.len());
    for root in roots {
        let object = root
            .as_object()
            .ok_or_else(|| "root_entry_invalid".to_string())?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "uri" | "name" | "_meta"))
        {
            return Err("root_entry_unknown_field".to_string());
        }
        let uri = object
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| "root_uri_required".to_string())?;
        if object.get("name").is_some_and(|name| !name.is_string()) {
            return Err("root_name_invalid".to_string());
        }
        if object.get("_meta").is_some_and(|meta| !meta.is_object()) {
            return Err("root_meta_invalid".to_string());
        }
        if uri.len() > MAX_ROOT_URI_BYTES {
            return Err("root_uri_limit_exceeded".to_string());
        }
        let decoded = decode_file_uri(uri).ok_or_else(|| "root_uri_invalid".to_string())?;
        let canonical = PathBuf::from(decoded)
            .canonicalize()
            .map_err(|error| format!("root_uri_unreadable: {error}"))?;
        if !unique.insert(canonical.clone()) {
            return Err("root_uri_duplicate".to_string());
        }
        paths.push(canonical);
    }
    Ok(paths)
}

pub(crate) fn decode_file_uri(uri: &str) -> Option<String> {
    let encoded = uri.strip_prefix("file://")?;
    let encoded = encoded.strip_prefix("localhost").unwrap_or(encoded);
    if !encoded.starts_with('/') || encoded.contains(['?', '#']) {
        return None;
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = *bytes.get(index + 1)?;
            let low = *bytes.get(index + 2)?;
            decoded.push(hex(high)? * 16 + hex(low)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    (!decoded.contains(&0))
        .then(|| String::from_utf8(decoded).ok())
        .flatten()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn decode_hex_into(value: &str, output: &mut Vec<u8>) -> Result<(), String> {
    if !value.len().is_multiple_of(2) {
        return Err("details_chunk_hex_length_invalid".to_string());
    }
    for pair in value.as_bytes().chunks_exact(2) {
        let high = hex(pair[0]).ok_or_else(|| "details_chunk_hex_invalid".to_string())?;
        let low = hex(pair[1]).ok_or_else(|| "details_chunk_hex_invalid".to_string())?;
        output.push(high * 16 + low);
    }
    Ok(())
}

fn tool_call_response<P: ControlPlaneSeam>(
    connection: &mut Connection<P>,
    request: &Value,
    id: RequestId,
) -> Outbound {
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Outbound::new(error_response(Some(id), -32602, "params.name required"));
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut applied_action_ref = None;
    let result = match name {
        TOOL_DECIDE => match serde_json::from_value::<DecideArguments>(arguments) {
            Ok(arguments) => connection.decide(arguments),
            Err(error) => {
                return Outbound::new(error_response(
                    Some(id),
                    -32602,
                    &format!("invalid ags_decide arguments: {error}"),
                ))
            }
        },
        TOOL_APPLY => match serde_json::from_value::<ApplyArguments>(arguments) {
            Ok(arguments) => {
                applied_action_ref = Some(arguments.action_ref.clone());
                connection.apply(arguments)
            }
            Err(error) => {
                return Outbound::new(error_response(
                    Some(id),
                    -32602,
                    &format!("invalid ags_apply arguments: {error}"),
                ))
            }
        },
        _ => {
            return Outbound::new(error_response(
                Some(id),
                -32602,
                &format!("unknown tool `{name}`"),
            ))
        }
    };
    match result {
        Ok(value) => match bounded_tool_result(&value) {
            Ok(result) => Outbound::with_terminal_delivery(
                ok_response(id, result),
                applied_action_ref.filter(|_| {
                    apply_response_disposition(&value) == ApplyResponseDisposition::Terminal
                }),
            ),
            Err(error) => Outbound::new(tool_error_response(id, &error)),
        },
        Err(error) => Outbound::new(tool_error_response(id, &error)),
    }
}

fn bounded_tool_result(value: &Value) -> Result<Value, String> {
    let encoded =
        serde_json::to_vec(value).map_err(|error| format!("result_encode_failed: {error}"))?;
    if encoded.len() <= MAX_INLINE_STRUCTURED_BYTES {
        let result = json!({
            "content": [{"type": "text", "text": concise_summary(value)}],
            "structuredContent": value,
            "isError": false
        });
        if serde_json::to_vec(&result).is_ok_and(|bytes| bytes.len() <= MAX_MCP_RESULT_BYTES) {
            return Ok(result);
        }
    }

    Err("result_too_large: details.read authorization is unavailable".to_string())
}

fn concise_summary(value: &Value) -> String {
    let mut lines = Vec::new();
    for key in ["state", "action_ref", "reason_code", "details_uri"] {
        if let Some(field) = value.get(key).and_then(Value::as_str) {
            lines.push(format!("{key}: {field}"));
        }
    }
    if lines.is_empty() {
        "AGS operation completed".to_string()
    } else {
        lines.join("\n")
    }
}

fn tool_error_response(id: RequestId, error: &str) -> Value {
    ok_response(
        id,
        json!({
            "content": [{"type": "text", "text": truncate_utf8(error, MAX_ERROR_TEXT_BYTES)}],
            "isError": true
        }),
    )
}

fn ok_response(id: RequestId, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.into_value(), "result": result})
}

fn error_response(id: Option<RequestId>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.map(RequestId::into_value).unwrap_or(Value::Null),
        "error": {"code": code, "message": truncate_utf8(message, MAX_ERROR_TEXT_BYTES)}
    })
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

pub fn list_tools() -> ToolListResult {
    ToolListResult {
        tools: vec![
            ToolDef {
                name: TOOL_DECIDE.to_string(),
                description: Some("Decide one typed contract-v2 Operation.".to_string()),
                inputSchema: json!({
                    "type": "object",
                    "required": ["operation"],
                    "additionalProperties": false,
                    "properties": {
                        "operation": operation_request_schema()
                    }
                }),
            },
            ToolDef {
                name: TOOL_APPLY.to_string(),
                description: Some("Consume one connection-bound action_ref.".to_string()),
                inputSchema: json!({
                    "type": "object",
                    "required": ["action_ref"],
                    "additionalProperties": false,
                    "properties": {
                        "action_ref": {"type": "string", "minLength": 1},
                        "outcome": host_outcome_input_schema()
                    }
                }),
            },
        ],
    }
}

pub fn tool_schema_bytes() -> usize {
    serde_json::to_vec(&list_tools()).map_or(usize::MAX, |bytes| bytes.len())
}

pub fn schemas_within_budget() -> bool {
    tool_schema_bytes() <= TOOL_SCHEMA_BUDGET
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io;
    use std::path::Path;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    fn serve_transcript<P: ControlPlaneSeam>(
        input: Vec<Value>,
        cwd: PathBuf,
        port: P,
    ) -> Vec<Value> {
        let wire = input
            .into_iter()
            .map(|message| serde_json::to_string(&message).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut output = Vec::new();
        serve(std::io::Cursor::new(wire), &mut output, cwd, port).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    struct FailOnThirdFlush {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for FailOnThirdFlush {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.flushes == 3 {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected flush failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    fn workspace(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&path)
            .status()
            .unwrap()
            .success());
        std::fs::write(path.join("AGENTS.md"), "# governed workspace\n").unwrap();
        std::fs::create_dir_all(path.join("config")).unwrap();
        std::fs::write(
            path.join("config/agent-project-profile.yaml"),
            "schema_version: ags://schema/contract/v2/project-profile\n",
        )
        .unwrap();
        path.canonicalize().unwrap()
    }

    fn operation(name: &str) -> OperationRequest {
        operation_at(name, None)
    }

    fn initialize_connection<P: ControlPlaneSeam>(
        connection: &mut Connection<P>,
        host: &str,
        roots: Vec<PathBuf>,
    ) {
        connection.initialize(host).unwrap();
        connection.update_roots(roots);
    }

    fn request_id(value: Value) -> RequestId {
        RequestId::parse(&value).unwrap()
    }

    fn operation_at(name: &str, workspace: Option<&Path>) -> OperationRequest {
        let context = workspace
            .map(|path| json!({"workspace": path}))
            .unwrap_or_else(|| json!({}));
        let request = match name {
            "doctor" => json!({"context": context, "scope": "all"}),
            "setup" => json!({"context": context, "approved_hosts": []}),
            _ => panic!("unsupported test operation"),
        };
        serde_json::from_value(json!({"operation": name, "request": request})).unwrap()
    }

    #[derive(Default)]
    struct State {
        actions: BTreeMap<String, RequestContext>,
        open_counts: BTreeMap<PathBuf, usize>,
        seen: Vec<PathBuf>,
        block_without_outcome: bool,
        effect_calls: usize,
        large_terminal: bool,
        details_payload: Option<Vec<u8>>,
    }

    #[derive(Clone, Default)]
    struct Port(Arc<Mutex<State>>);

    impl ControlPlaneSeam for Port {
        type Session = ();

        fn open(
            &mut self,
            context: &RequestContext,
        ) -> Result<AuthenticatedWorkspaceSession<Self::Session>, String> {
            *self
                .0
                .lock()
                .unwrap()
                .open_counts
                .entry(context.workspace.clone())
                .or_default() += 1;
            AuthenticatedWorkspaceSession::new(
                &context.workspace,
                format!(
                    "workspace-{}",
                    ags_platform::sha256(context.workspace.to_string_lossy().as_bytes())
                ),
                ags_session::WorkspaceState::new(context.workspace.clone(), PathBuf::new())?
                    .project_facts_hash(),
                format!(
                    "registry-{}",
                    ags_platform::sha256(context.workspace.to_string_lossy().as_bytes())
                ),
                format!(
                    "session-{}",
                    ags_platform::sha256(context.workspace.to_string_lossy().as_bytes())
                ),
                (),
            )
            .map_err(|error| error.to_string())
        }

        fn decide(
            &mut self,
            context: &RequestContext,
            _session: &mut Self::Session,
            operation: OperationRequest,
        ) -> Result<Value, String> {
            let mut state = self.0.lock().unwrap();
            if let OperationRequest::DetailsRead(request) = operation {
                let bytes = state
                    .details_payload
                    .as_ref()
                    .ok_or_else(|| "details fixture is unavailable".to_string())?;
                let start = usize::try_from(request.offset)
                    .map_err(|_| "details offset overflow".to_string())?;
                let end = start
                    .saturating_add(request.max_bytes as usize)
                    .min(bytes.len());
                let data = bytes
                    .get(start..end)
                    .ok_or_else(|| "details offset is outside fixture".to_string())?
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                return Ok(json!({
                    "result": {
                        "artifact": request.artifact,
                        "offset": request.offset,
                        "next_offset": end,
                        "byte_length": bytes.len(),
                        "eof": end == bytes.len(),
                        "encoding": "hex",
                        "data": data
                    }
                }));
            }
            state.seen.push(context.workspace.clone());
            let action_ref = format!("action-{}", state.seen.len());
            state.actions.insert(action_ref.clone(), context.clone());
            Ok(
                json!({"operation": operation.name().as_str(), "workspace": context.workspace, "action_ref": action_ref}),
            )
        }

        fn apply(
            &mut self,
            context: &RequestContext,
            _session: &mut Self::Session,
            action_ref: String,
            outcome: Option<HostOutcomeInput>,
        ) -> Result<Value, String> {
            let mut state = self.0.lock().unwrap();
            state.effect_calls = state.effect_calls.saturating_add(1);
            let sealed = state
                .actions
                .get(&action_ref)
                .cloned()
                .ok_or_else(|| "action_ref_invalid_or_consumed".to_string())?;
            if sealed != *context {
                return Err("action_ref_binding_mismatch".to_string());
            }
            if state.block_without_outcome && outcome.is_none() {
                return Ok(json!({
                    "state": "awaiting-outcome",
                    "reason_code": "host_outcome_required"
                }));
            }
            state.actions.remove(&action_ref);
            if state.large_terminal {
                Ok(json!({
                    "state": "receipted",
                    "receipt": {"receipt_id": "receipt-1"},
                    "payload": "x".repeat(32 * 1024)
                }))
            } else {
                Ok(json!({"state": "receipted"}))
            }
        }
    }

    #[test]
    fn tools_list_is_exact_and_under_eight_kib() {
        let first = list_tools();
        let second = list_tools();
        assert_eq!(
            first
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            [TOOL_DECIDE, TOOL_APPLY]
        );
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(
            first.tools[0].inputSchema["properties"]["operation"],
            operation_request_schema()
        );
        assert_eq!(
            first.tools[1].inputSchema["properties"]["outcome"],
            host_outcome_input_schema()
        );
        let decide_schema = serde_json::to_string(&first.tools[0].inputSchema).unwrap();
        assert!(
            !decide_schema.contains("host.lifecycle") && !decide_schema.contains("details.read"),
            "tools/list must expose only ProductCli Operations"
        );
        assert!(first.tools[1].inputSchema["properties"]
            .get("workspace")
            .is_none());
        assert!(schemas_within_budget(), "{} bytes", tool_schema_bytes());
    }

    #[test]
    fn internal_details_operation_is_rejected_by_public_decide_surface() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port);
        initialize_connection(&mut connection, "hermes", vec![a]);
        let error = connection
            .decide(DecideArguments {
                operation: OperationRequest::DetailsRead(DetailsReadRequest {
                    context: OperationContext::default(),
                    artifact: ContentAddressedArtifactRef {
                        uri: "ags://details/hidden".to_string(),
                        sha256: ags_platform::sha256("hidden"),
                    },
                    offset: 0,
                    max_bytes: DETAILS_CHUNK_LIMIT,
                }),
            })
            .unwrap_err();
        assert!(error.starts_with("operation_surface_forbidden: details.read"));
    }

    #[test]
    fn one_connection_routes_a_b_a_with_two_opens_and_one_warm_hit() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port.clone());
        initialize_connection(
            &mut connection,
            "Hermes Agent.v2",
            vec![a.clone(), b.clone()],
        );
        for target in [&a, &b, &a] {
            connection
                .decide(DecideArguments {
                    operation: operation_at("doctor", Some(target)),
                })
                .unwrap();
        }
        let state = port.0.lock().unwrap();
        assert_eq!(state.seen, [a.clone(), b.clone(), a]);
        assert_eq!(state.open_counts.get(&b), Some(&1));
        assert_eq!(connection.handshake_count(), 2);
        assert_eq!(connection.open_count(), 2);
        assert_eq!(connection.warm_hit_count(), 1);
    }

    #[test]
    fn omitted_workspace_uses_unique_root_and_rejects_ambiguity() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let port = Port::default();
        let mut unique = Connection::new(home, port.clone());
        initialize_connection(&mut unique, "hermes", vec![a.clone()]);
        unique
            .decide(DecideArguments {
                operation: operation("doctor"),
            })
            .unwrap();
        assert_eq!(
            port.0.lock().unwrap().seen.as_slice(),
            std::slice::from_ref(&a)
        );
        let mut ambiguous = Connection::new(a, Port::default());
        initialize_connection(
            &mut ambiguous,
            "hermes",
            vec![workspace(temp.path(), "c"), b],
        );
        assert!(ambiguous
            .decide(DecideArguments {
                operation: operation("doctor")
            })
            .unwrap_err()
            .starts_with("workspace_ambiguous"));
    }

    #[test]
    fn roots_result_decodes_a_unique_file_uri_root() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let workspace = workspace(temp.path(), "workspace with space");
        let uri = format!("file://{}", workspace.display()).replace(' ', "%20");
        let roots = parse_roots_result(&json!({"result": {"roots": [{"uri": uri}]}})).unwrap();
        let port = Port::default();
        let mut connection = Connection::new(home, port.clone());
        initialize_connection(&mut connection, "Hermes", roots);
        connection
            .decide(DecideArguments {
                operation: operation("doctor"),
            })
            .unwrap();
        assert_eq!(port.0.lock().unwrap().seen, [workspace]);
    }

    #[test]
    fn standard_initialized_roots_round_trip_ignores_private_initialize_roots() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let a = workspace(temp.path(), "a");
        let private_wire_root = workspace(temp.path(), "private-wire-must-be-ignored");
        let port = Port::default();
        let output = serve_transcript(
            vec![
                json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"roots": {"listChanged": true}},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"},
                        "roots": [{"path": private_wire_root}]
                    }
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({
                    "jsonrpc": "2.0",
                    "id": "ags-roots-1",
                    "result": {"roots": [{"uri": format!("file://{}", a.display())}]}
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": TOOL_DECIDE,
                        "arguments": {"operation": operation("doctor")}
                    }
                }),
            ],
            home,
            port.clone(),
        );

        assert_eq!(output.len(), 3, "{output:?}");
        assert_eq!(output[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(output[1]["id"], "ags-roots-1");
        assert_eq!(output[1]["method"], "roots/list");
        assert_eq!(output[2]["result"]["isError"], false, "{output:?}");
        assert_eq!(port.0.lock().unwrap().seen, [a]);
    }

    #[test]
    fn roots_list_changed_is_coalesced_while_pending_and_refreshes_once() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let port = Port::default();
        let output = serve_transcript(
            vec![
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"roots": {"listChanged": true}},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "method": "notifications/roots/list_changed"}),
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "result": {"roots": [{"uri": format!("file://{}", a.display())}]}
                }),
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-2",
                    "result": {"roots": [{"uri": format!("file://{}", b.display())}]}
                }),
                json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {
                        "name": TOOL_DECIDE,
                        "arguments": {"operation": operation("doctor")}
                    }
                }),
            ],
            home,
            port.clone(),
        );

        assert_eq!(output.len(), 4, "{output:?}");
        assert_eq!(output[1]["id"], "ags-roots-1");
        assert_eq!(output[2]["id"], "ags-roots-2");
        assert_eq!(output[3]["result"]["isError"], false, "{output:?}");
        assert_eq!(port.0.lock().unwrap().seen, [b]);
    }

    #[test]
    fn roots_pending_blocks_only_omitted_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let output = serve_transcript(
            vec![
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"roots": {"listChanged": true}},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {
                        "name": TOOL_DECIDE,
                        "arguments": {"operation": operation("doctor")}
                    }
                }),
                json!({
                    "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {
                        "name": TOOL_DECIDE,
                        "arguments": {"operation": operation_at("doctor", Some(&a))}
                    }
                }),
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "result": {"roots": [{"uri": format!("file://{}", a.display())}]}
                }),
            ],
            a.clone(),
            Port::default(),
        );

        assert_eq!(output.len(), 4, "{output:?}");
        assert_eq!(output[2]["result"]["isError"], true);
        assert!(output[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("workspace_roots_pending"));
        assert_eq!(output[3]["result"]["isError"], false, "{output:?}");
    }

    #[test]
    fn non_pending_root_states_with_no_valid_root_may_use_adapter_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        for roots_state in [
            RootsDiscovery::NotNegotiated,
            RootsDiscovery::Unavailable,
            RootsDiscovery::Available,
        ] {
            let port = Port::default();
            let mut connection = Connection::new(a.clone(), port.clone());
            connection.initialize("Generic Agent").unwrap();
            match roots_state {
                RootsDiscovery::Unavailable => connection.mark_roots_unavailable(),
                RootsDiscovery::Available => connection.update_roots(Vec::new()),
                RootsDiscovery::NotNegotiated => {}
                RootsDiscovery::Pending => unreachable!(),
            }
            connection
                .decide(DecideArguments {
                    operation: operation("doctor"),
                })
                .unwrap();
            assert_eq!(
                port.0.lock().unwrap().seen.as_slice(),
                std::slice::from_ref(&a)
            );
        }
    }

    #[test]
    fn roots_parser_rejects_non_file_duplicate_unknown_untyped_and_over_limit_entries() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let uri = format!("file://{}", a.display());
        for (response, expected) in [
            (
                json!({"result": {"roots": [{"uri": "https://example.com"}]}}),
                "root_uri_invalid",
            ),
            (
                json!({"result": {"roots": [{"path": a}]}}),
                "root_entry_unknown_field",
            ),
            (
                json!({"result": {"roots": [{"uri": uri, "name": 7}]}}),
                "root_name_invalid",
            ),
            (
                json!({"result": {"roots": [{"uri": uri, "_meta": "nope"}]}}),
                "root_meta_invalid",
            ),
            (
                json!({"result": {"roots": [{"uri": uri}, {"uri": uri}]}}),
                "root_uri_duplicate",
            ),
            (
                json!({"result": {"roots": (0..=MAX_MCP_ROOTS).map(|_| json!({"uri": uri})).collect::<Vec<_>>()}}),
                "roots_limit_exceeded",
            ),
        ] {
            assert_eq!(parse_roots_result(&response).unwrap_err(), expected);
        }
    }

    #[test]
    fn file_root_uri_decoder_rejects_authority_query_fragment_nul_and_size_overflow() {
        assert_eq!(
            decode_file_uri("file:///tmp/a%20b").as_deref(),
            Some("/tmp/a b")
        );
        assert_eq!(
            decode_file_uri("file://localhost/tmp/a").as_deref(),
            Some("/tmp/a")
        );
        for invalid in [
            "https://example.com/root",
            "file://remote.example/tmp/root",
            "file:///tmp/root?query=1",
            "file:///tmp/root#fragment",
            "file:///tmp/root%00suffix",
            "file:///tmp/root%ZZ",
        ] {
            assert_eq!(decode_file_uri(invalid), None, "{invalid}");
        }
        let oversized = format!("file:///{}", "x".repeat(MAX_ROOT_URI_BYTES));
        let error =
            parse_roots_result(&json!({"result": {"roots": [{"uri": oversized}]}})).unwrap_err();
        assert_eq!(error, "root_uri_limit_exceeded");
    }

    #[test]
    fn duplicate_initialize_is_rejected_and_out_of_order_notifications_do_not_mutate_state() {
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "Generic Agent", "version": "1.0"}
            }
        });
        let output = serve_transcript(
            vec![initialize.clone(), initialize],
            std::env::current_dir().unwrap(),
            Port::default(),
        );
        assert_eq!(output[1]["error"]["code"], -32600);

        let output = serve_transcript(
            vec![
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "method": "notifications/roots/list_changed"}),
                json!({"jsonrpc": "2.0", "method": 7}),
                json!({
                    "jsonrpc": "2.0", "id": 3, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}),
            ],
            std::env::current_dir().unwrap(),
            Port::default(),
        );
        assert_eq!(output.len(), 2, "{output:?}");
        assert_eq!(output[0]["id"], 3);
        assert_eq!(output[1]["id"], 4);
        assert!(output[1]["result"]["tools"].is_array());
    }

    #[test]
    fn unfamiliar_response_id_is_ignored_without_adopting_or_killing_roots_state() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let output = serve_transcript(
            vec![
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"roots": {}},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": "ags-roots-unknown", "result": {"roots": []}}),
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "result": {"roots": [{"uri": format!("file://{}", a.display())}]}
                }),
                json!({
                    "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                    "params": {
                        "name": TOOL_DECIDE,
                        "arguments": {"operation": operation("doctor")}
                    }
                }),
            ],
            temp.path().join("home"),
            Port::default(),
        );
        assert_eq!(output.len(), 3, "{output:?}");
        assert_eq!(output[2]["result"]["isError"], false, "{output:?}");
    }

    #[test]
    fn roots_error_or_invalid_result_keeps_explicit_usable_and_omitted_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let a = workspace(temp.path(), "a");
        for (roots_response, expected_error) in [
            (
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "error": {"code": -32603, "message": "roots unavailable"}
                }),
                "workspace_roots_unavailable",
            ),
            (
                json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "result": {"roots": [{"uri": "https://not-a-file-root.example"}]}
                }),
                "workspace_roots_pending",
            ),
        ] {
            let output = serve_transcript(
                vec![
                    json!({
                        "jsonrpc": "2.0", "id": 1, "method": "initialize",
                        "params": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {"roots": {}},
                            "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                        }
                    }),
                    json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                    roots_response,
                    json!({
                        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                        "params": {
                            "name": TOOL_DECIDE,
                            "arguments": {"operation": operation("doctor")}
                        }
                    }),
                    json!({
                        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                        "params": {
                            "name": TOOL_DECIDE,
                            "arguments": {"operation": operation_at("doctor", Some(&a))}
                        }
                    }),
                ],
                home.clone(),
                Port::default(),
            );
            assert_eq!(output.len(), 4, "{output:?}");
            assert!(output[2]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(expected_error));
            assert_eq!(output[3]["result"]["isError"], false, "{output:?}");
        }
    }

    #[test]
    fn matching_roots_response_requires_result_xor_a_typed_json_rpc_error() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        for malformed in [
            json!({
                "jsonrpc": "2.0", "id": "ags-roots-1",
                "result": {"roots": [{"uri": format!("file://{}", a.display())}]},
                "error": {"code": -32603, "message": "both are forbidden"}
            }),
            json!({
                "jsonrpc": "2.0", "id": "ags-roots-1",
                "error": {"code": "not-a-number", "message": "bad"}
            }),
            json!({
                "jsonrpc": "2.0", "id": "ags-roots-1",
                "result": {"roots": [{"uri": "https://untrusted.example"}]}
            }),
            json!({"jsonrpc": "2.0", "id": "ags-roots-1"}),
        ] {
            let mut connection = Connection::new(a.clone(), Port::default());
            let mut state = McpProtocolState::default();
            let initialize = json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"roots": {}},
                    "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                }
            });
            state.handle(&mut connection, &initialize).unwrap();
            state
                .handle(
                    &mut connection,
                    &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                )
                .unwrap();
            state.handle(&mut connection, &malformed).unwrap();
            assert_eq!(connection.roots_discovery, RootsDiscovery::Pending);
            assert_eq!(state.pending_roots_request.as_deref(), Some("ags-roots-1"));
            assert!(connection.roots.is_empty());

            state
                .handle(
                    &mut connection,
                    &json!({
                        "jsonrpc": "2.0", "id": "ags-roots-1",
                        "result": {"roots": [{"uri": format!("file://{}", a.display())}]}
                    }),
                )
                .unwrap();
            assert_eq!(connection.roots_discovery, RootsDiscovery::Available);
            assert_eq!(connection.roots.as_slice(), std::slice::from_ref(&a));
        }

        let mut connection = Connection::new(a.clone(), Port::default());
        let mut state = McpProtocolState::default();
        state
            .handle(
                &mut connection,
                &json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"roots": {}},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
            )
            .unwrap();
        state
            .handle(
                &mut connection,
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            )
            .unwrap();
        state
            .handle(
                &mut connection,
                &json!({
                    "jsonrpc": "2.0", "id": "ags-roots-1",
                    "error": {"code": -32603, "message": "roots unavailable"}
                }),
            )
            .unwrap();
        assert_eq!(connection.roots_discovery, RootsDiscovery::Unavailable);
        assert!(state.pending_roots_request.is_none());
    }

    #[test]
    fn oversized_json_rpc_line_is_rejected_and_parser_resynchronizes() {
        let oversized = format!("{{\"padding\":\"{}\"}}", "x".repeat(1024 * 1024));
        let initialize = serde_json::to_string(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "Generic Agent", "version": "1.0"}
            }
        }))
        .unwrap();
        let mut output = Vec::new();
        serve(
            std::io::Cursor::new(format!("{oversized}\n{initialize}\n")),
            &mut output,
            std::env::current_dir().unwrap(),
            Port::default(),
        )
        .unwrap();
        let messages = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert_eq!(messages[0]["error"]["code"], -32700);
        assert_eq!(messages[1]["result"]["protocolVersion"], "2025-06-18");
    }

    #[test]
    fn terminal_route_is_acknowledged_only_after_response_flush_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port.clone());
        let mut protocol = McpProtocolState::default();
        let initialize = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "Generic Agent", "version": "1.0"}
            }
        });
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let decide = json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {
                "name": TOOL_DECIDE,
                "arguments": {"operation": operation_at("setup", Some(&a))}
            }
        });
        let apply = |id| {
            json!({
                "jsonrpc": "2.0", "method": "tools/call",
                "id": id,
                "params": {"name": TOOL_APPLY, "arguments": {"action_ref": "action-1"}}
            })
        };
        let mut writer = FailOnThirdFlush {
            bytes: Vec::new(),
            flushes: 0,
        };

        for message in [&initialize, &initialized, &decide] {
            for outbound in protocol.handle(&mut connection, message).unwrap() {
                deliver_outbound(&mut writer, &mut connection, outbound).unwrap();
            }
        }
        assert_eq!(port.0.lock().unwrap().effect_calls, 0);
        let first_apply = apply(3);
        let outbound = protocol
            .handle(&mut connection, &first_apply)
            .unwrap()
            .pop()
            .unwrap();
        let terminal = outbound.envelope.clone();
        let error = deliver_outbound(&mut writer, &mut connection, outbound).unwrap_err();

        assert!(error.contains("injected flush failure"), "{error}");
        assert_eq!(port.0.lock().unwrap().effect_calls, 1);
        assert_eq!(connection.routed_action_count(), 1);

        let mut retry_wire = Vec::new();
        let retry_apply = apply(4);
        for outbound in protocol.handle(&mut connection, &retry_apply).unwrap() {
            assert_eq!(outbound.envelope["result"], terminal["result"]);
            deliver_outbound(&mut retry_wire, &mut connection, outbound).unwrap();
        }
        assert_eq!(port.0.lock().unwrap().effect_calls, 1);
        assert_eq!(connection.routed_action_count(), 0);

        let mut replay_wire = Vec::new();
        let replay_apply = apply(5);
        for outbound in protocol.handle(&mut connection, &replay_apply).unwrap() {
            assert_eq!(outbound.envelope["result"]["isError"], true);
            assert!(outbound.envelope["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("action_ref_invalid"));
            deliver_outbound(&mut replay_wire, &mut connection, outbound).unwrap();
        }
        assert_eq!(port.0.lock().unwrap().effect_calls, 1);
    }

    #[test]
    fn json_rpc_envelope_and_request_ids_are_typed_and_bounded() {
        let huge_id = "i".repeat(20 * 1024);
        let output = serve_transcript(
            vec![
                json!([]),
                json!({"jsonrpc": "2.0", "method": 7}),
                json!({
                    "jsonrpc": "1.0", "id": 1, "method": "initialize",
                    "params": {}
                }),
                json!({
                    "jsonrpc": "2.0", "id": true, "method": "initialize",
                    "params": {}
                }),
                json!({
                    "jsonrpc": "2.0", "id": huge_id, "method": "initialize",
                    "params": {}
                }),
                json!({
                    "jsonrpc": "2.0", "id": 9, "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "Generic Agent", "version": "1.0"}
                    }
                }),
            ],
            std::env::current_dir().unwrap(),
            Port::default(),
        );

        assert_eq!(output.len(), 5, "{output:?}");
        for invalid in &output[..4] {
            assert_eq!(invalid["error"]["code"], -32600, "{invalid:?}");
            assert!(serde_json::to_vec(invalid).unwrap().len() <= MAX_MCP_RESULT_BYTES);
        }
        assert_eq!(output[0]["id"], Value::Null);
        assert_eq!(output[1]["id"], 1);
        assert_eq!(output[2]["id"], Value::Null);
        assert_eq!(output[3]["id"], Value::Null);
        assert_eq!(output[4]["id"], 9);
    }

    #[test]
    fn action_ref_rejects_workspace_connection_host_tamper_session_and_replay() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let port = Port::default();
        let mut first = Connection::new(a.clone(), port.clone());
        initialize_connection(&mut first, "hermes", vec![a.clone(), b.clone()]);
        let decision = first
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap();
        let action_ref = decision["action_ref"].as_str().unwrap().to_string();
        let mut other_workspace = Connection::new(b.clone(), port.clone());
        initialize_connection(&mut other_workspace, "hermes", vec![b]);
        assert_eq!(
            other_workspace
                .apply(ApplyArguments {
                    action_ref: action_ref.clone(),
                    outcome: None
                })
                .unwrap_err(),
            "action_ref_invalid"
        );
        assert_eq!(
            first
                .apply(ApplyArguments {
                    action_ref: format!("{action_ref}-tampered"),
                    outcome: None
                })
                .unwrap_err(),
            "action_ref_invalid"
        );
        first
            .apply(ApplyArguments {
                action_ref: action_ref.clone(),
                outcome: None,
            })
            .unwrap();
        first.complete_terminal_delivery(&action_ref);
        assert_eq!(
            first
                .apply(ApplyArguments {
                    action_ref,
                    outcome: None
                })
                .unwrap_err(),
            "action_ref_invalid"
        );

        for host in ["hermes", "other-host"] {
            let decision = first
                .decide(DecideArguments {
                    operation: operation_at("setup", Some(&a)),
                })
                .unwrap();
            let mut other = Connection::new(a.clone(), port.clone());
            initialize_connection(&mut other, host, vec![a.clone()]);
            assert_eq!(
                other
                    .apply(ApplyArguments {
                        action_ref: decision["action_ref"].as_str().unwrap().to_string(),
                        outcome: None
                    })
                    .unwrap_err(),
                "action_ref_invalid"
            );
        }
    }

    #[test]
    fn action_ref_rejects_an_evicted_authenticated_session() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port);
        initialize_connection(&mut connection, "hermes", vec![]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();

        for index in 0..ags_session::MAX_WORKSPACE_SESSIONS {
            let other = workspace(temp.path(), &format!("other-{index}"));
            connection
                .decide(DecideArguments {
                    operation: operation_at("doctor", Some(&other)),
                })
                .unwrap();
        }

        assert!(connection
            .apply(ApplyArguments {
                action_ref,
                outcome: None,
            })
            .unwrap_err()
            .starts_with("workspace_binding_rejected:"));
    }

    #[test]
    fn routed_actions_are_bounded_and_oldest_is_evicted_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let mut connection = Connection::new(a.clone(), Port::default());
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let mut first = None;
        for _ in 0..=MAX_ROUTED_ACTIONS {
            let action_ref = connection
                .decide(DecideArguments {
                    operation: operation_at("setup", Some(&a)),
                })
                .unwrap()["action_ref"]
                .as_str()
                .unwrap()
                .to_string();
            first.get_or_insert(action_ref);
        }

        assert_eq!(connection.routed_action_count(), MAX_ROUTED_ACTIONS);
        assert_eq!(connection.action_eviction_count(), 1);
        assert_eq!(
            connection
                .apply(ApplyArguments {
                    action_ref: first.unwrap(),
                    outcome: None,
                })
                .unwrap_err(),
            "action_ref_invalid"
        );
    }

    #[test]
    fn stale_session_epoch_expires_its_routed_action() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let mut connection = Connection::new(a.clone(), Port::default());
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let old_action = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();

        std::fs::write(a.join("AGENTS.md"), "# governed workspace v2\n").unwrap();
        connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap();
        let before = connection.routed_action_count();
        assert!(connection
            .apply(ApplyArguments {
                action_ref: old_action,
                outcome: None,
            })
            .unwrap_err()
            .starts_with("workspace_binding_rejected:"));
        assert_eq!(connection.routed_action_count(), before);
        assert_eq!(connection.stale_reopen_count(), 1);
    }

    #[test]
    fn awaiting_host_outcome_retains_a_routed_action() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        port.0.lock().unwrap().block_without_outcome = true;
        let mut connection = Connection::new(a.clone(), port);
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();

        let blocked = connection
            .apply(ApplyArguments {
                action_ref,
                outcome: None,
            })
            .unwrap();
        assert_eq!(blocked["state"], "awaiting-outcome");
        assert_eq!(blocked["reason_code"], "host_outcome_required");
        assert_eq!(connection.routed_action_count(), 1);
    }

    #[test]
    fn terminal_response_is_cached_until_bounded_delivery_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port.clone());
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();

        let first = connection
            .apply(ApplyArguments {
                action_ref: action_ref.clone(),
                outcome: None,
            })
            .unwrap();
        let cached = connection
            .apply(ApplyArguments {
                action_ref: action_ref.clone(),
                outcome: None,
            })
            .unwrap();
        assert_eq!(cached, first);
        assert_eq!(port.0.lock().unwrap().effect_calls, 1);
        assert_eq!(connection.routed_action_count(), 1);

        connection.complete_terminal_delivery(&action_ref);
        assert_eq!(connection.routed_action_count(), 0);
        assert_eq!(
            connection
                .apply(ApplyArguments {
                    action_ref,
                    outcome: None,
                })
                .unwrap_err(),
            "action_ref_invalid"
        );
    }

    #[test]
    fn oversized_terminal_response_keeps_cached_receipt_route_for_details_delivery() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        port.0.lock().unwrap().large_terminal = true;
        let mut connection = Connection::new(a.clone(), port.clone());
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();

        let response = tool_call_response(
            &mut connection,
            &json!({
                "params": {
                    "name": TOOL_APPLY,
                    "arguments": {"action_ref": action_ref}
                }
            }),
            request_id(json!(1)),
        );
        assert_eq!(response.envelope["result"]["isError"], true);
        assert_eq!(connection.routed_action_count(), 1);
        let cached = connection
            .apply(ApplyArguments {
                action_ref,
                outcome: None,
            })
            .unwrap();
        assert_eq!(cached["receipt"]["receipt_id"], "receipt-1");
        assert_eq!(port.0.lock().unwrap().effect_calls, 1);
    }

    #[test]
    fn malformed_arguments_are_json_rpc_errors_and_execution_failures_are_tool_errors() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let mut connection = Connection::new(home, Port::default());
        initialize_connection(&mut connection, "hermes", vec![]);

        let malformed = tool_call_response(
            &mut connection,
            &json!({"params": {"name": TOOL_DECIDE, "arguments": {"ghost": true}}}),
            request_id(json!(1)),
        );
        assert_eq!(malformed.envelope["error"]["code"], -32602);

        let execution = tool_call_response(
            &mut connection,
            &json!({
                "params": {
                    "name": TOOL_DECIDE,
                    "arguments": {"operation": operation("doctor")}
                }
            }),
            request_id(json!(2)),
        );
        assert!(execution.envelope.get("error").is_none());
        assert_eq!(execution.envelope["result"]["isError"], true);
        assert!(execution.envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("workspace_required:"));
    }

    #[test]
    fn oversized_result_with_an_unsealed_details_reference_fails_closed() {
        #[derive(Clone)]
        struct LargePort;

        impl ControlPlaneSeam for LargePort {
            type Session = ();

            fn open(
                &mut self,
                context: &RequestContext,
            ) -> Result<AuthenticatedWorkspaceSession<Self::Session>, String> {
                AuthenticatedWorkspaceSession::new(
                    &context.workspace,
                    "identity",
                    ags_platform::sha256("facts"),
                    "registry",
                    "session",
                    (),
                )
                .map_err(|error| error.to_string())
            }

            fn decide(
                &mut self,
                _context: &RequestContext,
                _session: &mut Self::Session,
                _operation: OperationRequest,
            ) -> Result<Value, String> {
                Ok(json!({
                    "state": "no-change",
                    "details_uri": "ags://details/receipt-1",
                    "payload": "x".repeat(32 * 1024)
                }))
            }

            fn apply(
                &mut self,
                _context: &RequestContext,
                _session: &mut Self::Session,
                _action_ref: String,
                _outcome: Option<HostOutcomeInput>,
            ) -> Result<Value, String> {
                unreachable!()
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let mut connection = Connection::new(a.clone(), LargePort);
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let response = tool_call_response(
            &mut connection,
            &json!({
                "params": {
                    "name": TOOL_DECIDE,
                    "arguments": {"operation": operation_at("doctor", Some(&a))}
                }
            }),
            request_id(json!(1)),
        );
        let encoded = serde_json::to_vec(&response.envelope).unwrap();
        assert!(
            encoded.len() <= MAX_MCP_RESULT_BYTES,
            "{} bytes",
            encoded.len()
        );
        assert_eq!(response.envelope["result"]["isError"], true);
        assert!(response.envelope["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("details_reference_missing_sha256"));
        assert!(!response.envelope.to_string().contains("resource_link"));
        assert!(!response.envelope.to_string().contains(&"x".repeat(1024)));
        let fake_seam = ["authorize", "_details_uri"].concat();
        assert!(!include_str!("contract_v2.rs").contains(&fake_seam));
    }

    #[test]
    fn nested_details_routes_are_readable_past_the_old_chunk_ceiling() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let payload = vec![b'x'; 400 * 1024];
        assert!(128 * (DETAILS_CHUNK_LIMIT as usize) < payload.len());
        assert!(MAX_DETAILS_CHUNKS * DETAILS_CHUNK_LIMIT as usize >= MAX_DETAILS_RESOURCE_BYTES);
        let uri = "ags://details/large-read-only";
        let digest = ags_platform::sha256(&payload);
        port.0.lock().unwrap().details_payload = Some(payload.clone());
        let mut connection = Connection::new(a.clone(), port);
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let binding = connection.action_routes[&action_ref].binding.clone();
        connection
            .route_details(
                &json!({
                    "result": {
                        "schema_version": "ags://schema/contract/v2/details-reference",
                        "status": "details_available",
                        "details_uri": uri,
                        "sha256": digest,
                        "byte_length": payload.len()
                    }
                }),
                binding.clone(),
            )
            .unwrap();
        assert!(connection.details_routes.contains_key(uri));
        let resource = connection.read_resource(uri).unwrap();
        assert_eq!(
            resource["contents"][0]["text"].as_str().unwrap().len(),
            payload.len()
        );

        let apply_uri = "ags://details/terminal-apply";
        connection
            .route_details(
                &json!({
                    "state": "receipted",
                    "details_uri": apply_uri,
                    "sha256": ags_platform::sha256("terminal"),
                    "byte_length": 8
                }),
                binding,
            )
            .unwrap();
        assert!(connection.details_routes.contains_key(apply_uri));
    }

    #[test]
    fn raw_file_details_uri_is_rejected_at_any_response_depth() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let port = Port::default();
        let mut connection = Connection::new(a.clone(), port);
        initialize_connection(&mut connection, "hermes", vec![a.clone()]);
        let action_ref = connection
            .decide(DecideArguments {
                operation: operation_at("setup", Some(&a)),
            })
            .unwrap()["action_ref"]
            .as_str()
            .unwrap()
            .to_string();
        let binding = connection.action_routes[&action_ref].binding.clone();
        assert_eq!(
            connection
                .route_details(
                    &json!({
                        "result": {
                            "details_uri": "file:///tmp/private-snapshot",
                            "sha256": ags_platform::sha256("snapshot")
                        }
                    }),
                    binding,
                )
                .unwrap_err(),
            "details_reference_invalid"
        );
    }

    #[test]
    fn cli_machine_json_and_mcp_use_the_same_typed_operation() {
        use ags_cli::{Cli, Invocation};
        let parsed = Cli::try_parse_from(["ags", "doctor", "all", "--workspace", "."])
            .unwrap()
            .into_invocation();
        let Invocation::Decide(human) = parsed.invocation else {
            panic!("doctor is a decide operation")
        };
        let machine: OperationRequest =
            serde_json::from_value(serde_json::to_value(&human).unwrap()).unwrap();
        let mcp: DecideArguments = serde_json::from_value(json!({"operation": machine})).unwrap();

        assert_eq!(human, mcp.operation);
        assert_eq!(
            ags_platform::sha256(serde_json::to_vec(&human).unwrap()),
            ags_platform::sha256(serde_json::to_vec(&mcp.operation).unwrap())
        );
    }
}
