//! Pure lifecycle codecs and planning helpers.
//!
//! Lifecycle mutation is intentionally absent from this module. Host events
//! become typed control-plane Operations; only sealed apply or a verified host
//! outcome may close an effectful event.

#[cfg(unix)]
use crate::control_plane::platform_io::unix::{read_regular_fd, StableReadError};
use crate::control_plane::{
    LifecycleSessionEndRequest, LifecycleSessionStartRequest, LifecycleStopGuardRequest,
    OperationContext, OperationRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(unix)]
use std::os::fd::OwnedFd;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

pub const LIFECYCLE_SCHEMA_VERSION: &str = "ags://schema/contract/v2/workspace-lifecycle";
pub const CLOSURE_POINTER_SCHEMA_VERSION: &str = "ags://schema/contract/v2/closure-pointer";
#[cfg(unix)]
const MAX_CAPSULE_CHARS: usize = 12_000;
#[cfg(unix)]
const MAX_TASK_MEMORY_CHARS: usize = 8_000;
#[cfg(unix)]
const CLOSURE_POINTERS_DIR: &str = ".ags/state/closure-pointers";
#[cfg(unix)]
const MAX_CLOSURE_POINTER_ENTRIES: usize = 256;
#[cfg(unix)]
const MAX_CLOSURE_POINTER_NAME_BYTES: usize = 4096;
#[cfg(unix)]
const MAX_CLOSURE_POINTER_BYTES: usize = 1024 * 1024;
#[cfg(unix)]
const MAX_CLOSURE_RECEIPT_BYTES: usize = 4 * 1024 * 1024;
#[cfg(unix)]
const MAX_CLOSURE_TOTAL_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClosurePointer {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_identity: Option<String>,
    pub receipt_id: String,
    pub receipt_path: String,
    pub receipt_sha256: String,
    pub task_card_hash: String,
    pub launch_plan_hash: String,
    pub delivery_report_hash: String,
    pub authority_key_id: String,
    pub authority_seal: String,
}

