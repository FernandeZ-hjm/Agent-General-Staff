//! Contract-v2 MCP adapter and private per-workspace daemon.
//!
//! The stdio process owns only JSON-RPC framing and request-scoped routing.
//! Every Operation crosses an authenticated daemon session and the daemon owns
//! the single control-plane state machine.

mod protocol;

pub mod contract_v2;

use ags_control_plane::{
    ApplyRequest, ApplyResult, AuthenticatedBinding, AuthenticatedHostOutcome, ControlPlaneError,
    Decision, HostOutcomeInput, OpenRequest, OpenedSession, OperationRequest,
    ProductionControlPlane,
};
use ags_session::{
    read_workspace_wire_frame, WorkspaceCapabilityActivationRequest,
    WorkspaceCapabilityActivationResult, WorkspaceCommandContext, WorkspaceControlRequest,
    WorkspaceControlResponse, WorkspaceControlSurface, WorkspaceServiceInspection,
    WorkspaceSessionContext, WorkspaceSessionHandler, WorkspaceState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

type ControlRequest = WorkspaceControlRequest<OperationRequest, HostOutcomeInput>;
pub type ControlResponse = WorkspaceControlResponse<OpenedSession, Decision, ApplyResult>;
const PRODUCT_CLI_EXECUTOR_ID: &str = "product-cli";
const MAX_HOST_OUTCOME_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOST_OUTCOME_COMPONENTS: usize = 64;

#[cfg(test)]
thread_local! {
    static HOST_OUTCOME_BETWEEN_READS_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_host_outcome_between_reads_hook() {
    HOST_OUTCOME_BETWEEN_READS_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn run_host_outcome_between_reads_hook() {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControlFailure {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum ControlReply {
    Ok(Box<ControlResponse>),
    Error(ControlFailure),
}

impl From<ControlPlaneError> for ControlFailure {
    fn from(error: ControlPlaneError) -> Self {
        Self {
            code: error.code.to_string(),
            detail: error.detail,
        }
    }
}

struct ControlPlaneSessionHandler {
    runtime_home: PathBuf,
    plane: Mutex<ProductionControlPlane>,
    #[cfg(unix)]
    workspace_root: HeldRoot,
    #[cfg(unix)]
    runtime_root: HeldRoot,
}

impl ControlPlaneSessionHandler {
    fn new(workspace: &Path) -> Result<Self, String> {
        let runtime_home = ags_platform::normalize_path(&ags_platform::runtime_home());
        ensure_private_runtime_root(&runtime_home)?;
        let plane =
            ProductionControlPlane::new(runtime_home.clone()).map_err(|error| error.to_string())?;
        Ok(Self {
            plane: Mutex::new(plane),
            #[cfg(unix)]
            workspace_root: HeldRoot::open(workspace)?,
            #[cfg(unix)]
            runtime_root: HeldRoot::open(&runtime_home)?,
            runtime_home,
        })
    }

    fn authorized_roots(&self, workspace: &Path) -> Vec<PathBuf> {
        vec![workspace.to_path_buf(), self.runtime_home.clone()]
    }

    fn mcp_binding(&self, context: &WorkspaceSessionContext) -> AuthenticatedBinding {
        AuthenticatedBinding::mcp(
            &context.connection_id,
            &context.host_id,
            &context.canonical_workspace,
            &context.workspace_identity,
            &context.project_facts_hash,
            &context.registry_key,
            &context.authenticated_session,
            self.authorized_roots(&context.canonical_workspace),
        )
    }

    fn cli_binding(
        &self,
        context: &WorkspaceCommandContext,
        workspace: &WorkspaceState,
    ) -> AuthenticatedBinding {
        AuthenticatedBinding::cli(
            PRODUCT_CLI_EXECUTOR_ID,
            &context.canonical_workspace,
            workspace.instance_key(),
            workspace.project_facts_hash(),
            workspace.instance_key(),
            &context.workspace_service_identity,
            self.authorized_roots(&context.canonical_workspace),
        )
    }

    fn open(&self, binding: AuthenticatedBinding) -> Result<OpenedSession, ControlFailure> {
        self.plane
            .lock()
            .map_err(|_| failure("control_plane_lock_poisoned", "control plane lock poisoned"))?
            .open(OpenRequest {
                binding,
                policy_hash: ags_platform::sha256("ags-contract-v2-default-policy"),
            })
            .map_err(Into::into)
    }

    fn decide(
        &self,
        session: &OpenedSession,
        operation: OperationRequest,
    ) -> Result<Decision, ControlFailure> {
        self.plane
            .lock()
            .map_err(|_| failure("control_plane_lock_poisoned", "control plane lock poisoned"))?
            .decide(session, operation)
            .map_err(Into::into)
    }

    fn apply(
        &self,
        binding: AuthenticatedBinding,
        action_ref: String,
        outcome: Option<HostOutcomeInput>,
    ) -> Result<ApplyResult, ControlFailure> {
        let outcome = outcome
            .map(|input| self.authenticate_host_outcome(&binding, input))
            .transpose()?;
        self.plane
            .lock()
            .map_err(|_| failure("control_plane_lock_poisoned", "control plane lock poisoned"))?
            .apply(
                &binding,
                ApplyRequest {
                    action_ref,
                    outcome,
                },
            )
            .map_err(Into::into)
    }

    fn authenticate_host_outcome(
        &self,
        binding: &AuthenticatedBinding,
        input: HostOutcomeInput,
    ) -> Result<AuthenticatedHostOutcome, ControlFailure> {
        let decoded = crate::contract_v2::decode_file_uri(&input.receipt.uri).ok_or_else(|| {
            failure(
                "host_outcome_uri_invalid",
                "host outcome receipt must use a local absolute file:// URI",
            )
        })?;
        let path = PathBuf::from(decoded);
        #[cfg(unix)]
        let bytes = {
            let (root, relative) = if binding.canonical_workspace()
                == self.workspace_root.canonical_path
                && path.starts_with(&self.workspace_root.canonical_path)
            {
                (
                    &self.workspace_root,
                    path.strip_prefix(&self.workspace_root.canonical_path)
                        .expect("workspace prefix was checked"),
                )
            } else if path.starts_with(&self.runtime_root.canonical_path) {
                (
                    &self.runtime_root,
                    path.strip_prefix(&self.runtime_root.canonical_path)
                        .expect("runtime prefix was checked"),
                )
            } else {
                return Err(failure(
                    "host_outcome_artifact_outside_binding",
                    "host outcome receipt is outside the authenticated workspace/runtime roots",
                ));
            };
            read_regular_outcome(root, relative)?
        };
        #[cfg(not(unix))]
        let bytes = {
            let _ = (binding, path);
            return Err(failure(
                "host_outcome_artifact_unreadable",
                "host outcome receipt reading requires an fd-relative no-follow backend",
            ));
        };
        if ags_platform::sha256(&bytes) != input.receipt.sha256 {
            return Err(failure(
                "host_outcome_artifact_digest_mismatch",
                "host outcome receipt bytes do not match the declared sha256",
            ));
        }
        Ok(AuthenticatedHostOutcome::from_artifact(
            binding.clone(),
            input.receipt,
            bytes,
        ))
    }

    fn handle_session_request(
        &self,
        request: ControlRequest,
        context: &WorkspaceSessionContext,
        workspace: &WorkspaceState,
        opened: &mut Option<OpenedSession>,
    ) -> Result<ControlResponse, ControlFailure> {
        let binding = self.mcp_binding(context);
        match request {
            WorkspaceControlRequest::Open { surface } => {
                if surface != WorkspaceControlSurface::Mcp {
                    return Err(failure(
                        "control_surface_mismatch",
                        "persistent workspace sessions require the mcp surface",
                    ));
                }
                let session = self.open(binding)?;
                *opened = Some(session.clone());
                Ok(WorkspaceControlResponse::Opened(session))
            }
            WorkspaceControlRequest::Decide { operation } => {
                ensure_live_project_facts(context, workspace)?;
                let session = opened.as_ref().ok_or_else(|| {
                    failure("control_session_not_open", "open must precede decide")
                })?;
                self.decide(session, operation)
                    .map(WorkspaceControlResponse::Decided)
            }
            WorkspaceControlRequest::Apply {
                action_ref,
                outcome,
            } => {
                ensure_live_project_facts(context, workspace)?;
                self.apply(binding, action_ref, outcome)
                    .map(WorkspaceControlResponse::Applied)
            }
        }
    }

    fn handle_cli_request(
        &self,
        request: ControlRequest,
        context: &WorkspaceCommandContext,
        workspace: &WorkspaceState,
    ) -> Result<ControlResponse, ControlFailure> {
        let binding = self.cli_binding(context, workspace);
        match request {
            WorkspaceControlRequest::Open { surface } => {
                if surface != WorkspaceControlSurface::Cli {
                    return Err(failure(
                        "control_surface_mismatch",
                        "workspace command accepts only the cli surface",
                    ));
                }
                self.open(binding).map(WorkspaceControlResponse::Opened)
            }
            WorkspaceControlRequest::Decide { operation } => {
                let session = self.open(binding)?;
                self.decide(&session, operation)
                    .map(WorkspaceControlResponse::Decided)
            }
            WorkspaceControlRequest::Apply {
                action_ref,
                outcome,
            } => self
                .apply(binding, action_ref, outcome)
                .map(WorkspaceControlResponse::Applied),
        }
    }
}

fn ensure_private_runtime_root(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|error| format!("cannot create daemon runtime root: {error}"))?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect daemon runtime root: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("daemon runtime root must be a real directory".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot secure daemon runtime root: {error}"))?;
    }
    Ok(())
}

impl WorkspaceSessionHandler for ControlPlaneSessionHandler {
    fn run(
        &self,
        mut reader: BufReader<TcpStream>,
        mut writer: TcpStream,
        workspace: Arc<WorkspaceState>,
        context: WorkspaceSessionContext,
        _startup_executable_hash: String,
    ) {
        let mut opened = None;
        let mut frame = Vec::with_capacity(8 * 1024);
        loop {
            let line = match read_workspace_wire_frame(&mut reader, &mut frame) {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    let code = if error.starts_with("workspace_wire_frame_too_large:") {
                        "control_frame_too_large"
                    } else {
                        "control_read_failed"
                    };
                    let _ = write_reply(&mut writer, &ControlReply::Error(failure(code, error)));
                    break;
                }
            };
            let reply = match serde_json::from_str::<ControlRequest>(&line) {
                Ok(request) => {
                    match self.handle_session_request(request, &context, &workspace, &mut opened) {
                        Ok(response) => ControlReply::Ok(Box::new(response)),
                        Err(error) => ControlReply::Error(error),
                    }
                }
                Err(error) => {
                    ControlReply::Error(failure("control_request_invalid", error.to_string()))
                }
            };
            if write_reply(&mut writer, &reply).is_err() {
                break;
            }
        }
    }

    fn run_workspace_command(
        &self,
        kind: &str,
        payload: Value,
        workspace: Arc<WorkspaceState>,
        context: WorkspaceCommandContext,
    ) -> Result<Value, String> {
        if kind == "status" {
            return serde_json::to_value(WorkspaceServiceInspection {
                schema_version: ags_session::WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION.to_string(),
                canonical_workspace: workspace.root().to_string_lossy().to_string(),
                workspace_identity: workspace.instance_key().to_string(),
                loaded_snapshot_hashes: workspace.loaded_snapshot_hashes()?,
            })
            .map_err(|error| format!("workspace daemon status encode failed: {error}"));
        }
        if kind == ags_session::WORKSPACE_COMMAND_ACTIVATE_CAPABILITIES {
            let request: WorkspaceCapabilityActivationRequest = serde_json::from_value(payload)
                .map_err(|error| format!("capability activation request invalid: {error}"))?;
            if request.schema_version != ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION
            {
                return Err("capability activation schema mismatch".to_string());
            }
            let active_hosts = request
                .active_hosts
                .iter()
                .map(|host| ags_host_integration::HostId::new(host).map(|id| id.to_string()))
                .collect::<Result<Vec<_>, String>>()?;
            let retired_hosts = request
                .retired_hosts
                .iter()
                .map(|host| ags_host_integration::HostId::new(host).map(|id| id.to_string()))
                .collect::<Result<Vec<_>, String>>()?;
            let activated_snapshot_hashes = workspace.activate_host_snapshots(
                &active_hosts,
                &retired_hosts,
                request.replace_all,
            )?;
            return serde_json::to_value(WorkspaceCapabilityActivationResult {
                schema_version: ags_session::WORKSPACE_CAPABILITY_ACTIVATION_SCHEMA_VERSION
                    .to_string(),
                activated_snapshot_hashes,
                loaded_snapshot_hashes: workspace.loaded_snapshot_hashes()?,
            })
            .map_err(|error| format!("capability activation encode failed: {error}"));
        }
        if kind == ags_session::WORKSPACE_COMMAND_CONTROL_PLANE {
            let request: ControlRequest = serde_json::from_value(payload)
                .map_err(|error| format!("control-plane request invalid: {error}"))?;
            let response = self
                .handle_cli_request(request, &context, &workspace)
                .map_err(|error| format!("{}: {}", error.code, error.detail))?;
            return serde_json::to_value(response)
                .map_err(|error| format!("control-plane response encode failed: {error}"));
        }
        Err(format!("unsupported workspace command `{kind}`"))
    }
}

fn ensure_live_project_facts(
    context: &WorkspaceSessionContext,
    workspace: &WorkspaceState,
) -> Result<(), ControlFailure> {
    let live = workspace.project_facts_hash();
    if live == context.project_facts_hash {
        Ok(())
    } else {
        Err(failure(
            "workspace_binding_stale",
            "governance project facts changed after the authenticated workspace handshake",
        ))
    }
}

#[cfg(unix)]
struct HeldRoot {
    canonical_path: PathBuf,
    fd: OwnedFd,
}

#[cfg(unix)]
impl HeldRoot {
    fn open(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::MetadataExt;

        let canonical_path = path
            .canonicalize()
            .map_err(|error| format!("cannot canonicalize held outcome root: {error}"))?;
        let expected = std::fs::metadata(&canonical_path)
            .map_err(|error| format!("cannot inspect held outcome root: {error}"))?;
        if !expected.is_dir() {
            return Err("held outcome root must be a directory".to_string());
        }
        let name = CString::new(canonical_path.as_os_str().as_bytes())
            .map_err(|_| "held outcome root contains a NUL byte".to_string())?;
        // SAFETY: name is NUL terminated. O_NOFOLLOW and O_DIRECTORY bind the
        // descriptor to the audited root object, not to a replaceable path.
        let fd = unsafe {
            libc::open(
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(format!(
                "cannot open held outcome root without following links: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: open returned a new owned descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: stat is writable and fd is live.
        if unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(format!(
                "cannot inspect held outcome root descriptor: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: fstat succeeded and initialized stat.
        let stat = unsafe { stat.assume_init() };
        if expected.dev() != stat.st_dev as u64 || expected.ino() != stat.st_ino {
            return Err("held outcome root changed while it was being opened".to_string());
        }
        Ok(Self { canonical_path, fd })
    }
}

#[cfg(unix)]
fn read_regular_outcome(root: &HeldRoot, relative: &Path) -> Result<Vec<u8>, ControlFailure> {
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components.len() > MAX_HOST_OUTCOME_COMPONENTS
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(failure(
            "host_outcome_artifact_unreadable",
            "host outcome path must be a bounded root-relative normal-component path",
        ));
    }

    let mut directory_fds = Vec::<OwnedFd>::with_capacity(components.len().saturating_sub(1));
    let mut parent_raw_fd = root.fd.as_raw_fd();
    for component in &components[..components.len() - 1] {
        let std::path::Component::Normal(name) = component else {
            unreachable!("outcome path components were validated")
        };
        let name = CString::new(name.as_bytes()).map_err(|_| {
            failure(
                "host_outcome_artifact_unreadable",
                "host outcome path contains a NUL byte",
            )
        })?;
        // SAFETY: parent_raw_fd is a live held directory and name is one
        // NUL-terminated component. Each returned directory fd anchors the
        // next lookup even if the namespace parent is concurrently replaced.
        let child = unsafe {
            libc::openat(
                parent_raw_fd,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
            )
        };
        if child < 0 {
            return Err(failure(
                "host_outcome_artifact_unreadable",
                format!(
                    "cannot open host outcome parent without following links: {}",
                    std::io::Error::last_os_error()
                ),
            ));
        }
        // SAFETY: openat returned a new owned descriptor.
        directory_fds.push(unsafe { OwnedFd::from_raw_fd(child) });
        parent_raw_fd = directory_fds
            .last()
            .expect("directory descriptor was installed")
            .as_raw_fd();
    }

    let std::path::Component::Normal(file_name) = components[components.len() - 1] else {
        unreachable!("outcome path components were validated")
    };
    let file_name = CString::new(file_name.as_bytes()).map_err(|_| {
        failure(
            "host_outcome_artifact_unreadable",
            "host outcome path contains a NUL byte",
        )
    })?;
    // SAFETY: parent_raw_fd is held and file_name is one NUL-terminated
    // component. O_NONBLOCK prevents a substituted FIFO from blocking.
    let outcome_fd = unsafe {
        libc::openat(
            parent_raw_fd,
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if outcome_fd < 0 {
        return Err(failure(
            "host_outcome_artifact_unreadable",
            format!(
                "cannot open host outcome receipt without following links: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    let mut file = File::from(unsafe { OwnedFd::from_raw_fd(outcome_fd) });
    let before = file.metadata().map_err(|error| {
        failure(
            "host_outcome_artifact_unreadable",
            format!("cannot inspect host outcome receipt: {error}"),
        )
    })?;
    if !before.file_type().is_file() {
        return Err(failure(
            "host_outcome_artifact_not_regular",
            "host outcome receipt must be a regular file",
        ));
    }
    if before.len() > MAX_HOST_OUTCOME_BYTES as u64 {
        return Err(failure(
            "host_outcome_artifact_too_large",
            "host outcome receipt exceeds 8 MiB",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_HOST_OUTCOME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            failure(
                "host_outcome_artifact_unreadable",
                format!("cannot read host outcome receipt: {error}"),
            )
        })?;
    if bytes.len() > MAX_HOST_OUTCOME_BYTES {
        return Err(failure(
            "host_outcome_artifact_too_large",
            "host outcome receipt exceeds 8 MiB",
        ));
    }
    run_host_outcome_between_reads_hook();
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        failure(
            "host_outcome_artifact_unreadable",
            format!("cannot rewind host outcome receipt: {error}"),
        )
    })?;
    let mut confirmation = Vec::with_capacity(bytes.len());
    (&mut file)
        .take((MAX_HOST_OUTCOME_BYTES + 1) as u64)
        .read_to_end(&mut confirmation)
        .map_err(|error| {
            failure(
                "host_outcome_artifact_unreadable",
                format!("cannot confirm host outcome receipt: {error}"),
            )
        })?;
    if confirmation != bytes {
        return Err(failure(
            "host_outcome_artifact_changed",
            "host outcome receipt changed between stable reads",
        ));
    }
    let after = file.metadata().map_err(|error| {
        failure(
            "host_outcome_artifact_unreadable",
            format!("cannot re-inspect host outcome receipt: {error}"),
        )
    })?;
    if before.len() != after.len() {
        return Err(failure(
            "host_outcome_artifact_changed",
            "host outcome receipt changed while it was being read",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(failure(
                "host_outcome_artifact_changed",
                "host outcome receipt changed while it was being read",
            ));
        }
    }
    Ok(bytes)
}

fn failure(code: impl Into<String>, detail: impl Into<String>) -> ControlFailure {
    ControlFailure {
        code: code.into(),
        detail: detail.into(),
    }
}

fn write_reply(writer: &mut TcpStream, reply: &ControlReply) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, reply)
        .map_err(|error| format!("control response encode failed: {error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("control response write failed: {error}"))
}

pub fn run_stdio_adapter() -> Result<(), String> {
    let adapter_cwd =
        std::env::current_dir().map_err(|error| format!("cannot resolve adapter cwd: {error}"))?;
    contract_v2::serve(
        BufReader::new(std::io::stdin().lock()),
        std::io::BufWriter::new(std::io::stdout().lock()),
        adapter_cwd,
        contract_v2::WorkspaceRpcPort,
    )
}

pub fn run_workspace_daemon(workspace: &Path) -> Result<(), String> {
    ags_session::run_workspace_daemon(
        workspace,
        Arc::new(ControlPlaneSessionHandler::new(workspace)?),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead as _;
    use std::net::TcpListener;
    use std::time::Duration;

    #[cfg(unix)]
    #[test]
    fn host_outcome_reader_accepts_only_bounded_regular_files() {
        use std::os::unix::fs::{symlink, FileTypeExt};

        let temp = tempfile::tempdir().unwrap();
        let receipt = temp.path().join("receipt.json");
        let link = temp.path().join("receipt-link.json");
        let linked_parent = temp.path().join("linked-parent");
        let fifo = temp.path().join("receipt.fifo");
        let oversized = temp.path().join("oversized.json");
        std::fs::write(&receipt, br#"{"schema_version":"v2"}"#).unwrap();
        symlink(&receipt, &link).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("receipt.json"), b"outside").unwrap();
        symlink(outside.path(), &linked_parent).unwrap();
        assert!(std::fs::metadata(&linked_parent)
            .unwrap()
            .file_type()
            .is_dir());
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert!(std::fs::metadata(&fifo).unwrap().file_type().is_fifo());
        std::fs::write(&oversized, vec![b'x'; MAX_HOST_OUTCOME_BYTES + 1]).unwrap();
        let root = HeldRoot::open(temp.path()).unwrap();

        assert_eq!(
            read_regular_outcome(&root, Path::new("receipt.json")).unwrap(),
            br#"{"schema_version":"v2"}"#
        );
        assert_eq!(
            read_regular_outcome(&root, Path::new("receipt-link.json"))
                .unwrap_err()
                .code,
            "host_outcome_artifact_unreadable"
        );
        assert_eq!(
            read_regular_outcome(&root, Path::new("linked-parent/receipt.json"))
                .unwrap_err()
                .code,
            "host_outcome_artifact_unreadable"
        );
        assert_eq!(
            read_regular_outcome(&root, Path::new("receipt.fifo"))
                .unwrap_err()
                .code,
            "host_outcome_artifact_not_regular"
        );
        assert_eq!(
            read_regular_outcome(&root, Path::new("oversized.json"))
                .unwrap_err()
                .code,
            "host_outcome_artifact_too_large"
        );
    }

    #[cfg(unix)]
    #[test]
    fn host_outcome_reader_rejects_same_length_in_place_rewrite_between_reads() {
        let temp = tempfile::tempdir().unwrap();
        let receipt = temp.path().join("receipt.json");
        std::fs::write(&receipt, b"safe").unwrap();
        let rewrite = receipt.clone();
        HOST_OUTCOME_BETWEEN_READS_HOOK.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                std::fs::write(&rewrite, b"evil").unwrap();
            }));
        });
        let root = HeldRoot::open(temp.path()).unwrap();

        assert_eq!(
            read_regular_outcome(&root, Path::new("receipt.json"))
                .unwrap_err()
                .code,
            "host_outcome_artifact_changed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn held_outcome_root_survives_namespace_parent_replacement() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root_path = temp.path().join("root");
        let moved_path = temp.path().join("moved-root");
        let attacker = temp.path().join("attacker");
        std::fs::create_dir(&root_path).unwrap();
        std::fs::create_dir(&attacker).unwrap();
        std::fs::write(root_path.join("receipt.json"), b"safe").unwrap();
        std::fs::write(attacker.join("receipt.json"), b"evil").unwrap();
        let root = HeldRoot::open(&root_path).unwrap();
        std::fs::rename(&root_path, &moved_path).unwrap();
        symlink(&attacker, &root_path).unwrap();

        assert_eq!(
            read_regular_outcome(&root, Path::new("receipt.json")).unwrap(),
            b"safe"
        );
    }

    #[cfg(unix)]
    #[test]
    fn authenticated_host_outcome_verifies_descriptor_bytes_digest() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let receipt_path = workspace.join("receipt.json");
        std::fs::write(&receipt_path, b"typed-receipt").unwrap();
        let handler = ControlPlaneSessionHandler::new(&workspace).unwrap();
        let binding = AuthenticatedBinding::cli(
            "host",
            &workspace,
            "workspace",
            "facts",
            "registry",
            "service",
            vec![workspace.clone()],
        );
        let error = handler
            .authenticate_host_outcome(
                &binding,
                HostOutcomeInput {
                    receipt: ags_control_plane::ContentAddressedArtifactRef {
                        uri: format!("file://{}", receipt_path.display()),
                        sha256: ags_platform::sha256(b"different"),
                    },
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "host_outcome_artifact_digest_mismatch");
    }

    #[test]
    fn oversized_authenticated_control_frame_gets_one_error_then_disconnects() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        let workspace = workspace.canonicalize().unwrap();
        let state =
            Arc::new(WorkspaceState::new(workspace.clone(), temp.path().join("runtime")).unwrap());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(endpoint).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_millis(250)))
                .unwrap();
            stream.write_all(&vec![b'x'; 1024 * 1024 + 1]).unwrap();
            let mut reader = BufReader::new(stream);
            let mut reply = String::new();
            reader.read_line(&mut reply).unwrap();
            let reply: ControlReply = serde_json::from_str(&reply).unwrap();
            let ControlReply::Error(error) = reply else {
                panic!("oversized frame must return a structured error")
            };
            assert_eq!(error.code, "control_frame_too_large");
            let mut trailing = Vec::new();
            reader.read_to_end(&mut trailing).unwrap();
            assert!(trailing.is_empty(), "connection must terminate after error");
        });
        let (server, _) = listener.accept().unwrap();
        let reader = BufReader::new(server.try_clone().unwrap());
        ControlPlaneSessionHandler::new(&workspace).unwrap().run(
            reader,
            server,
            state,
            WorkspaceSessionContext {
                canonical_workspace: workspace,
                workspace_service_identity: "daemon".to_string(),
                workspace_identity: "workspace".to_string(),
                project_facts_hash: ags_platform::sha256("facts"),
                registry_key: "registry".to_string(),
                authenticated_session: "session".to_string(),
                connection_id: "connection".to_string(),
                host_id: "host".to_string(),
            },
            "executable".to_string(),
        );
        client.join().unwrap();
    }
}