#[cfg_attr(not(unix), allow(dead_code))]
fn closure_pointer_seal_material(pointer: &ClosurePointer) -> Vec<u8> {
    let mut material = Vec::new();
    for field in [
        b"ags-control-plane/closure-pointer/v1".as_slice(),
        pointer.schema_version.as_bytes(),
        pointer.authority_key_id.as_bytes(),
        pointer
            .canonical_workspace
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        pointer
            .workspace_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        pointer.receipt_id.as_bytes(),
        pointer.receipt_path.as_bytes(),
        pointer.receipt_sha256.as_bytes(),
        pointer.task_card_hash.as_bytes(),
        pointer.launch_plan_hash.as_bytes(),
        pointer.delivery_report_hash.as_bytes(),
    ] {
        material.extend_from_slice(&(field.len() as u64).to_be_bytes());
        material.extend_from_slice(field);
    }
    material
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn closure_authority_key_id(machine_key: &[u8; 32]) -> String {
    blake3::keyed_hash(
        machine_key,
        b"ags-control-plane/closure-authority-key-id/v1",
    )
    .to_hex()
    .to_string()
}

#[cfg_attr(not(unix), allow(dead_code))]
fn workspace_closure_key(machine_key: &[u8; 32], pointer: &ClosurePointer) -> [u8; 32] {
    let mut material = Vec::new();
    for field in [
        b"ags-control-plane/closure-workspace-key/v1".as_slice(),
        pointer
            .canonical_workspace
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        pointer
            .workspace_identity
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    ] {
        material.extend_from_slice(&(field.len() as u64).to_be_bytes());
        material.extend_from_slice(field);
    }
    *blake3::keyed_hash(machine_key, &material).as_bytes()
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn seal_closure_pointer(
    machine_key: &[u8; 32],
    pointer: &mut ClosurePointer,
) -> Result<(), String> {
    pointer.authority_key_id = closure_authority_key_id(machine_key);
    pointer.authority_seal.clear();
    let key = workspace_closure_key(machine_key, pointer);
    pointer.authority_seal = blake3::keyed_hash(&key, &closure_pointer_seal_material(pointer))
        .to_hex()
        .to_string();
    Ok(())
}

#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn verify_closure_pointer_authority(
    machine_key: &[u8; 32],
    pointer: &ClosurePointer,
) -> Result<(), String> {
    if pointer.authority_key_id != closure_authority_key_id(machine_key) {
        return Err("closure authority key id mismatch".to_string());
    }
    let key = workspace_closure_key(machine_key, pointer);
    let expected = blake3::keyed_hash(&key, &closure_pointer_seal_material(pointer))
        .to_hex()
        .to_string();
    if pointer.authority_seal != expected {
        return Err("closure authority seal mismatch".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEnvelope {
    pub schema_version: String,
    pub canonical_workspace: String,
    pub workspace_identity: String,
    pub host: String,
    pub host_session_id: String,
    pub event: String,
    pub event_id: String,
    #[serde(default)]
    pub payload: Value,
}

impl LifecycleEnvelope {
    pub fn new(workspace: &Path, host: &str, event: &str, payload: Value) -> Result<Self, String> {
        let workspace = ags_platform::canonical_workspace_root(workspace)?;
        let host = ags_host_integration::HostId::new(host)?.to_string();
        let canonical_workspace = workspace.to_string_lossy().to_string();
        let explicit_event_id = payload
            .get("event_id")
            .or_else(|| payload.get("eventId"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string);
        let host_session_id = payload
            .get("session_id")
            .or_else(|| payload.get("sessionId"))
            .or_else(|| payload.get("conversation_id"))
            .or_else(|| payload.get("conversationId"))
            .or_else(|| payload.get("thread_id"))
            .or_else(|| payload.get("threadId"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .or_else(|| {
                explicit_event_id
                    .as_ref()
                    .map(|event_id| format!("event-{event_id}"))
            })
            .unwrap_or_else(|| {
                stable_json_id(
                    "synthetic-session",
                    &serde_json::json!({
                        "workspace": canonical_workspace,
                        "host": host,
                        "event": event,
                        "payload": payload,
                    }),
                )
            });
        let event_id = explicit_event_id.unwrap_or_else(|| {
            stable_json_id(
                "event",
                &serde_json::json!({
                    "workspace": canonical_workspace,
                    "host": host,
                    "host_session_id": host_session_id,
                    "event": event,
                    "payload": payload,
                }),
            )
        });
        Ok(Self {
            schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
            workspace_identity: workspace_identity(&workspace),
            canonical_workspace,
            host,
            host_session_id,
            event: event.to_string(),
            event_id,
            payload,
        })
    }

    pub fn into_operation(self) -> Result<OperationRequest, String> {
        if self.schema_version != LIFECYCLE_SCHEMA_VERSION {
            return Err("workspace lifecycle schema mismatch".to_string());
        }
        let context = OperationContext {
            workspace: Some(self.canonical_workspace),
        };
        match self.event.as_str() {
            "session-start" => Ok(OperationRequest::HostLifecycleSessionStart(
                LifecycleSessionStartRequest {
                    context,
                    host_id: self.host,
                    host_session_id: self.host_session_id,
                    event_id: self.event_id,
                },
            )),
            "session-end" => Ok(OperationRequest::HostLifecycleSessionEnd(
                LifecycleSessionEndRequest {
                    context,
                    host_id: self.host,
                    host_session_id: self.host_session_id,
                    event_id: self.event_id,
                },
            )),
            "stop-guard" => Ok(OperationRequest::HostLifecycleStopGuard(
                LifecycleStopGuardRequest {
                    context,
                    host_id: self.host,
                    host_session_id: self.host_session_id,
                    event_id: self.event_id,
                    last_assistant_message: text_content(
                        self.payload
                            .get("last_assistant_message")
                            .or_else(|| self.payload.get("lastAssistantMessage"))
                            .unwrap_or(&Value::Null),
                    ),
                },
            )),
            other => Err(format!("unsupported lifecycle event `{other}`")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifecycleDecision {
    pub schema_version: String,
    pub workspace_identity: String,
    pub host: String,
    pub host_session_id: String,
    pub event: String,
    pub event_id: String,
    pub status: String,
    pub duplicate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleSessionEndPlan {
    pub action_digest: String,
    pub memory_key: String,
    pub memory_uri: String,
    pub expected_write_paths: Vec<String>,
    pub receipt_ids: Vec<String>,
    pub pointer_paths: Vec<String>,
}

#[cfg(unix)]
pub fn session_start(
    workspace: &Path,
    home: &Path,
    request: &LifecycleSessionStartRequest,
) -> Result<LifecycleDecision, String> {
    let host = canonical_host(&request.host_id)?;
    let canonical_workspace = ags_platform::canonical_workspace_root(workspace)?;
    let memory_key = ags_host_integration::project_memory_key(&canonical_workspace)?;
    let memory_uri = format!("ags-memory://project/{memory_key}");
    let memory_dir = home.join(".agents/memory/projects").join(&memory_key);
    let home_descriptor = rustix::fs::open(
        home,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("cannot open lifecycle memory home: {error}"))?;
    let memory_descriptor = match open_directory_beneath(
        &home_descriptor,
        Path::new(".agents/memory/projects")
            .join(&memory_key)
            .as_path(),
    ) {
        Ok(descriptor) => Some(descriptor),
        Err(rustix::io::Errno::NOENT) => None,
        Err(error) => return Err(format!("cannot traverse lifecycle memory store: {error}")),
    };
    let capsule = memory_descriptor.as_ref().and_then(|descriptor| {
        bounded_read_at(descriptor, "context-capsule.md", MAX_CAPSULE_CHARS)
    });
    let task_memory = memory_descriptor.as_ref().and_then(|descriptor| {
        bounded_read_at(descriptor, "task-memory.md", MAX_TASK_MEMORY_CHARS)
    });
    let mut parts = vec![
        "## AGS Project Memory Context".to_string(),
        String::new(),
        "Read-only startup context. Receipt-bound raw artifacts remain authoritative.".to_string(),
        format!("Repository: {}", workspace.display()),
        format!("Memory store: {}", memory_dir.display()),
    ];
    if let Some(content) = capsule {
        parts.extend([String::new(), "### context-capsule.md".to_string(), content]);
    }
    if let Some(content) = task_memory {
        parts.extend([String::new(), "### task-memory.md".to_string(), content]);
    }
    let additional_context = (parts.len() > 5).then(|| parts.join("\n"));
    Ok(decision(DecisionContext {
        workspace,
        host: &host,
        host_session_id: &request.host_session_id,
        event: "session-start",
        event_id: &request.event_id,
        status: if additional_context.is_some() {
            "ready"
        } else {
            "empty"
        },
        additional_context,
        reason: None,
        memory_key: Some(memory_key),
        memory_uri: Some(memory_uri),
    }))
}

#[cfg_attr(not(unix), allow(dead_code))]
pub fn stop_guard(
    workspace: &Path,
    request: &LifecycleStopGuardRequest,
) -> Result<LifecycleDecision, String> {
    let host = canonical_host(&request.host_id)?;
    let normalized = request.last_assistant_message.to_ascii_lowercase();
    let blocked = normalized.contains("<invoke ")
        || normalized.contains("<parameter ")
        || normalized.contains("</invoke>");
    Ok(decision(DecisionContext {
        workspace,
        host: &host,
        host_session_id: &request.host_session_id,
        event: "stop-guard",
        event_id: &request.event_id,
        status: if blocked { "blocked" } else { "clear" },
        additional_context: blocked.then(|| "The previous assistant message leaked raw tool-call markup. Continue with a real tool call; do not expose tool markup.".to_string()),
        reason: None,
        memory_key: None,
        memory_uri: None,
    }))
}

#[cfg(unix)]
pub fn plan_session_end(
    workspace: &Path,
    home: &Path,
    request: &LifecycleSessionEndRequest,
    machine_key: &[u8; 32],
) -> Result<LifecycleSessionEndPlan, String> {
    let host = canonical_host(&request.host_id)?;
    let canonical_workspace = ags_platform::canonical_workspace_root(workspace)?;
    let identity = workspace_identity(&canonical_workspace);
    let memory_key = ags_host_integration::project_memory_key(&canonical_workspace)?;
    let memory_uri = format!("ags-memory://project/{memory_key}");
    let memory_dir = home.join(".agents/memory/projects").join(&memory_key);
    let pointer_dir = canonical_workspace.join(CLOSURE_POINTERS_DIR);
    let workspace_descriptor = rustix::fs::open(
        &canonical_workspace,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("cannot open canonical workspace: {error}"))?;
    let pointer_descriptor =
        match open_directory_beneath(&workspace_descriptor, Path::new(CLOSURE_POINTERS_DIR)) {
            Ok(descriptor) => Some(descriptor),
            Err(rustix::io::Errno::NOENT) => None,
            Err(error) => return Err(format!("cannot open closure pointer directory: {error}")),
        };
    let mut pointer_names = if let Some(descriptor) = pointer_descriptor.as_ref() {
        let mut names = Vec::new();
        let mut name_bytes = 0usize;
        for entry in rustix::fs::Dir::read_from(descriptor)
            .map_err(|error| format!("cannot read closure pointer directory: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("cannot read closure pointer directory: {error}"))?;
            let name = entry
                .file_name()
                .to_str()
                .map_err(|error| format!("invalid closure pointer name: {error}"))?
                .to_string();
            if name == "." || name == ".." {
                continue;
            }
            if names.len() >= MAX_CLOSURE_POINTER_ENTRIES {
                return Err("closure pointer directory exceeds entry budget".to_string());
            }
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > MAX_CLOSURE_POINTER_NAME_BYTES {
                return Err("closure pointer directory exceeds name byte budget".to_string());
            }
            names.push(name);
        }
        names.retain(|name| {
            Path::new(name).extension().and_then(|value| value.to_str()) == Some("json")
        });
        names
    } else {
        Vec::new()
    };
    pointer_names.sort();
    let mut expected = Vec::new();
    let mut receipt_ids = Vec::new();
    let mut consumed_pointer_paths = Vec::new();
    let mut identity_material = Vec::new();
    let mut total_bytes = 0usize;
    for directory in [
        home.join(".agents"),
        home.join(".agents/memory"),
        home.join(".agents/memory/projects"),
        memory_dir.clone(),
        memory_dir.join("task-archive"),
    ] {
        if !directory.is_dir() {
            expected.push(directory.display().to_string());
        }
    }
    for pointer_name in pointer_names {
        let pointer_path = pointer_dir.join(&pointer_name);
        let pointer_bytes = read_regular_at(
            pointer_descriptor
                .as_ref()
                .expect("pointer names require a retained directory descriptor"),
            Path::new(&pointer_name),
            MAX_CLOSURE_POINTER_BYTES,
            &mut total_bytes,
            "closure pointer",
        )?;
        let pointer: ClosurePointer = serde_json::from_slice(&pointer_bytes)
            .map_err(|error| format!("invalid closure pointer JSON: {error}"))?;
        verify_closure_pointer_authority(machine_key, &pointer)?;
        if pointer.schema_version != CLOSURE_POINTER_SCHEMA_VERSION
            || pointer.workspace_identity.as_deref() != Some(identity.as_str())
            || pointer.canonical_workspace.as_deref()
                != Some(canonical_workspace.to_string_lossy().as_ref())
        {
            return Err("closure pointer workspace binding mismatch".to_string());
        }
        let receipt_path = PathBuf::from(&pointer.receipt_path);
        let canonical_receipt_path = canonical_workspace
            .join(".ags/evidence")
            .join(format!("{}.json", pointer.receipt_id));
        if receipt_path != canonical_receipt_path {
            return Err("closure pointer receipt path is not canonical".to_string());
        }
        let receipt_relative = receipt_path
            .strip_prefix(&canonical_workspace)
            .map_err(|_| "closure receipt is outside the canonical workspace".to_string())?;
        let receipt_bytes = read_regular_at(
            &workspace_descriptor,
            receipt_relative,
            MAX_CLOSURE_RECEIPT_BYTES,
            &mut total_bytes,
            "closure receipt",
        )?;
        let receipt: ags_evidence::Receipt = serde_json::from_slice(&receipt_bytes)
            .map_err(|error| format!("invalid closure receipt: {error}"))?;
        if receipt.receipt_id != pointer.receipt_id
            || receipt.receipt_id
                != ags_evidence::receipt_id(&receipt.task_card_hash, &receipt.launch_plan_hash)
            || ags_platform::sha256(&receipt_bytes) != pointer.receipt_sha256
            || receipt.task_card_hash != pointer.task_card_hash
            || receipt.launch_plan_hash != pointer.launch_plan_hash
            || receipt.delivery_report_hash != pointer.delivery_report_hash
        {
            return Err("closure pointer metadata does not match receipt".to_string());
        }
        let archive_dir = memory_dir.join("task-archive").join(&receipt.receipt_id);
        if !archive_dir.is_dir() {
            expected.push(archive_dir.display().to_string());
        }
        expected.extend([
            archive_dir.join("task-card.md").display().to_string(),
            archive_dir.join("launch-plan.json").display().to_string(),
            archive_dir.join("delivery-report.md").display().to_string(),
            archive_dir.join("receipt.json").display().to_string(),
            memory_dir.join("task-memory.md").display().to_string(),
            pointer_path.display().to_string(),
        ]);
        receipt_ids.push(receipt.receipt_id);
        consumed_pointer_paths.push(pointer_path.display().to_string());
        identity_material.push(ags_platform::sha256(pointer_bytes));
        identity_material.push(ags_platform::sha256(receipt_bytes));
    }
    let lifecycle_directory = canonical_workspace.join(".ags/state/lifecycle");
    if !lifecycle_directory.is_dir() {
        expected.push(lifecycle_directory.display().to_string());
    }
    expected.push(
        lifecycle_directory
            .join(format!(
                "session-end-{}.json",
                ags_platform::sha256_hex(
                    format!("{}\0{}", host, request.host_session_id).as_bytes()
                )
            ))
            .display()
            .to_string(),
    );
    expected.sort();
    expected.dedup();
    Ok(LifecycleSessionEndPlan {
        action_digest: ags_platform::sha256(format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            identity,
            memory_key,
            memory_uri,
            host,
            request.host_session_id,
            request.event_id,
            identity_material.join("\n")
        )),
        memory_key,
        memory_uri,
        expected_write_paths: expected,
        receipt_ids,
        pointer_paths: consumed_pointer_paths,
    })
}

#[cfg(unix)]
fn open_directory_beneath(root: &OwnedFd, relative: &Path) -> Result<OwnedFd, rustix::io::Errno> {
    let flags = rustix::fs::OFlags::RDONLY
        | rustix::fs::OFlags::DIRECTORY
        | rustix::fs::OFlags::NOFOLLOW
        | rustix::fs::OFlags::CLOEXEC;
    let mut current = rustix::fs::openat(root, ".", flags, rustix::fs::Mode::empty())?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(rustix::io::Errno::INVAL);
        };
        current = rustix::fs::openat(&current, name, flags, rustix::fs::Mode::empty())?;
    }
    Ok(current)
}

#[cfg(unix)]
fn read_regular_at(
    root: &OwnedFd,
    relative: &Path,
    limit: usize,
    total_bytes: &mut usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    let remaining_total = MAX_CLOSURE_TOTAL_BYTES
        .checked_sub(*total_bytes)
        .ok_or_else(|| "closure inputs exceed total byte budget".to_string())?;
    if remaining_total == 0 {
        return Err("closure inputs exceed total byte budget".to_string());
    }
    let read_limit = limit.min(remaining_total);
    let mut components = relative.components().peekable();
    let mut directory = rustix::fs::openat(
        root,
        ".",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("cannot open {label} root: {error}"))?;
    let mut file_name = None;
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(format!("{label} path is not a normalized relative path"));
        };
        if components.peek().is_none() {
            file_name = Some(name.to_os_string());
            break;
        }
        directory = rustix::fs::openat(
            &directory,
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("cannot traverse {label}: {error}"))?;
    }
    let file_name = file_name.ok_or_else(|| format!("{label} path has no file name"))?;
    let descriptor = rustix::fs::openat(
        &directory,
        &file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("cannot read {label}: {error}"))?;
    let stable = read_regular_fd(&descriptor, read_limit as u64, || {
        #[cfg(all(test, unix))]
        tests::run_read_regular_after_read_rewrite_test_hook(relative);
    })
    .map_err(|error| match error {
        StableReadError::NotRegular => format!("{label} is not a regular file"),
        StableReadError::TooLarge if read_limit < limit => {
            "closure inputs exceed total byte budget".to_string()
        }
        StableReadError::TooLarge => format!("{label} exceeds byte budget"),
        StableReadError::Changed => format!("{label} changed during read"),
        StableReadError::Io(error) => format!("cannot read {label}: {error}"),
    })?;
    *total_bytes = total_bytes
        .checked_add(stable.bytes.len())
        .ok_or_else(|| "closure input byte budget overflow".to_string())?;
    if *total_bytes > MAX_CLOSURE_TOTAL_BYTES {
        return Err("closure inputs exceed total byte budget".to_string());
    }
    Ok(stable.bytes)
}

#[cfg_attr(not(unix), allow(dead_code))]
struct DecisionContext<'a> {
    workspace: &'a Path,
    host: &'a str,
    host_session_id: &'a str,
    event: &'a str,
    event_id: &'a str,
    status: &'a str,
    additional_context: Option<String>,
    reason: Option<String>,
    memory_key: Option<String>,
    memory_uri: Option<String>,
}

#[cfg_attr(not(unix), allow(dead_code))]
fn decision(context: DecisionContext<'_>) -> LifecycleDecision {
    LifecycleDecision {
        schema_version: LIFECYCLE_SCHEMA_VERSION.to_string(),
        workspace_identity: workspace_identity(context.workspace),
        host: context.host.to_string(),
        host_session_id: context.host_session_id.to_string(),
        event: context.event.to_string(),
        event_id: context.event_id.to_string(),
        status: context.status.to_string(),
        duplicate: false,
        memory_key: context.memory_key,
        memory_uri: context.memory_uri,
        additional_context: context.additional_context,
        reason: context.reason,
        archive: None,
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn canonical_host(host: &str) -> Result<String, String> {
    ags_host_integration::HostId::new(host).map(|host| host.to_string())
}

pub fn workspace_identity(workspace: &Path) -> String {
    ags_platform::sha256_hex(workspace.to_string_lossy().as_bytes())
}

fn stable_json_id(prefix: &str, value: &Value) -> String {
    format!(
        "{prefix}-{}",
        ags_platform::sha256_hex(
            serde_json::to_vec(value).expect("lifecycle event is serializable")
        )
    )
}

#[cfg(unix)]
fn bounded_read_at(root: &OwnedFd, name: &str, limit: usize) -> Option<String> {
    let descriptor = rustix::fs::openat(
        root,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let byte_limit = limit.saturating_mul(4).saturating_add(1);
    let bytes = read_regular_fd(&descriptor, byte_limit as u64, || {})
        .ok()?
        .bytes;
    let content = String::from_utf8_lossy(&bytes);
    let bounded = content.chars().take(limit).collect::<String>();
    Some(if content.chars().count() > limit {
        format!("{bounded}\n\n[truncated by AGS at {limit} characters]")
    } else {
        bounded
    })
}

fn text_content(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .map(str::to_string)
                    .or_else(|| part.get("text").and_then(Value::as_str).map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    thread_local! {
        static READ_REGULAR_AFTER_READ_REWRITE: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> =
            const { std::cell::RefCell::new(None) };
    }

    #[cfg(unix)]
    pub(super) fn run_read_regular_after_read_rewrite_test_hook(relative: &Path) {
        READ_REGULAR_AFTER_READ_REWRITE.with(|slot| {
            let rewrite = slot
                .borrow()
                .as_ref()
                .is_some_and(|(expected, _)| expected.ends_with(relative));
            if !rewrite {
                return;
            }
            let Some((path, replacement)) = slot.borrow_mut().take() else {
                return;
            };
            std::fs::write(path, replacement).unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn read_regular_at_rejects_same_fd_same_inode_same_size_rewrite() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().canonicalize().unwrap();
        let file = root_path.join("closure.json");
        std::fs::write(&file, b"original").unwrap();
        let before = std::fs::metadata(&file).unwrap();
        let root = rustix::fs::open(
            &root_path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        READ_REGULAR_AFTER_READ_REWRITE.with(|slot| {
            *slot.borrow_mut() = Some((file.clone(), b"tampered".to_vec()));
        });
        let mut total = 0;
        let error = read_regular_at(
            &root,
            Path::new("closure.json"),
            1024,
            &mut total,
            "closure artifact",
        )
        .unwrap_err();
        READ_REGULAR_AFTER_READ_REWRITE.with(|slot| {
            slot.borrow_mut().take();
        });
        assert!(error.contains("changed during read"), "{error}");
        let after = std::fs::metadata(file).unwrap();
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before.len(), after.len());
    }

    #[cfg(unix)]
    #[test]
    fn read_regular_at_checks_shared_total_budget_before_opening_next_member() {
        let directory = tempfile::tempdir().unwrap();
        let root_path = directory.path().canonicalize().unwrap();
        let fifo = root_path.join("must-not-open.fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let root = rustix::fs::open(
            &root_path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .unwrap();
        let mut total = MAX_CLOSURE_TOTAL_BYTES;
        let error = read_regular_at(
            &root,
            Path::new("must-not-open.fifo"),
            1024,
            &mut total,
            "closure artifact",
        )
        .unwrap_err();
        assert!(error.contains("total byte budget"), "{error}");
    }

    #[test]
    fn lifecycle_codec_normalizes_generic_hosts_without_platform_allowlist() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".git")).unwrap();
        let envelope = LifecycleEnvelope::new(
            root.path(),
            "  New_Host  ",
            "session-start",
            serde_json::json!({"session_id":"s", "event_id":"e"}),
        )
        .unwrap();
        let OperationRequest::HostLifecycleSessionStart(request) =
            envelope.into_operation().unwrap()
        else {
            panic!("session-start codec must return the typed operation")
        };
        assert_eq!(request.host_id, "new-host");
        let decision = session_start(root.path(), root.path(), &request).unwrap();
        assert_eq!(decision.host, "new-host");
    }

    #[test]
    fn session_start_is_workspace_keyed_and_descriptor_confined() {
        let home = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::create_dir(first.path().join(".git")).unwrap();
        fs::create_dir(second.path().join(".git")).unwrap();
        let first = first.path().canonicalize().unwrap();
        let second = second.path().canonicalize().unwrap();
        let first_key = ags_host_integration::project_memory_key(&first).unwrap();
        let second_key = ags_host_integration::project_memory_key(&second).unwrap();
        assert_ne!(first_key, second_key);
        let first_memory = home.path().join(".agents/memory/projects").join(&first_key);
        let second_memory = home
            .path()
            .join(".agents/memory/projects")
            .join(&second_key);
        fs::create_dir_all(&first_memory).unwrap();
        fs::create_dir_all(&second_memory).unwrap();
        fs::write(first_memory.join("context-capsule.md"), "first-only").unwrap();
        fs::write(second_memory.join("context-capsule.md"), "second-only").unwrap();
        let request = LifecycleSessionStartRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "session-a".to_string(),
            event_id: "event-a".to_string(),
        };

        let decision = session_start(&first, home.path(), &request).unwrap();
        let context = decision.additional_context.unwrap();
        assert!(context.contains("first-only"));
        assert!(!context.contains("second-only"));
        assert_eq!(decision.memory_key.as_deref(), Some(first_key.as_str()));
        assert_eq!(
            decision.memory_uri.as_deref(),
            Some(format!("ags-memory://project/{first_key}").as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn session_start_rejects_symlinked_memory_ancestor_and_ignores_unsafe_leafs() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir(workspace.path().join(".git")).unwrap();
        let workspace = workspace.path().canonicalize().unwrap();
        let request = LifecycleSessionStartRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "session-a".to_string(),
            event_id: "event-a".to_string(),
        };

        let linked_home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), linked_home.path().join(".agents")).unwrap();
        assert!(session_start(&workspace, linked_home.path(), &request).is_err());

        let safe_home = tempfile::tempdir().unwrap();
        let key = ags_host_integration::project_memory_key(&workspace).unwrap();
        let memory = safe_home.path().join(".agents/memory/projects").join(key);
        fs::create_dir_all(&memory).unwrap();
        fs::write(outside.path().join("secret"), "must-not-load").unwrap();
        symlink(
            outside.path().join("secret"),
            memory.join("context-capsule.md"),
        )
        .unwrap();
        assert!(std::process::Command::new("mkfifo")
            .arg(memory.join("task-memory.md"))
            .status()
            .unwrap()
            .success());
        let decision = session_start(&workspace, safe_home.path(), &request).unwrap();
        assert_eq!(decision.status, "empty");
        assert!(decision.additional_context.is_none());
    }

    #[test]
    fn session_end_digest_commits_to_canonical_memory_identity() {
        let home = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::create_dir(first.path().join(".git")).unwrap();
        fs::create_dir(second.path().join(".git")).unwrap();
        let request = LifecycleSessionEndRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "session-a".to_string(),
            event_id: "event-a".to_string(),
        };

        let first_plan =
            plan_session_end(first.path(), home.path(), &request, &[7_u8; 32]).unwrap();
        let second_plan =
            plan_session_end(second.path(), home.path(), &request, &[7_u8; 32]).unwrap();
        assert_ne!(first_plan.memory_key, second_plan.memory_key);
        assert_ne!(first_plan.memory_uri, second_plan.memory_uri);
        assert_ne!(first_plan.action_digest, second_plan.action_digest);
    }

    #[test]
    fn session_end_consumes_the_unique_receipt_bound_closure_pointer() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        let workspace = workspace.path().canonicalize().unwrap();
        let evidence_dir = workspace.join(".ags/evidence");
        let pointer_dir = workspace.join(CLOSURE_POINTERS_DIR);
        std::fs::create_dir_all(&evidence_dir).unwrap();
        std::fs::create_dir_all(&pointer_dir).unwrap();

        let task_path = workspace.join("task.md");
        let plan_path = workspace.join("launch-plan.json");
        let report_path = workspace.join("delivery-report.md");
        std::fs::write(&task_path, b"canonical task").unwrap();
        std::fs::write(&report_path, b"delivery report").unwrap();
        let mut plan = serde_json::json!({
            "schema_version": ags_task_contract::runner::SCHEMA_VERSION,
            "task_card_hash": ags_evidence::sha256_hex(b"canonical task")
        });
        let launch_plan_hash =
            ags_task_contract::runner::canonical_launch_plan_hash(&plan).unwrap();
        plan["launch_plan_hash"] = serde_json::Value::String(launch_plan_hash.clone());
        std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
        let task_card_hash = ags_evidence::sha256_hex(b"canonical task");
        let delivery_report_hash = ags_evidence::sha256_hex(b"delivery report");
        let receipt_id = ags_evidence::receipt_id(&task_card_hash, &launch_plan_hash);
        let receipt = ags_evidence::Receipt {
            schema_version: ags_evidence::RECEIPT_SCHEMA_VERSION.to_string(),
            receipt_id: receipt_id.clone(),
            timestamp: "unix-0".to_string(),
            task_card_hash: task_card_hash.clone(),
            launch_plan_hash: launch_plan_hash.clone(),
            task_card_path: Some(task_path.display().to_string()),
            launch_plan_path: plan_path.display().to_string(),
            delivery_report_path: report_path.display().to_string(),
            gate_result: ags_evidence::GateResult {
                decision: "allow".to_string(),
                reason: None,
            },
            verification_results: Vec::new(),
            delivery_report_hash: delivery_report_hash.clone(),
            execution_footprint: ags_evidence::ExecutionFootprint {
                execution_mode_used: "single-writer".to_string(),
                execution_topology_used: "single".to_string(),
                delegation_used: "none".to_string(),
            },
            closure_status: "completed".to_string(),
            exit_code: Some(0),
            governance_status: Some(ags_governance_decision::GovernanceStatus::DoneWithReceipt),
            governance_evidence: None,
        };
        let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
        let receipt_path = evidence_dir.join(format!("{receipt_id}.json"));
        std::fs::write(&receipt_path, &receipt_bytes).unwrap();
        let pointer_path = pointer_dir.join(format!("{receipt_id}.json"));
        let mut pointer = ClosurePointer {
            schema_version: CLOSURE_POINTER_SCHEMA_VERSION.to_string(),
            canonical_workspace: Some(workspace.display().to_string()),
            workspace_identity: Some(workspace_identity(&workspace)),
            receipt_id: receipt_id.clone(),
            receipt_path: receipt_path.display().to_string(),
            receipt_sha256: ags_platform::sha256(&receipt_bytes),
            task_card_hash,
            launch_plan_hash,
            delivery_report_hash,
            authority_key_id: String::new(),
            authority_seal: String::new(),
        };
        seal_closure_pointer(&[7_u8; 32], &mut pointer).unwrap();
        std::fs::write(&pointer_path, serde_json::to_vec_pretty(&pointer).unwrap()).unwrap();

        let request = LifecycleSessionEndRequest {
            context: OperationContext::default(),
            host_id: "hermes".to_string(),
            host_session_id: "session-a".to_string(),
            event_id: "event-a".to_string(),
        };
        let plan = plan_session_end(&workspace, &workspace, &request, &[7_u8; 32]).unwrap();
        assert_eq!(plan.receipt_ids, vec![receipt_id]);
        assert!(plan
            .expected_write_paths
            .contains(&pointer_path.display().to_string()));

        #[cfg(unix)]
        {
            let symlink_target = workspace.join("pointer-outside-authority.json");
            std::fs::rename(&pointer_path, &symlink_target).unwrap();
            std::os::unix::fs::symlink(&symlink_target, &pointer_path).unwrap();
            assert!(
                plan_session_end(&workspace, &workspace, &request, &[7_u8; 32])
                    .unwrap_err()
                    .contains("closure pointer")
            );
            fs::remove_file(&pointer_path).unwrap();
            std::fs::rename(&symlink_target, &pointer_path).unwrap();

            let receipt_target = workspace.join("receipt-outside-authority.json");
            std::fs::rename(&receipt_path, &receipt_target).unwrap();
            std::os::unix::fs::symlink(&receipt_target, &receipt_path).unwrap();
            assert!(
                plan_session_end(&workspace, &workspace, &request, &[7_u8; 32])
                    .unwrap_err()
                    .contains("closure receipt")
            );
            fs::remove_file(&receipt_path).unwrap();
            std::fs::rename(&receipt_target, &receipt_path).unwrap();

            let fifo_path = pointer_dir.join("fifo.json");
            assert!(std::process::Command::new("mkfifo")
                .arg(&fifo_path)
                .status()
                .unwrap()
                .success());
            assert!(
                plan_session_end(&workspace, &workspace, &request, &[7_u8; 32])
                    .unwrap_err()
                    .contains("regular file")
            );
            fs::remove_file(fifo_path).unwrap();
        }

        let oversized_pointer = pointer_dir.join("oversized.json");
        std::fs::write(
            &oversized_pointer,
            vec![b'x'; MAX_CLOSURE_POINTER_BYTES + 1],
        )
        .unwrap();
        assert!(
            plan_session_end(&workspace, &workspace, &request, &[7_u8; 32])
                .unwrap_err()
                .contains("byte budget")
        );
        fs::remove_file(oversized_pointer).unwrap();

        std::fs::write(&receipt_path, b"{}").unwrap();
        assert!(
            plan_session_end(&workspace, &workspace, &request, &[7_u8; 32])
                .unwrap_err()
                .contains("invalid closure receipt")
        );
    }

    #[test]
    fn session_end_pointer_enumeration_enforces_name_bytes_before_collecting() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join(".git")).unwrap();
        let workspace = workspace.path().canonicalize().unwrap();
        let pointer_dir = workspace.join(CLOSURE_POINTERS_DIR);
        std::fs::create_dir_all(&pointer_dir).unwrap();
        for index in 0..32 {
            let name = format!("{index:02}-{}", "x".repeat(200));
            std::fs::write(pointer_dir.join(name), b"ignored").unwrap();
        }
        let error = plan_session_end(
            &workspace,
            &workspace,
            &LifecycleSessionEndRequest {
                context: OperationContext::default(),
                host_id: "hermes".to_string(),
                host_session_id: "budget-session".to_string(),
                event_id: "budget-event".to_string(),
            },
            &[7_u8; 32],
        )
        .unwrap_err();
        assert!(error.contains("name byte budget"), "{error}");
    }
}
