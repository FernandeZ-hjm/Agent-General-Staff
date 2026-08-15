//! Structured project-test execution for the contract v2 LocalExecution path.
//!
//! This module accepts program and argument vectors separately, never invokes
//! a shell, and never rolls source bytes back after a failing test. A failure
//! is evidence; an unexpected write additionally escalates risk.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const TEST_RECEIPT_SCHEMA: &str = "ags://schema/contract/v2/test-receipt";
#[cfg(unix)]
const PROJECT_PROFILE_SCHEMA: &str = "ags://schema/contract/v2/project-profile";
const OUTPUT_CAPTURE_LIMIT: usize = 256 * 1024;
const OUTPUT_READER_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
const PROJECT_PROFILE_MAX_BYTES: usize = 1024 * 1024;
const SNAPSHOT_MAX_ENTRIES: usize = 200_000;
const SNAPSHOT_MAX_DEPTH: usize = 128;
const SNAPSHOT_MAX_HASHED_BYTES: u64 = 1024 * 1024 * 1024;
#[cfg(target_os = "macos")]
const MACOS_SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestProfile {
    Smoke,
    Standard,
    Full,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub allowed_write_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectTestProfiles {
    pub smoke: CommandSpec,
    pub standard: CommandSpec,
    pub full: CommandSpec,
}

impl ProjectTestProfiles {
    pub fn get(&self, profile: TestProfile) -> &CommandSpec {
        match profile {
            TestProfile::Smoke => &self.smoke,
            TestProfile::Standard => &self.standard,
            TestProfile::Full => &self.full,
        }
    }
}

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDocument {
    schema_version: String,
    #[serde(default)]
    project: Option<serde_yaml::Value>,
    #[serde(default)]
    defaults: Option<serde_yaml::Value>,
    verification: VerificationSection,
    #[serde(default)]
    risk: Option<serde_yaml::Value>,
    #[serde(default)]
    workflow: Option<serde_yaml::Value>,
    #[serde(default)]
    user_preferences: Option<serde_yaml::Value>,
}

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationSection {
    project_tests: ProjectTestProfiles,
    #[serde(default)]
    evidence_required: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestExecutionStatus {
    Succeeded,
    Failed,
    TimedOut,
    RiskEscalated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestExecutionErrorCode {
    WorkspaceInvalid,
    InvalidSpec,
    SandboxUnavailable,
    NotGitWorkspace,
    GitUnavailable,
    GitIdentityInvalid,
    SnapshotFailed,
    ProcessFailed,
    OutputCaptureFailed,
}

impl TestExecutionErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceInvalid => "workspace_invalid",
            Self::InvalidSpec => "invalid_spec",
            Self::SandboxUnavailable => "sandbox_unavailable",
            Self::NotGitWorkspace => "not_git_workspace",
            Self::GitUnavailable => "git_unavailable",
            Self::GitIdentityInvalid => "git_identity_invalid",
            Self::SnapshotFailed => "snapshot_failed",
            Self::ProcessFailed => "process_failed",
            Self::OutputCaptureFailed => "output_capture_failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LocalExecutionPlatformSupport {
    Supported {
        backend: String,
    },
    Blocked {
        error_code: TestExecutionErrorCode,
        reason: String,
    },
}

/// Report the audited LocalExecution containment backend for this host.
/// Unsupported hosts are explicitly blocked; there is no unsandboxed fallback.
pub fn local_execution_platform_support() -> LocalExecutionPlatformSupport {
    #[cfg(target_os = "macos")]
    {
        match macos_seatbelt_path() {
            Ok(_) => LocalExecutionPlatformSupport::Supported {
                backend: "macos-seatbelt".to_string(),
            },
            Err(reason) => LocalExecutionPlatformSupport::Blocked {
                error_code: TestExecutionErrorCode::SandboxUnavailable,
                reason,
            },
        }
    }
    #[cfg(target_os = "linux")]
    {
        match linux_bubblewrap_path() {
            Ok(_) => LocalExecutionPlatformSupport::Supported {
                backend: "linux-bubblewrap".to_string(),
            },
            Err(reason) => LocalExecutionPlatformSupport::Blocked {
                error_code: TestExecutionErrorCode::SandboxUnavailable,
                reason,
            },
        }
    }
    #[cfg(target_os = "windows")]
    {
        LocalExecutionPlatformSupport::Blocked {
            error_code: TestExecutionErrorCode::SandboxUnavailable,
            reason: "Windows LocalExecution is blocked: std::process cannot atomically bind a suspended child to a Job Object before its first instruction, and no audited AppContainer or filesystem-filter write-containment backend is in scope; use a policy-approved HostDelegated test outcome"
                .to_string(),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        LocalExecutionPlatformSupport::Blocked {
            error_code: TestExecutionErrorCode::SandboxUnavailable,
            reason: format!(
                "no audited fail-closed LocalExecution containment backend is registered for {}",
                std::env::consts::OS
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TestExecutionError {
    pub code: TestExecutionErrorCode,
    pub message: String,
}

impl TestExecutionError {
    fn new(code: TestExecutionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn contains(&self, value: &str) -> bool {
        self.message.contains(value)
    }
}

impl std::fmt::Display for TestExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}",
            serde_error_code(self.code),
            self.message
        )
    }
}

impl std::error::Error for TestExecutionError {}

fn serde_error_code(code: TestExecutionErrorCode) -> &'static str {
    match code {
        TestExecutionErrorCode::WorkspaceInvalid => "workspace_invalid",
        TestExecutionErrorCode::InvalidSpec => "invalid_spec",
        TestExecutionErrorCode::SandboxUnavailable => "sandbox_unavailable",
        TestExecutionErrorCode::NotGitWorkspace => "not_git_workspace",
        TestExecutionErrorCode::GitUnavailable => "git_unavailable",
        TestExecutionErrorCode::GitIdentityInvalid => "git_identity_invalid",
        TestExecutionErrorCode::SnapshotFailed => "snapshot_failed",
        TestExecutionErrorCode::ProcessFailed => "process_failed",
        TestExecutionErrorCode::OutputCaptureFailed => "output_capture_failed",
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TestReceipt {
    pub schema_version: String,
    pub profile: TestProfile,
    pub canonical_workspace: String,
    /// Commit object checked out when execution opened, or `unborn`.
    pub commit_hash: String,
    /// Tree object referenced by `commit_hash`, or `unborn`.
    pub tree_hash: String,
    /// Content-sensitive snapshot of dirty and untracked workspace bytes before
    /// execution. This complements, rather than replaces, the Git tree binding.
    pub workspace_tree_hash: String,
    pub argv_hash: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub output_digest: String,
    pub output_bytes: u64,
    pub output_truncated: bool,
    pub sandbox_backend: String,
    /// True only for a timed-out execution whose complete discovered
    /// descendant tree was stopped, killed, and confirmed non-live before the
    /// receipt was created.
    pub timeout_descendants_terminated: bool,
    pub observed_write_set: Vec<String>,
    pub unexpected_write_set: Vec<String>,
    pub status: TestExecutionStatus,
    pub closed: bool,
    pub source_rollback_performed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadOnlyCommandReceipt {
    pub schema_version: String,
    pub canonical_workspace: String,
    pub argv_hash: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub output_digest: String,
    pub output_bytes: u64,
    pub output_truncated: bool,
    pub sandbox_backend: String,
    pub observed_write_set: Vec<String>,
    /// True only when the complete workspace audit snapshot is unchanged.
    /// Containment is enforced by the sandbox; this snapshot is evidence, not
    /// the enforcement mechanism.
    pub zero_write_preserved: bool,
    pub status: TestExecutionStatus,
    pub closed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadOnlyCommandOutput {
    pub receipt: ReadOnlyCommandReceipt,
    /// Bounded UTF-8-lossy prefix. `receipt.output_digest` covers the complete
    /// byte stream even when this prefix is truncated.
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
struct TreeEntry {
    path: String,
    kind: &'static str,
    len: u64,
    modified_ns: u128,
    link_target: Option<String>,
    content_digest: Option<String>,
}

/// Load the structured project-test section relative to a validated workspace.
#[cfg(unix)]
pub fn load_project_test_profiles(
    canonical_workspace: &Path,
    relative_path: &Path,
) -> Result<ProjectTestProfiles, String> {
    if !canonical_workspace.is_absolute() {
        return Err("project profile workspace root must be absolute".to_string());
    }
    let components = relative_path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "project profile path must be non-empty and workspace-relative: {}",
            relative_path.display()
        ));
    }

    let workspace_name = CString::new(canonical_workspace.as_os_str().as_bytes())
        .map_err(|_| "project profile workspace path contains a NUL byte".to_string())?;
    // SAFETY: workspace_name is a valid NUL-terminated path. O_NOFOLLOW rejects
    // a substituted final workspace symlink and O_DIRECTORY requires a root.
    let workspace_fd = unsafe {
        libc::open(
            workspace_name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
        )
    };
    if workspace_fd < 0 {
        return Err(format!(
            "cannot open validated project profile workspace root without following links: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: open returned a new owned descriptor.
    let mut directory = unsafe { OwnedFd::from_raw_fd(workspace_fd) };
    for component in &components[..components.len() - 1] {
        let Component::Normal(name) = component else {
            unreachable!("components were validated above")
        };
        let name = CString::new(name.as_bytes())
            .map_err(|_| "project profile path contains a NUL byte".to_string())?;
        // SAFETY: directory is live and name is one NUL-terminated component.
        let child = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
            )
        };
        if child < 0 {
            return Err(format!(
                "cannot open project profile parent without following links {}: {}",
                relative_path.display(),
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: openat returned a new owned descriptor.
        directory = unsafe { OwnedFd::from_raw_fd(child) };
    }

    let Component::Normal(file_name) = components[components.len() - 1] else {
        unreachable!("components were validated above")
    };
    let file_name = CString::new(file_name.as_bytes())
        .map_err(|_| "project profile path contains a NUL byte".to_string())?;
    // SAFETY: directory is live and file_name is one NUL-terminated component.
    // O_NONBLOCK prevents a substituted FIFO from blocking before fstat rejects it.
    let profile_fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if profile_fd < 0 {
        return Err(format!(
            "cannot open project profile without following links {}: {}",
            relative_path.display(),
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: openat returned a new owned descriptor.
    let profile_fd = unsafe { OwnedFd::from_raw_fd(profile_fd) };
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: stat points to writable storage and profile_fd is live.
    if unsafe { libc::fstat(profile_fd.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "cannot inspect project profile descriptor {}: {}",
            relative_path.display(),
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fstat succeeded and initialized stat.
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(format!(
            "project profile must be a regular file: {}",
            relative_path.display()
        ));
    }
    let mut bytes = Vec::with_capacity((stat.st_size as usize).min(PROJECT_PROFILE_MAX_BYTES));
    std::fs::File::from(profile_fd)
        .take((PROJECT_PROFILE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "cannot read bounded project profile {}: {error}",
                relative_path.display()
            )
        })?;
    if bytes.len() > PROJECT_PROFILE_MAX_BYTES {
        return Err(format!(
            "project profile exceeds {PROJECT_PROFILE_MAX_BYTES} byte limit: {}",
            relative_path.display()
        ));
    }
    let document: ProfileDocument = serde_yaml::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid structured project profile {}: {error}",
            relative_path.display()
        )
    })?;
    if document.schema_version != PROJECT_PROFILE_SCHEMA {
        return Err(format!(
            "project profile schema_version must be {PROJECT_PROFILE_SCHEMA}, found {}",
            document.schema_version
        ));
    }
    let _ = (
        document.project,
        document.defaults,
        document.risk,
        document.workflow,
        document.user_preferences,
    );
    let _ = document.verification.evidence_required;
    Ok(document.verification.project_tests)
}

#[cfg(not(unix))]
pub fn load_project_test_profiles(
    _canonical_workspace: &Path,
    _relative_path: &Path,
) -> Result<ProjectTestProfiles, String> {
    Err("project profile loading is blocked: no audited fd-relative no-follow backend".to_string())
}

/// Execute one profile with direct argv and return a closed receipt even when
/// the command exits non-zero or times out.
pub fn run_project_test(
    workspace: &Path,
    profile: TestProfile,
    spec: &CommandSpec,
) -> Result<TestReceipt, TestExecutionError> {
    run_project_test_with_authority(workspace, profile, spec, ExecutionAuthority::Local)
}

/// Execute one already-authorized HostDelegated test instruction. The host
/// owns process execution; AGS still enforces typed argv/cwd/env/timeout,
/// captures bounded output, snapshots the workspace, and closes a receipt.
/// Unexpected writes are evidence and escalate risk instead of triggering a
/// source rollback.
pub fn run_host_project_test(
    workspace: &Path,
    profile: TestProfile,
    spec: &CommandSpec,
) -> Result<TestReceipt, TestExecutionError> {
    run_project_test_with_authority(workspace, profile, spec, ExecutionAuthority::HostDelegated)
}

fn run_project_test_with_authority(
    workspace: &Path,
    profile: TestProfile,
    spec: &CommandSpec,
    authority: ExecutionAuthority,
) -> Result<TestReceipt, TestExecutionError> {
    let execution = execute_structured(workspace, spec, true, authority)?;
    let git = execution
        .git
        .expect("project-test execution always captures Git identity");
    let output_digest = output_digest(&execution.captures);
    let output_bytes = execution
        .captures
        .values()
        .map(|capture| capture.total_bytes)
        .sum();
    let output_truncated = execution.captures.values().any(|capture| capture.truncated);

    Ok(TestReceipt {
        schema_version: TEST_RECEIPT_SCHEMA.to_string(),
        profile,
        canonical_workspace: execution.workspace.to_string_lossy().into_owned(),
        commit_hash: git.commit_hash,
        tree_hash: git.tree_hash,
        workspace_tree_hash: execution.workspace_tree_hash,
        argv_hash: execution.argv_hash,
        exit_code: execution.exit_code,
        duration_ms: execution.duration_ms,
        output_digest,
        output_bytes,
        output_truncated,
        sandbox_backend: execution.sandbox_backend,
        timeout_descendants_terminated: execution.timed_out,
        observed_write_set: execution.observed_write_set,
        unexpected_write_set: execution.unexpected_write_set,
        status: execution.status,
        closed: true,
        source_rollback_performed: false,
    })
}

/// Execute an external ReadOnly command through the same fail-closed child
/// runner as project tests. The command receives only isolated scratch as a
/// writable location: workspace, host-state, and every other filesystem root
/// remain non-writable. Tree snapshots are retained only as audit evidence.
pub fn run_read_only_command(
    workspace: &Path,
    spec: &CommandSpec,
) -> Result<ReadOnlyCommandOutput, TestExecutionError> {
    if !spec.allowed_write_paths.is_empty() {
        return Err(TestExecutionError::new(
            TestExecutionErrorCode::InvalidSpec,
            "ReadOnly external execution permits no writable roots",
        ));
    }
    let execution = execute_sandboxed(workspace, spec, false)?;
    let output_digest = output_digest(&execution.captures);
    let output_bytes = execution
        .captures
        .values()
        .map(|capture| capture.total_bytes)
        .sum();
    let output_truncated = execution.captures.values().any(|capture| capture.truncated);
    let stdout = execution
        .captures
        .get("stdout")
        .map(|capture| String::from_utf8_lossy(&capture.prefix).into_owned())
        .unwrap_or_default();
    let stderr = execution
        .captures
        .get("stderr")
        .map(|capture| String::from_utf8_lossy(&capture.prefix).into_owned())
        .unwrap_or_default();
    let zero_write_preserved = execution.observed_write_set.is_empty();
    Ok(ReadOnlyCommandOutput {
        receipt: ReadOnlyCommandReceipt {
            schema_version: "ags://schema/contract/v2/read-only-command-receipt".to_string(),
            canonical_workspace: execution.workspace.to_string_lossy().into_owned(),
            argv_hash: execution.argv_hash,
            exit_code: execution.exit_code,
            duration_ms: execution.duration_ms,
            output_digest,
            output_bytes,
            output_truncated,
            sandbox_backend: execution.sandbox_backend,
            observed_write_set: execution.observed_write_set,
            zero_write_preserved,
            status: execution.status,
            closed: true,
        },
        stdout,
        stderr,
    })
}

struct SandboxedExecution {
    workspace: PathBuf,
    git: Option<GitIdentity>,
    workspace_tree_hash: String,
    argv_hash: String,
    exit_code: i32,
    duration_ms: u64,
    captures: BTreeMap<&'static str, Capture>,
    sandbox_backend: String,
    timed_out: bool,
    observed_write_set: Vec<String>,
    unexpected_write_set: Vec<String>,
    status: TestExecutionStatus,
}

fn execute_sandboxed(
    workspace: &Path,
    spec: &CommandSpec,
    capture_git_identity: bool,
) -> Result<SandboxedExecution, TestExecutionError> {
    execute_structured(
        workspace,
        spec,
        capture_git_identity,
        ExecutionAuthority::Local,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExecutionAuthority {
    Local,
    HostDelegated,
}

fn execute_structured(
    workspace: &Path,
    spec: &CommandSpec,
    capture_git_identity: bool,
    authority: ExecutionAuthority,
) -> Result<SandboxedExecution, TestExecutionError> {
    let workspace = workspace.canonicalize().map_err(|error| {
        TestExecutionError::new(
            TestExecutionErrorCode::WorkspaceInvalid,
            format!(
                "cannot canonicalize workspace {}: {error}",
                workspace.display()
            ),
        )
    })?;
    validate_spec(&workspace, spec)
        .map_err(|message| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, message))?;
    let cwd = resolve_inside(&workspace, &spec.cwd, "cwd")
        .map_err(|message| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, message))?;
    let allowed = spec
        .allowed_write_paths
        .iter()
        .map(|path| resolve_allowed_inside(&workspace, path))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|message| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, message))?;

    let git = capture_git_identity
        .then(|| git_identity(&workspace))
        .transpose()?;
    if capture_git_identity && authority == ExecutionAuthority::Local {
        if let LocalExecutionPlatformSupport::Blocked { error_code, reason } =
            local_execution_platform_support()
        {
            return Err(TestExecutionError::new(error_code, reason));
        }
    }
    let mut sandbox = match authority {
        ExecutionAuthority::Local => {
            sandboxed_command(&workspace, spec, &cwd, &allowed, capture_git_identity)?
        }
        ExecutionAuthority::HostDelegated => host_delegated_command(spec, &cwd)?,
    };
    let sandbox_backend = sandbox.backend.clone();
    let command = &mut sandbox.command;
    let execution_membership = configure_execution_membership(command)?;

    let before = snapshot_tree(&workspace, &allowed).map_err(|message| {
        TestExecutionError::new(TestExecutionErrorCode::SnapshotFailed, message)
    })?;
    let workspace_tree_hash = digest_tree(&before);
    let argv_hash = argv_hash(spec)
        .map_err(|message| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, message))?;

    let started = Instant::now();
    configure_process_group(command)?;
    let mut child = command.spawn().map_err(|error| {
        TestExecutionError::new(
            TestExecutionErrorCode::ProcessFailed,
            format!(
                "cannot execute sandboxed program {:?}: {error}",
                spec.program
            ),
        )
    })?;
    let process_group = child.id();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (capture_tx, capture_rx) = mpsc::channel();
    let stdout_tx = capture_tx.clone();
    thread::spawn(move || {
        let _ = stdout_tx.send(("stdout", read_bounded(stdout)));
    });
    thread::spawn(move || {
        let _ = capture_tx.send(("stderr", read_bounded(stderr)));
    });

    let timeout = Duration::from_millis(spec.timeout_ms);
    let (exit_code, timed_out) = loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                format!("cannot poll test process: {error}"),
            )
        })? {
            break (status.code().unwrap_or(-1), false);
        }
        if started.elapsed() >= timeout {
            terminate_process_tree(process_group, execution_membership.as_deref())?;
            let status = child.wait().map_err(|error| {
                TestExecutionError::new(
                    TestExecutionErrorCode::ProcessFailed,
                    format!("cannot reap timed-out test process: {error}"),
                )
            })?;
            break (status.code().unwrap_or(-1), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !timed_out {
        terminate_remaining_processes(process_group, execution_membership.as_deref())?;
    }
    let captures = collect_captures(
        capture_rx,
        timeout.saturating_sub(started.elapsed()),
        process_group,
    )?;
    let duration_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    let after = snapshot_tree(&workspace, &allowed).map_err(|message| {
        TestExecutionError::new(TestExecutionErrorCode::SnapshotFailed, message)
    })?;
    let mut observed_write_set = changed_paths(&before, &after);
    observed_write_set.extend(allowed.iter().filter_map(|path| {
        let relative = path.strip_prefix(&workspace).ok()?;
        let key = relative.to_string_lossy();
        (before.get(key.as_ref()) != after.get(key.as_ref())).then(|| key.into_owned())
    }));
    observed_write_set.sort();
    observed_write_set.dedup();
    let unexpected_write_set = observed_write_set
        .iter()
        .filter(|relative| {
            let absolute = workspace.join(relative);
            !allowed.iter().any(|root| absolute.starts_with(root))
        })
        .cloned()
        .collect::<Vec<_>>();
    let status = if !unexpected_write_set.is_empty() {
        TestExecutionStatus::RiskEscalated
    } else if timed_out {
        TestExecutionStatus::TimedOut
    } else if exit_code == 0 {
        TestExecutionStatus::Succeeded
    } else {
        TestExecutionStatus::Failed
    };
    Ok(SandboxedExecution {
        workspace,
        git,
        workspace_tree_hash,
        argv_hash,
        exit_code,
        duration_ms,
        captures,
        sandbox_backend,
        timed_out,
        observed_write_set,
        unexpected_write_set,
        status,
    })
}

#[derive(Debug)]
struct Capture {
    digest: String,
    total_bytes: u64,
    truncated: bool,
    prefix: Vec<u8>,
}

fn read_bounded(mut reader: impl Read) -> Result<Capture, String> {
    let mut hasher = Sha256::new();
    let mut prefix = Vec::with_capacity(OUTPUT_CAPTURE_LIMIT);
    let mut total_bytes = 0_u64;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("cannot read test process output: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
        total_bytes = total_bytes.saturating_add(read as u64);
        let remaining = OUTPUT_CAPTURE_LIMIT.saturating_sub(prefix.len());
        prefix.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(Capture {
        digest: format!("sha256:{:x}", hasher.finalize()),
        total_bytes,
        truncated: total_bytes > prefix.len() as u64,
        prefix,
    })
}

fn collect_captures(
    receiver: mpsc::Receiver<(&'static str, Result<Capture, String>)>,
    remaining: Duration,
    process_group: u32,
) -> Result<BTreeMap<&'static str, Capture>, TestExecutionError> {
    let mut captures = BTreeMap::new();
    let deadline = Instant::now() + remaining;
    while captures.len() < 2 {
        let wait = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(wait) {
            Ok((stream, Ok(capture))) => {
                captures.insert(stream, capture);
            }
            Ok((_, Err(message))) => {
                return Err(TestExecutionError::new(
                    TestExecutionErrorCode::OutputCaptureFailed,
                    message,
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_process_group(process_group)?;
                match receiver.recv_timeout(OUTPUT_READER_GRACE) {
                    Ok((stream, Ok(capture))) => {
                        captures.insert(stream, capture);
                    }
                    Ok((_, Err(message))) => {
                        return Err(TestExecutionError::new(
                            TestExecutionErrorCode::OutputCaptureFailed,
                            message,
                        ));
                    }
                    Err(_) => {
                        return Err(TestExecutionError::new(
                            TestExecutionErrorCode::OutputCaptureFailed,
                            "output pipe remained open after process-group termination",
                        ));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(TestExecutionError::new(
                    TestExecutionErrorCode::OutputCaptureFailed,
                    "output capture channel disconnected",
                ));
            }
        }
    }
    Ok(captures)
}

fn output_digest(captures: &BTreeMap<&'static str, Capture>) -> String {
    let evidence = captures
        .iter()
        .map(|(stream, capture)| {
            serde_json::json!({
                "stream": stream,
                "digest": capture.digest,
                "total_bytes": capture.total_bytes,
                "captured_prefix_sha256": ags_platform::sha256(&capture.prefix),
            })
        })
        .collect::<Vec<_>>();
    ags_platform::sha256(serde_json::to_vec(&evidence).expect("capture evidence serializes"))
}

fn validate_spec(workspace: &Path, spec: &CommandSpec) -> Result<(), String> {
    if spec.program.trim().is_empty() {
        return Err("test program must not be empty".to_string());
    }
    let program_name = Path::new(&spec.program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if [
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
        "pwsh",
        "powershell",
        "cmd",
        "cmd.exe",
    ]
    .contains(&program_name.as_str())
    {
        return Err(format!(
            "test program must be a direct executable, not a shell: {}",
            spec.program
        ));
    }
    resolve_direct_program(&spec.program)?;
    if spec.timeout_ms == 0 {
        return Err("test timeout_ms must be greater than zero".to_string());
    }
    resolve_inside(workspace, &spec.cwd, "cwd")?;
    for path in &spec.allowed_write_paths {
        resolve_allowed_inside(workspace, path)?;
    }
    Ok(())
}

fn resolve_direct_program(program: &str) -> Result<PathBuf, String> {
    let program_path = Path::new(program);
    let resolved = if program_path.components().count() == 1 {
        ags_platform::find_in_path(program)
    } else if program_path.is_absolute() && program_path.is_file() {
        program_path.canonicalize().ok()
    } else {
        None
    };
    resolved.ok_or_else(|| format!("cannot resolve direct executable: {program}"))
}

fn resolve_allowed_inside(workspace: &Path, path: &Path) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "allowed_write_paths must not contain '..': {}",
            path.display()
        ));
    }
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    if !candidate.starts_with(workspace) {
        return Err(format!(
            "allowed_write_paths escapes canonical workspace {}: {}",
            workspace.display(),
            candidate.display()
        ));
    }
    let protected = protected_workspace_paths(workspace);
    if candidate == workspace
        || protected
            .iter()
            .any(|path| candidate.starts_with(path) || path.starts_with(&candidate))
    {
        return Err(format!(
            "allowed_write_paths overlaps protected workspace state: {}",
            candidate.display()
        ));
    }
    let mut existing = candidate.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| {
            format!(
                "allowed_write_paths has no existing ancestor: {}",
                candidate.display()
            )
        })?;
    }
    let canonical_existing = existing.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize allowed_write_paths ancestor {}: {error}",
            existing.display()
        )
    })?;
    if !canonical_existing.starts_with(workspace) {
        return Err(format!(
            "allowed_write_paths escapes canonical workspace through a symlink: {}",
            candidate.display()
        ));
    }
    let missing_suffix = candidate
        .strip_prefix(existing)
        .expect("existing ancestor belongs to candidate");
    let resolved = canonical_existing.join(missing_suffix);
    if resolved == workspace
        || protected
            .iter()
            .any(|path| resolved.starts_with(path) || path.starts_with(&resolved))
    {
        return Err(format!(
            "allowed_write_paths resolves into protected workspace state: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn resolve_inside(workspace: &Path, path: &Path, field: &str) -> Result<PathBuf, String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("{field} must not contain '..': {}", path.display()));
    }
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    let canonical = candidate.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize {field} {}: {error}",
            candidate.display()
        )
    })?;
    if !canonical.starts_with(workspace) {
        return Err(format!(
            "{field} escapes canonical workspace {}: {}",
            workspace.display(),
            canonical.display()
        ));
    }
    Ok(canonical)
}

#[derive(Debug)]
struct GitIdentity {
    commit_hash: String,
    tree_hash: String,
}

fn git_identity(workspace: &Path) -> Result<GitIdentity, TestExecutionError> {
    git_identity_with_program(workspace, "git")
}

fn git_identity_with_program(
    workspace: &Path,
    program: &str,
) -> Result<GitIdentity, TestExecutionError> {
    let inside = Command::new(program)
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::GitUnavailable,
                format!("cannot execute git identity probe: {error}"),
            )
        })?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return Err(TestExecutionError::new(
            TestExecutionErrorCode::NotGitWorkspace,
            format!(
                "not a git workspace: {}",
                String::from_utf8_lossy(&inside.stderr).trim()
            ),
        ));
    }

    let commit = git_rev_parse(program, workspace, "HEAD")?;
    match commit {
        Some(commit_hash) => {
            let tree_hash = git_rev_parse(program, workspace, "HEAD^{tree}")?.ok_or_else(|| {
                TestExecutionError::new(
                    TestExecutionErrorCode::GitIdentityInvalid,
                    "HEAD exists but its tree cannot be resolved",
                )
            })?;
            if !ags_platform::is_git_commit(&commit_hash)
                || !ags_platform::is_git_commit(&tree_hash)
            {
                return Err(TestExecutionError::new(
                    TestExecutionErrorCode::GitIdentityInvalid,
                    "git returned a malformed commit or tree object id",
                ));
            }
            Ok(GitIdentity {
                commit_hash,
                tree_hash,
            })
        }
        None => {
            let symbolic_head = Command::new(program)
                .arg("-C")
                .arg(workspace)
                .args(["symbolic-ref", "-q", "HEAD"])
                .stdin(Stdio::null())
                .output()
                .map_err(|error| {
                    TestExecutionError::new(
                        TestExecutionErrorCode::GitUnavailable,
                        format!("cannot execute git unborn probe: {error}"),
                    )
                })?;
            let object_count = Command::new(program)
                .arg("-C")
                .arg(workspace)
                .args(["rev-list", "--all", "--count"])
                .stdin(Stdio::null())
                .output()
                .map_err(|error| {
                    TestExecutionError::new(
                        TestExecutionErrorCode::GitUnavailable,
                        format!("cannot execute git object-count probe: {error}"),
                    )
                })?;
            let count = String::from_utf8_lossy(&object_count.stdout);
            if symbolic_head.status.success()
                && object_count.status.success()
                && count.trim() == "0"
            {
                Ok(GitIdentity {
                    commit_hash: "unborn".to_string(),
                    tree_hash: "unborn".to_string(),
                })
            } else {
                Err(TestExecutionError::new(
                    TestExecutionErrorCode::GitIdentityInvalid,
                    "HEAD cannot be resolved and repository is not explicitly unborn",
                ))
            }
        }
    }
}

fn git_rev_parse(
    program: &str,
    workspace: &Path,
    revision: &str,
) -> Result<Option<String>, TestExecutionError> {
    let output = Command::new(program)
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--verify", revision])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::GitUnavailable,
                format!("cannot execute git rev-parse: {error}"),
            )
        })?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout).map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::GitIdentityInvalid,
                format!("git object id is not UTF-8: {error}"),
            )
        })?;
        Ok(Some(value.trim().to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(target_os = "macos")]
fn sandboxed_command(
    workspace: &Path,
    spec: &CommandSpec,
    cwd: &Path,
    allowed: &[PathBuf],
    _allow_descendants: bool,
) -> Result<SandboxedCommand, TestExecutionError> {
    let sandbox_exec = macos_seatbelt_path().map_err(|reason| {
        TestExecutionError::new(TestExecutionErrorCode::SandboxUnavailable, reason)
    })?;
    let scratch = isolated_scratch()?;
    let resolved_program = resolve_direct_program(&spec.program)
        .map_err(|error| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, error))?;
    let scratch_path = scratch.path().canonicalize().map_err(|error| {
        TestExecutionError::new(
            TestExecutionErrorCode::SandboxUnavailable,
            format!("cannot canonicalize isolated LocalExecution scratch: {error}"),
        )
    })?;
    let mut profile = String::from("(version 1)\n(deny default)\n");
    // Seatbelt has no revocable process-tree membership primitive. Local
    // execution therefore permits direct exec but denies fork: the dedicated
    // session has exactly one killable process. Multi-process project tools
    // remain available through the separately authorized HostDelegated path.
    profile.push_str("(allow process-exec)\n(deny process-fork)\n");
    profile.push_str("(allow file-read*)\n(allow sysctl-read)\n(allow mach-lookup)\n");
    profile.push_str("(allow file-write* (literal \"/dev/null\"))\n");
    profile.push_str(&format!(
        "(allow file-write* (subpath \"{}\"))\n",
        sandbox_escape(&scratch_path)?
    ));
    for path in allowed {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            sandbox_escape(path)?
        ));
    }
    for protected in protected_workspace_paths(workspace) {
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            sandbox_escape(&protected)?
        ));
    }
    let mut command = Command::new(sandbox_exec);
    command
        .arg("-p")
        .arg(profile)
        .arg("--")
        .arg(&resolved_program)
        .args(&spec.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(&spec.env)
        .env(
            "PATH",
            spec.env.get("PATH").map_or_else(
                || {
                    resolved_program
                        .parent()
                        .unwrap_or(Path::new("/usr/bin"))
                        .as_os_str()
                },
                String::as_ref,
            ),
        )
        .env("TMPDIR", &scratch_path)
        .env("TMP", &scratch_path)
        .env("TEMP", &scratch_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(SandboxedCommand {
        command,
        backend: "macos-seatbelt".to_string(),
        _scratch: Some(scratch),
    })
}

#[cfg(target_os = "linux")]
fn sandboxed_command(
    _workspace: &Path,
    spec: &CommandSpec,
    cwd: &Path,
    allowed: &[PathBuf],
    _allow_descendants: bool,
) -> Result<SandboxedCommand, TestExecutionError> {
    let bubblewrap = linux_bubblewrap_path().map_err(|reason| {
        TestExecutionError::new(TestExecutionErrorCode::SandboxUnavailable, reason)
    })?;
    let scratch = isolated_scratch()?;
    let resolved_program = resolve_direct_program(&spec.program)
        .map_err(|error| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, error))?;
    let scratch_path = scratch.path().canonicalize().map_err(|error| {
        TestExecutionError::new(
            TestExecutionErrorCode::SandboxUnavailable,
            format!("cannot canonicalize isolated LocalExecution scratch: {error}"),
        )
    })?;
    let mut command = Command::new(bubblewrap);
    command
        .args([
            "--die-with-parent",
            "--new-session",
            "--unshare-all",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
        ])
        .arg("--bind")
        .arg(&scratch_path)
        .arg(&scratch_path)
        .arg("--chdir")
        .arg(cwd);
    for path in allowed {
        if !path.exists() {
            return Err(TestExecutionError::new(
                TestExecutionErrorCode::InvalidSpec,
                format!(
                    "linux-bubblewrap requires each allowed write root to exist before execution: {}",
                    path.display()
                ),
            ));
        }
        command.arg("--bind").arg(path).arg(path);
    }
    command
        .arg("--")
        .arg(&resolved_program)
        .args(&spec.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(&spec.env)
        .env(
            "PATH",
            spec.env.get("PATH").map_or_else(
                || {
                    resolved_program
                        .parent()
                        .unwrap_or(Path::new("/usr/bin"))
                        .as_os_str()
                },
                String::as_ref,
            ),
        )
        .env("TMPDIR", &scratch_path)
        .env("TMP", &scratch_path)
        .env("TEMP", &scratch_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(SandboxedCommand {
        command,
        backend: "linux-bubblewrap".to_string(),
        _scratch: Some(scratch),
    })
}

#[cfg(target_os = "linux")]
fn linux_bubblewrap_path() -> Result<PathBuf, String> {
    let bubblewrap = ags_platform::find_in_path("bwrap").ok_or_else(|| {
        "Linux LocalExecution is blocked: bubblewrap is not installed or visible on PATH"
            .to_string()
    })?;
    let true_program = ags_platform::find_in_path("true").ok_or_else(|| {
        "Linux LocalExecution is blocked: cannot resolve the containment probe executable"
            .to_string()
    })?;
    let status = Command::new(&bubblewrap)
        .args([
            "--die-with-parent",
            "--new-session",
            "--unshare-all",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
        ])
        .arg(true_program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| {
            format!("Linux LocalExecution is blocked: cannot probe bubblewrap: {error}")
        })?;
    if !status.success() {
        return Err(format!(
            "Linux LocalExecution is blocked: bubblewrap containment probe exited with {status}"
        ));
    }
    if ags_platform::find_in_path("ps").is_none() {
        return Err(
            "Linux LocalExecution is blocked: the audited process-table probe is unavailable"
                .to_string(),
        );
    }
    Ok(bubblewrap)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn sandboxed_command(
    _workspace: &Path,
    _spec: &CommandSpec,
    _cwd: &Path,
    _allowed: &[PathBuf],
    _allow_descendants: bool,
) -> Result<SandboxedCommand, TestExecutionError> {
    match local_execution_platform_support() {
        LocalExecutionPlatformSupport::Blocked { error_code, reason } => {
            Err(TestExecutionError::new(error_code, reason))
        }
        LocalExecutionPlatformSupport::Supported { .. } => {
            unreachable!("unsupported builds cannot report a LocalExecution backend")
        }
    }
}

struct SandboxedCommand {
    command: Command,
    backend: String,
    _scratch: Option<tempfile::TempDir>,
}

fn host_delegated_command(
    spec: &CommandSpec,
    cwd: &Path,
) -> Result<SandboxedCommand, TestExecutionError> {
    let resolved_program = resolve_direct_program(&spec.program)
        .map_err(|error| TestExecutionError::new(TestExecutionErrorCode::InvalidSpec, error))?;
    let mut command = Command::new(&resolved_program);
    command
        .args(&spec.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(&spec.env)
        .env(
            "PATH",
            spec.env.get("PATH").map_or_else(
                || {
                    resolved_program
                        .parent()
                        .unwrap_or(Path::new("/usr/bin"))
                        .as_os_str()
                },
                String::as_ref,
            ),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(SandboxedCommand {
        command,
        backend: "host-delegated".to_string(),
        _scratch: None,
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn isolated_scratch() -> Result<tempfile::TempDir, TestExecutionError> {
    tempfile::Builder::new()
        .prefix("ags-local-execution-")
        .tempdir()
        .map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::SandboxUnavailable,
                format!("cannot create isolated LocalExecution scratch: {error}"),
            )
        })
}

fn protected_workspace_paths(workspace: &Path) -> [PathBuf; 8] {
    [
        workspace.join(".git"),
        workspace.join(".ags"),
        workspace.join("task"),
        workspace.join("protocol"),
        workspace.join("config"),
        workspace.join("AGENTS.md"),
        workspace.join("CLAUDE.md"),
        workspace.join("AGENT_SUITE_PROTOCOL.md"),
    ]
}

#[cfg(target_os = "macos")]
fn sandbox_escape(path: &Path) -> Result<String, TestExecutionError> {
    let value = path.to_string_lossy();
    if value.chars().any(char::is_control) {
        return Err(TestExecutionError::new(
            TestExecutionErrorCode::InvalidSpec,
            format!(
                "sandbox path contains control characters: {}",
                path.display()
            ),
        ));
    }
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn macos_seatbelt_path() -> Result<PathBuf, String> {
    let sandbox = PathBuf::from(MACOS_SANDBOX_EXEC);
    let metadata = std::fs::symlink_metadata(&sandbox).map_err(|error| {
        format!("macOS LocalExecution is blocked: cannot inspect {MACOS_SANDBOX_EXEC}: {error}")
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "macOS LocalExecution is blocked: {MACOS_SANDBOX_EXEC} is not a fixed regular executable"
        ));
    }

    let scratch = tempfile::Builder::new()
        .prefix("ags-seatbelt-probe-")
        .tempdir()
        .map_err(|error| {
            format!("macOS LocalExecution is blocked: cannot create probe scratch: {error}")
        })?;
    let scratch_path = scratch.path().canonicalize().map_err(|error| {
        format!("macOS LocalExecution is blocked: cannot canonicalize probe scratch: {error}")
    })?;
    let allowed = scratch_path.join("allowed");
    let denied = scratch_path
        .parent()
        .ok_or_else(|| "macOS LocalExecution is blocked: probe scratch has no parent".to_string())?
        .join(format!("ags-seatbelt-denied-{}", std::process::id()));
    let _ = std::fs::remove_file(&denied);
    let profile = format!(
        "(version 1)\n(deny default)\n(allow process*)\n(allow file-read*)\n(allow sysctl-read)\n(allow mach-lookup)\n(allow file-write* (subpath \"{}\"))\n",
        sandbox_escape(&scratch_path).map_err(|error| error.to_string())?
    );
    let allowed_status = Command::new(&sandbox)
        .arg("-p")
        .arg(&profile)
        .arg("--")
        .arg("/usr/bin/touch")
        .arg(&allowed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| {
            format!("macOS LocalExecution is blocked: Seatbelt self-probe could not start: {error}")
        })?;
    if !allowed_status.success() || !allowed.is_file() {
        return Err(format!(
            "macOS LocalExecution is blocked: Seatbelt self-probe could not write its isolated scratch (status {allowed_status})"
        ));
    }
    let denied_status = Command::new(&sandbox)
        .arg("-p")
        .arg(profile)
        .arg("--")
        .arg("/usr/bin/touch")
        .arg(&denied)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| {
            format!("macOS LocalExecution is blocked: Seatbelt deny probe could not start: {error}")
        })?;
    if denied_status.success() || denied.exists() {
        let _ = std::fs::remove_file(&denied);
        return Err(
            "macOS LocalExecution is blocked: Seatbelt self-probe allowed an undeclared write"
                .to_string(),
        );
    }
    Ok(sandbox)
}

fn configure_execution_membership(
    _command: &mut Command,
) -> Result<Option<String>, TestExecutionError> {
    Ok(None)
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) -> Result<(), TestExecutionError> {
    // SAFETY: `pre_exec` runs only async-signal-safe `setsid` and constructs no
    // heap state in the child. The parent has no other pre-exec hooks.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) -> Result<(), TestExecutionError> {
    Err(TestExecutionError::new(
        TestExecutionErrorCode::SandboxUnavailable,
        "process-group containment is unavailable on this platform",
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn terminate_process_tree(
    root_pid: u32,
    _execution_membership: Option<&str>,
) -> Result<(), TestExecutionError> {
    signal_process(root_pid, libc::SIGSTOP)?;
    let mut known = BTreeSet::from([root_pid]);
    let freeze_deadline = Instant::now() + Duration::from_secs(2);
    let mut stable_rounds = 0_u8;

    while stable_rounds < 2 {
        if Instant::now() >= freeze_deadline {
            let _ = kill_process_group(root_pid);
            return Err(TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                "cannot freeze a stable descendant tree before timeout termination",
            ));
        }
        let table = process_table()?;
        let discovered = descendant_pids(root_pid, &table);
        let before = known.len();
        for pid in discovered {
            signal_process(pid, libc::SIGSTOP)?;
            known.insert(pid);
        }
        if known.len() == before {
            stable_rounds += 1;
        } else {
            stable_rounds = 0;
        }
        thread::sleep(Duration::from_millis(10));
    }

    for pid in known.iter().rev().copied() {
        signal_process(pid, libc::SIGKILL)?;
    }
    kill_process_group(root_pid)?;

    let confirmation_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let table = process_table()?;
        let live = table
            .iter()
            .filter_map(|(&pid, observation)| {
                ((known.contains(&pid) || observation.process_group == root_pid)
                    && !observation.zombie)
                    .then_some(pid)
            })
            .collect::<Vec<_>>();
        if live.is_empty() {
            return Ok(());
        }
        for pid in live {
            signal_process(pid, libc::SIGKILL)?;
        }
        if Instant::now() >= confirmation_deadline {
            return Err(TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                format!(
                    "timed-out process tree still has live descendants after termination: {:?}",
                    known
                ),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn terminate_remaining_processes(
    root_pid: u32,
    _execution_membership: Option<&str>,
) -> Result<(), TestExecutionError> {
    kill_process_group(root_pid)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let table = process_table()?;
        let live = table
            .iter()
            .filter_map(|(&pid, observation)| {
                (observation.process_group == root_pid && !observation.zombie).then_some(pid)
            })
            .collect::<Vec<_>>();
        if live.is_empty() {
            return Ok(());
        }
        for pid in live {
            signal_process(pid, libc::SIGKILL)?;
        }
        if Instant::now() >= deadline {
            return Err(TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                format!("LocalExecution process group {root_pid} remained live after termination"),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "linux")]
fn terminate_remaining_processes(
    _root_pid: u32,
    _execution_membership: Option<&str>,
) -> Result<(), TestExecutionError> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn terminate_process_tree(
    root_pid: u32,
    execution_membership: Option<&str>,
) -> Result<(), TestExecutionError> {
    let _ = (root_pid, execution_membership);
    Err(TestExecutionError::new(
        TestExecutionErrorCode::SandboxUnavailable,
        "audited descendant-tree termination is unavailable on this platform",
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn terminate_remaining_processes(
    _root_pid: u32,
    _execution_membership: Option<&str>,
) -> Result<(), TestExecutionError> {
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Clone, Copy)]
struct ProcessObservation {
    parent_pid: u32,
    process_group: u32,
    zombie: bool,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn process_table() -> Result<BTreeMap<u32, ProcessObservation>, TestExecutionError> {
    #[cfg(target_os = "macos")]
    let ps = PathBuf::from("/bin/ps");
    #[cfg(target_os = "linux")]
    let ps = ags_platform::find_in_path("ps").ok_or_else(|| {
        TestExecutionError::new(
            TestExecutionErrorCode::SandboxUnavailable,
            "audited process-table probe is unavailable",
        )
    })?;
    let output = Command::new(ps)
        .args(["-axo", "pid=,ppid=,pgid=,state="])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                format!("cannot enumerate descendant process table: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(TestExecutionError::new(
            TestExecutionErrorCode::ProcessFailed,
            format!(
                "process-table probe failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|error| {
        TestExecutionError::new(
            TestExecutionErrorCode::ProcessFailed,
            format!("process-table output is not UTF-8: {error}"),
        )
    })?;
    let mut table = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let pid = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                TestExecutionError::new(
                    TestExecutionErrorCode::ProcessFailed,
                    format!("invalid process-table pid row: {line}"),
                )
            })?;
        let parent_pid = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                TestExecutionError::new(
                    TestExecutionErrorCode::ProcessFailed,
                    format!("invalid process-table parent row: {line}"),
                )
            })?;
        let process_group = fields
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| {
                TestExecutionError::new(
                    TestExecutionErrorCode::ProcessFailed,
                    format!("invalid process-table group row: {line}"),
                )
            })?;
        let state = fields.next().ok_or_else(|| {
            TestExecutionError::new(
                TestExecutionErrorCode::ProcessFailed,
                format!("invalid process-table state row: {line}"),
            )
        })?;
        table.insert(
            pid,
            ProcessObservation {
                parent_pid,
                process_group,
                zombie: state.starts_with('Z'),
            },
        );
    }
    Ok(table)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn descendant_pids(root_pid: u32, table: &BTreeMap<u32, ProcessObservation>) -> BTreeSet<u32> {
    let mut descendants = BTreeSet::new();
    let mut frontier = vec![root_pid];
    while let Some(parent) = frontier.pop() {
        for (&pid, observation) in table {
            if observation.parent_pid == parent && descendants.insert(pid) {
                frontier.push(pid);
            }
        }
    }
    descendants
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn signal_process(pid: u32, signal: i32) -> Result<(), TestExecutionError> {
    // SAFETY: positive pid targets exactly the discovered process and signal is
    // SIGSTOP or SIGKILL. ESRCH means the process already exited.
    let result = unsafe { libc::kill(pid as i32, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(TestExecutionError::new(
            TestExecutionErrorCode::ProcessFailed,
            format!("cannot signal descendant process {pid}: {error}"),
        ))
    }
}

#[cfg(unix)]
fn kill_process_group(process_group: u32) -> Result<(), TestExecutionError> {
    // SAFETY: negative pid addresses the dedicated session/process group
    // created immediately before exec. SIGKILL is used only after timeout.
    let result = unsafe { libc::kill(-(process_group as i32), libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(TestExecutionError::new(
            TestExecutionErrorCode::ProcessFailed,
            format!("cannot kill timed-out process group {process_group}: {error}"),
        ))
    }
}

#[cfg(not(unix))]
fn kill_process_group(_process_group: u32) -> Result<(), TestExecutionError> {
    Err(TestExecutionError::new(
        TestExecutionErrorCode::SandboxUnavailable,
        "process-group termination is unavailable on this platform",
    ))
}

fn argv_hash(spec: &CommandSpec) -> Result<String, String> {
    #[derive(Serialize)]
    struct Argv<'a> {
        program: &'a str,
        argv: &'a [String],
        cwd: &'a Path,
        env: &'a BTreeMap<String, String>,
        timeout_ms: u64,
        allowed_write_paths: &'a [PathBuf],
    }
    let bytes = serde_json::to_vec(&Argv {
        program: &spec.program,
        argv: &spec.argv,
        cwd: &spec.cwd,
        env: &spec.env,
        timeout_ms: spec.timeout_ms,
        allowed_write_paths: &spec.allowed_write_paths,
    })
    .map_err(|error| format!("cannot serialize command spec: {error}"))?;
    Ok(ags_platform::sha256(bytes))
}

fn snapshot_tree(
    workspace: &Path,
    allowed_write_paths: &[PathBuf],
) -> Result<BTreeMap<String, TreeEntry>, String> {
    snapshot_tree_with_limits(
        workspace,
        allowed_write_paths,
        SnapshotLimits {
            max_entries: SNAPSHOT_MAX_ENTRIES,
            max_depth: SNAPSHOT_MAX_DEPTH,
            max_hashed_bytes: SNAPSHOT_MAX_HASHED_BYTES,
        },
    )
}

#[derive(Clone, Copy)]
struct SnapshotLimits {
    max_entries: usize,
    max_depth: usize,
    max_hashed_bytes: u64,
}

fn snapshot_tree_with_limits(
    workspace: &Path,
    allowed_write_paths: &[PathBuf],
    limits: SnapshotLimits,
) -> Result<BTreeMap<String, TreeEntry>, String> {
    let mut entries = BTreeMap::new();
    let mut hashed_bytes = 0_u64;
    let mut pending = vec![workspace.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let children = std::fs::read_dir(&directory)
            .map_err(|error| format!("cannot inspect {}: {error}", directory.display()))?;
        for child in children {
            let child =
                child.map_err(|error| format!("cannot inspect directory entry: {error}"))?;
            let path = child.path();
            let relative = path
                .strip_prefix(workspace)
                .expect("snapshot path stays inside workspace");
            let depth = relative.components().count();
            if depth > limits.max_depth {
                return Err(format!(
                    "workspace audit exceeds maximum depth {}: {}",
                    limits.max_depth,
                    relative.display()
                ));
            }
            if entries.len() >= limits.max_entries {
                return Err(format!(
                    "workspace audit exceeds maximum entry count {}",
                    limits.max_entries
                ));
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            let modified_ns = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let file_type = metadata.file_type();
            let kind = if file_type.is_dir() {
                "directory"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "file"
            };
            let key = relative.to_string_lossy().into_owned();
            let inside_allowed = allowed_write_paths
                .iter()
                .any(|allowed| path == *allowed || path.starts_with(allowed));
            let content_digest = if file_type.is_file() && !inside_allowed {
                hashed_bytes = hashed_bytes.checked_add(metadata.len()).ok_or_else(|| {
                    "workspace audit hashed-byte accounting overflowed".to_string()
                })?;
                if hashed_bytes > limits.max_hashed_bytes {
                    return Err(format!(
                        "workspace audit exceeds maximum hashed bytes {}",
                        limits.max_hashed_bytes
                    ));
                }
                Some(streaming_file_digest(&path)?)
            } else {
                None
            };
            entries.insert(
                key.clone(),
                TreeEntry {
                    path: key,
                    kind,
                    len: metadata.len(),
                    modified_ns,
                    link_target: if file_type.is_symlink() {
                        std::fs::read_link(&path)
                            .ok()
                            .map(|target| target.to_string_lossy().into_owned())
                    } else {
                        None
                    },
                    // Build roots can contain gigabytes of outputs. They are
                    // represented by the allowed root entry itself; every
                    // path outside that envelope, including `.git` and
                    // `.codegraph`, remains content-sensitive audit evidence.
                    content_digest,
                },
            );
            if file_type.is_dir() && !allowed_write_paths.contains(&path) {
                pending.push(path);
            }
        }
    }
    Ok(entries)
}

/// True streaming SHA-256 with constant memory. This detects content
/// replacement even when size and mtime are preserved without buffering an
/// arbitrarily large build artifact or accumulating a chunk manifest.
fn streaming_file_digest(path: &Path) -> Result<String, String> {
    const CHUNK_BYTES: usize = 64 * 1024;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    let mut hasher = Sha256::new();
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("cannot hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn digest_tree(entries: &BTreeMap<String, TreeEntry>) -> String {
    ags_platform::sha256(serde_json::to_vec(entries).expect("tree snapshot serializes"))
}

fn changed_paths(
    before: &BTreeMap<String, TreeEntry>,
    after: &BTreeMap<String, TreeEntry>,
) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| {
            before
                .get(path)
                .or_else(|| after.get(path))
                .is_some_and(|entry| entry.kind != "directory")
        })
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str], allowed: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.to_string(),
            argv: args.iter().map(|arg| (*arg).to_string()).collect(),
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
            timeout_ms: 2_000,
            allowed_write_paths: allowed.iter().map(PathBuf::from).collect(),
        }
    }

    #[cfg(target_os = "macos")]
    fn run_macos_seatbelt_policy_probe(
        workspace: &Path,
        spec: &CommandSpec,
    ) -> std::process::Output {
        let workspace = workspace.canonicalize().unwrap();
        validate_spec(&workspace, spec).unwrap();
        let cwd = resolve_inside(&workspace, &spec.cwd, "cwd").unwrap();
        let allowed = spec
            .allowed_write_paths
            .iter()
            .map(|path| resolve_allowed_inside(&workspace, path))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut sandbox = sandboxed_command(&workspace, spec, &cwd, &allowed, true).unwrap();
        configure_process_group(&mut sandbox.command).unwrap();
        let child = sandbox.command.spawn().unwrap();
        let process_group = child.id();
        let output = child.wait_with_output().unwrap();
        terminate_remaining_processes(process_group, None).unwrap();
        output
    }

    fn init_git(workspace: &Path) {
        std::fs::write(workspace.join("seed.txt"), b"seed\n").unwrap();
        for args in [&["init", "-q"][..], &["add", "seed.txt"][..]] {
            assert!(Command::new("git")
                .arg("-C")
                .arg(workspace)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        assert!(Command::new("git")
            .arg("-C")
            .arg(workspace)
            .args(["commit", "-qm", "fixture"])
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .status()
            .unwrap()
            .success());
    }

    #[test]
    #[cfg(unix)]
    fn host_delegated_runner_closes_success_failure_and_unexpected_write_receipts() {
        let success_workspace = tempfile::tempdir().unwrap();
        init_git(success_workspace.path());
        std::fs::create_dir(success_workspace.path().join("out")).unwrap();
        let success = run_host_project_test(
            success_workspace.path(),
            TestProfile::Smoke,
            &spec("/usr/bin/touch", &["out/result"], &["out"]),
        )
        .unwrap();
        assert_eq!(success.status, TestExecutionStatus::Succeeded);
        assert_eq!(success.sandbox_backend, "host-delegated");
        assert_eq!(success.observed_write_set, ["out"]);
        assert!(success.unexpected_write_set.is_empty());
        assert!(success.closed);
        assert!(!success.source_rollback_performed);

        let failure_workspace = tempfile::tempdir().unwrap();
        init_git(failure_workspace.path());
        let failure = run_host_project_test(
            failure_workspace.path(),
            TestProfile::Standard,
            &spec("/usr/bin/false", &[], &[]),
        )
        .unwrap();
        assert_eq!(failure.status, TestExecutionStatus::Failed);
        assert!(failure.closed);
        assert!(!failure.source_rollback_performed);

        let risk_workspace = tempfile::tempdir().unwrap();
        init_git(risk_workspace.path());
        let risk = run_host_project_test(
            risk_workspace.path(),
            TestProfile::Full,
            &spec("/usr/bin/touch", &["unexpected"], &[]),
        )
        .unwrap();
        assert_eq!(risk.status, TestExecutionStatus::RiskEscalated);
        assert_eq!(risk.observed_write_set, ["unexpected"]);
        assert_eq!(risk.unexpected_write_set, ["unexpected"]);
        assert!(risk.closed);
        assert!(!risk.source_rollback_performed);
    }

    #[test]
    #[cfg(unix)]
    fn host_delegated_audit_detects_protected_git_and_codegraph_writes() {
        for relative in [".git/evil", ".codegraph/evil"] {
            let workspace = tempfile::tempdir().unwrap();
            init_git(workspace.path());
            std::fs::create_dir_all(workspace.path().join(".codegraph")).unwrap();
            let receipt = run_host_project_test(
                workspace.path(),
                TestProfile::Full,
                &spec("/usr/bin/touch", &[relative], &[]),
            )
            .unwrap();
            assert_eq!(receipt.status, TestExecutionStatus::RiskEscalated);
            assert_eq!(receipt.unexpected_write_set, [relative]);
        }
    }

    #[test]
    #[cfg(unix)]
    fn host_delegated_noop_does_not_fabricate_an_allowed_write() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let receipt = run_host_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec("/usr/bin/true", &[], &["out"]),
        )
        .unwrap();
        assert!(receipt.observed_write_set.is_empty());
        assert!(receipt.unexpected_write_set.is_empty());
    }

    #[cfg(target_os = "linux")]
    fn linux_local_execution_available() -> bool {
        matches!(
            local_execution_platform_support(),
            LocalExecutionPlatformSupport::Supported { .. }
        )
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn runs_direct_argv_and_binds_complete_receipt() {
        if !linux_local_execution_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec("touch", &["out/result"], &["out"]),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Succeeded);
        assert!(receipt.closed);
        assert!(!receipt.source_rollback_performed);
        assert!(receipt.argv_hash.starts_with("sha256:"));
        assert!(ags_platform::is_git_commit(&receipt.commit_hash));
        assert!(ags_platform::is_git_commit(&receipt.tree_hash));
        assert!(receipt.workspace_tree_hash.starts_with("sha256:"));
        assert!(receipt.output_digest.starts_with("sha256:"));
        assert_eq!(receipt.sandbox_backend, "linux-bubblewrap");
        assert!(!receipt.timeout_descendants_terminated);
        assert_eq!(receipt.observed_write_set, ["out"]);
        assert!(receipt.unexpected_write_set.is_empty());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_failure_is_closed_without_source_rollback() {
        if !linux_local_execution_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Standard,
            &spec("false", &[], &[]),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Failed);
        assert!(receipt.closed);
        assert!(!receipt.source_rollback_performed);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn protected_workspace_write_is_blocked_by_sandbox() {
        if !linux_local_execution_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Full,
            &spec("touch", &["source.txt"], &[]),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Failed);
        assert!(receipt.unexpected_write_set.is_empty());
        assert!(!workspace.path().join("source.txt").exists());
    }

    #[test]
    fn path_escape_and_shell_text_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let mut escaping = spec("true", &[], &[]);
        escaping.cwd = PathBuf::from("..");
        assert!(
            run_project_test(workspace.path(), TestProfile::Smoke, &escaping)
                .unwrap_err()
                .contains("must not contain '..'")
        );

        let shell_text = spec("echo ok && touch pwned", &[], &[]);
        assert!(
            run_project_test(workspace.path(), TestProfile::Smoke, &shell_text)
                .unwrap_err()
                .contains("cannot resolve direct executable")
        );
        assert!(!workspace.path().join("pwned").exists());

        let shell = spec("sh", &["-c", "touch pwned"], &[]);
        assert!(
            run_project_test(workspace.path(), TestProfile::Smoke, &shell)
                .unwrap_err()
                .contains("not a shell")
        );
        assert!(!workspace.path().join("pwned").exists());
    }

    #[test]
    fn same_size_same_mtime_content_tamper_changes_snapshot() {
        let before = BTreeMap::from([(
            "source.rs".to_string(),
            TreeEntry {
                path: "source.rs".to_string(),
                kind: "file",
                len: 4,
                modified_ns: 42,
                link_target: None,
                content_digest: Some(ags_platform::sha256(b"safe")),
            },
        )]);
        let after = BTreeMap::from([(
            "source.rs".to_string(),
            TreeEntry {
                content_digest: Some(ags_platform::sha256(b"evil")),
                ..before["source.rs"].clone()
            },
        )]);
        assert_ne!(digest_tree(&before), digest_tree(&after));
        assert_eq!(changed_paths(&before, &after), ["source.rs"]);
    }

    #[test]
    fn workspace_audit_limits_fail_closed() {
        let entries = tempfile::tempdir().unwrap();
        std::fs::write(entries.path().join("a"), b"a").unwrap();
        std::fs::write(entries.path().join("b"), b"b").unwrap();
        let entry_error = snapshot_tree_with_limits(
            entries.path(),
            &[],
            SnapshotLimits {
                max_entries: 1,
                max_depth: 8,
                max_hashed_bytes: 8,
            },
        )
        .unwrap_err();
        assert!(entry_error.contains("maximum entry count 1"));

        let bytes = tempfile::tempdir().unwrap();
        std::fs::write(bytes.path().join("payload"), b"ab").unwrap();
        let byte_error = snapshot_tree_with_limits(
            bytes.path(),
            &[],
            SnapshotLimits {
                max_entries: 8,
                max_depth: 8,
                max_hashed_bytes: 1,
            },
        )
        .unwrap_err();
        assert!(byte_error.contains("maximum hashed bytes 1"));

        let depth = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(depth.path().join("one/two")).unwrap();
        let depth_error = snapshot_tree_with_limits(
            depth.path(),
            &[],
            SnapshotLimits {
                max_entries: 8,
                max_depth: 1,
                max_hashed_bytes: 8,
            },
        )
        .unwrap_err();
        assert!(depth_error.contains("maximum depth 1"));
    }

    #[test]
    fn structured_profile_rejects_free_form_commands() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = workspace.path().join("profile.yaml");
        std::fs::write(
            &profile,
            "verification:\n  project_tests:\n    smoke: cargo test\n    standard: cargo test\n    full: cargo test\n",
        )
        .unwrap();
        assert!(load_project_test_profiles(workspace.path(), Path::new("profile.yaml")).is_err());
    }

    #[test]
    fn structured_profile_requires_exact_v2_schema() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = workspace.path().join("profile.yaml");
        std::fs::write(
            &profile,
            "verification:\n  project_tests:\n    smoke: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n    standard: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n    full: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n",
        )
        .unwrap();
        assert!(load_project_test_profiles(workspace.path(), Path::new("profile.yaml")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn profile_loader_is_root_bound_nofollow_regular_and_bounded() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let valid = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/project-profile-v2.yaml"
        ));
        std::fs::write(workspace.path().join("valid.yaml"), valid).unwrap();
        load_project_test_profiles(workspace.path(), Path::new("valid.yaml")).unwrap();

        symlink("valid.yaml", workspace.path().join("symlink.yaml")).unwrap();
        let error =
            load_project_test_profiles(workspace.path(), Path::new("symlink.yaml")).unwrap_err();
        assert!(
            error.contains("without following") || error.contains("regular"),
            "{error}"
        );

        let root_alias = workspace.path().with_extension("root-symlink");
        symlink(workspace.path(), &root_alias).unwrap();
        let error = load_project_test_profiles(&root_alias, Path::new("valid.yaml")).unwrap_err();
        std::fs::remove_file(&root_alias).unwrap();
        assert!(
            error.contains("workspace root without following"),
            "{error}"
        );

        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("valid.yaml"), valid).unwrap();
        symlink(outside.path(), workspace.path().join("linked-parent")).unwrap();
        let error =
            load_project_test_profiles(workspace.path(), Path::new("linked-parent/valid.yaml"))
                .unwrap_err();
        assert!(error.contains("parent without following"), "{error}");

        let fifo = workspace.path().join("profile.fifo");
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: fifo_name is a valid NUL-terminated path inside the temp workspace.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        let error =
            load_project_test_profiles(workspace.path(), Path::new("profile.fifo")).unwrap_err();
        assert!(error.contains("regular file"), "{error}");

        let oversized = workspace.path().join("oversized.yaml");
        std::fs::write(&oversized, vec![b'x'; PROJECT_PROFILE_MAX_BYTES + 1]).unwrap();
        let error =
            load_project_test_profiles(workspace.path(), Path::new("oversized.yaml")).unwrap_err();
        assert!(error.contains("exceeds"), "{error}");

        let error = load_project_test_profiles(Path::new("/"), Path::new("dev/null")).unwrap_err();
        assert!(error.contains("regular file"), "{error}");
    }

    #[test]
    fn command_spec_schema_is_single_source_and_rejects_legacy_field_names() {
        let legacy = "program: true\nargs: []\ncwd: .\nenv: {}\ntimeout_seconds: 1\nallowed_write_paths: []\n";
        assert!(serde_yaml::from_str::<CommandSpec>(legacy).is_err());

        let schema = serde_json::to_value(schemars::schema_for!(CommandSpec)).unwrap();
        let required = schema["required"].as_array().unwrap();
        for field in [
            "program",
            "argv",
            "cwd",
            "env",
            "timeout_ms",
            "allowed_write_paths",
        ] {
            assert!(required.iter().any(|value| value == field), "{field}");
        }
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("args").is_none());
        assert!(schema["properties"].get("timeout_seconds").is_none());
    }

    #[test]
    fn project_profile_protocol_rejects_legacy_command_spec_names() {
        let protocol = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol/project-profile.md"
        ));
        assert!(
            !protocol.contains("`args`"),
            "legacy args remains in protocol"
        );
        assert!(protocol.contains("`argv`"));
        assert!(!protocol.contains("timeout_seconds"));
    }

    #[test]
    fn non_git_workspace_fails_closed_before_execution() {
        let workspace = tempfile::tempdir().unwrap();
        let marker = workspace.path().join("should-not-exist");
        let error = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec("touch", &["should-not-exist"], &["should-not-exist"]),
        )
        .unwrap_err();
        assert!(error.contains("not a git workspace"), "{error}");
        assert!(!marker.exists());
    }

    #[test]
    fn git_identity_distinguishes_unborn_non_git_and_unavailable() {
        let unborn = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(unborn.path())
            .args(["init", "-q"])
            .status()
            .unwrap()
            .success());
        let identity = git_identity(unborn.path()).unwrap();
        assert_eq!(identity.commit_hash, "unborn");
        assert_eq!(identity.tree_hash, "unborn");

        let non_git = tempfile::tempdir().unwrap();
        assert_eq!(
            git_identity(non_git.path()).unwrap_err().code,
            TestExecutionErrorCode::NotGitWorkspace
        );
        assert_eq!(
            git_identity_with_program(non_git.path(), "ags-definitely-missing-git")
                .unwrap_err()
                .code,
            TestExecutionErrorCode::GitUnavailable
        );
    }

    #[test]
    fn profile_rejects_legacy_schema_and_unknown_root_fields() {
        let workspace = tempfile::tempdir().unwrap();
        let profile = workspace.path().join("profile.yaml");
        let commands = "verification:\n  project_tests:\n    smoke: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n    standard: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n    full: { program: \"true\", argv: [], cwd: ., env: {}, timeout_ms: 1000, allowed_write_paths: [] }\n";
        std::fs::write(&profile, format!("schema_version: 1\n{commands}")).unwrap();
        assert!(load_project_test_profiles(workspace.path(), Path::new("profile.yaml")).is_err());
        std::fs::write(
            &profile,
            format!("schema_version: {PROJECT_PROFILE_SCHEMA}\nunknown: true\n{commands}"),
        )
        .unwrap();
        assert!(load_project_test_profiles(workspace.path(), Path::new("profile.yaml")).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn external_transient_and_symlink_escape_writes_are_blocked() {
        if !linux_local_execution_available() {
            return;
        }
        let python = ags_platform::find_in_path("python3").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("external.txt");
        let external_literal = serde_json::to_string(&external.to_string_lossy()).unwrap();
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(
                &python.to_string_lossy(),
                &["-c", &format!("open({external_literal}, 'w').write('bad')")],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Failed);
        assert!(!external.exists());

        let transient = workspace.path().join("transient.txt");
        let transient_literal = serde_json::to_string(&transient.to_string_lossy()).unwrap();
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(
                &python.to_string_lossy(),
                &[
                    "-c",
                    &format!(
                        "import os; open({transient_literal}, 'w').write('bad'); os.unlink({transient_literal})"
                    ),
                ],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Failed);
        assert!(!transient.exists());

        std::os::unix::fs::symlink(outside.path(), workspace.path().join("escape")).unwrap();
        let error = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec("touch", &["escape/pwned"], &["escape"]),
        )
        .unwrap_err();
        assert_eq!(error.code, TestExecutionErrorCode::InvalidSpec);
        assert!(!outside.path().join("pwned").exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn timeout_kills_setsid_descendant_before_late_write() {
        if !linux_local_execution_available() {
            return;
        }
        let python = ags_platform::find_in_path("python3").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let late = workspace.path().join("out/late.txt");
        let late_literal = serde_json::to_string(&late.to_string_lossy()).unwrap();
        let child_code =
            format!("import time; time.sleep(2); open({late_literal}, 'w').write('late')");
        let parent_code = format!(
            "import subprocess, sys, time; subprocess.Popen([sys.executable, '-c', {}], start_new_session=True); time.sleep(30)",
            serde_json::to_string(&child_code).unwrap()
        );
        let mut command = spec(&python.to_string_lossy(), &["-c", &parent_code], &["out"]);
        command.timeout_ms = 1_000;
        let started = Instant::now();
        let receipt = run_project_test(workspace.path(), TestProfile::Smoke, &command).unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::TimedOut);
        assert!(receipt.timeout_descendants_terminated);
        assert!(started.elapsed() < Duration::from_secs(4));
        thread::sleep(Duration::from_secs(3));
        assert!(!late.exists());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn timeout_does_not_misreport_double_fork_reparent_as_contained() {
        #[cfg(target_os = "linux")]
        if !linux_local_execution_available() {
            return;
        }
        let python = ags_platform::find_in_path("python3").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let late = workspace.path().join("out/reparented-late.txt");
        let observed_parent = workspace.path().join("out/reparented-ppid.txt");
        let late_literal = serde_json::to_string(&late.to_string_lossy()).unwrap();
        let parent_literal = serde_json::to_string(&observed_parent.to_string_lossy()).unwrap();
        let code = format!(
            "import os, time\npid = os.fork()\nif pid == 0:\n    os.setsid()\n    daemon = os.fork()\n    if daemon > 0:\n        os._exit(0)\n    deadline = time.time() + 0.5\n    while os.getppid() != 1 and time.time() < deadline:\n        time.sleep(0.01)\n    open({parent_literal}, 'w').write(str(os.getppid()))\n    time.sleep(2)\n    open({late_literal}, 'w').write('late')\n    os._exit(0)\ntime.sleep(30)"
        );
        let mut command = spec(&python.to_string_lossy(), &["-c", &code], &["out"]);
        command.timeout_ms = 1_000;
        #[cfg(target_os = "macos")]
        {
            let receipt = run_project_test(workspace.path(), TestProfile::Smoke, &command).unwrap();
            assert_eq!(receipt.status, TestExecutionStatus::Failed);
            assert_eq!(receipt.sandbox_backend, "macos-seatbelt");
            assert!(!observed_parent.exists());
            assert!(!late.exists());
        }
        #[cfg(target_os = "linux")]
        let receipt = run_project_test(workspace.path(), TestProfile::Smoke, &command).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(receipt.status, TestExecutionStatus::TimedOut);
        #[cfg(target_os = "linux")]
        assert!(receipt.timeout_descendants_terminated);
        #[cfg(target_os = "linux")]
        assert_eq!(std::fs::read_to_string(observed_parent).unwrap(), "1");
        #[cfg(target_os = "linux")]
        thread::sleep(Duration::from_secs(3));
        #[cfg(target_os = "linux")]
        assert!(
            !late.exists(),
            "a reparented descendant wrote after a closed timeout receipt"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_backend_is_usable_or_structurally_unavailable() {
        let backend = match local_execution_platform_support() {
            LocalExecutionPlatformSupport::Supported { backend } => backend,
            LocalExecutionPlatformSupport::Blocked { error_code, reason } => {
                assert_eq!(error_code, TestExecutionErrorCode::SandboxUnavailable);
                assert!(reason.contains("Linux LocalExecution is blocked"));
                return;
            }
        };
        assert_eq!(backend, "linux-bubblewrap");

        let touch = ags_platform::find_in_path("touch").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(&touch.to_string_lossy(), &["out/result"], &["out"]),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Succeeded);
        assert_eq!(receipt.sandbox_backend, "linux-bubblewrap");
        assert!(workspace.path().join("out/result").exists());

        let outside = tempfile::tempdir().unwrap();
        let external = outside.path().join("blocked");
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(
                &touch.to_string_lossy(),
                &[&external.to_string_lossy()],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Failed);
        assert!(!external.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn output_capture_is_bounded_but_digest_covers_full_stream() {
        if !linux_local_execution_available() {
            return;
        }
        let python = ags_platform::find_in_path("python3").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(
                &python.to_string_lossy(),
                &["-c", "import sys; sys.stdout.write('x' * 1048576)"],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Succeeded);
        assert_eq!(receipt.output_bytes, 1_048_576);
        assert!(receipt.output_truncated);
        assert!(receipt.output_digest.starts_with("sha256:"));
    }

    #[test]
    fn streaming_digest_has_constant_memory_shape_and_budget() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("eight-mib.bin");
        std::fs::write(&path, vec![0x5a; 8 * 1024 * 1024]).unwrap();
        let started = Instant::now();
        let digest = streaming_file_digest(&path).unwrap();
        assert!(digest.starts_with("sha256:"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn tracked_profile_fixture_defines_all_three_structured_levels() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let profiles =
            load_project_test_profiles(&root, Path::new("tests/fixtures/project-profile-v2.yaml"))
                .unwrap();
        for level in [TestProfile::Smoke, TestProfile::Standard, TestProfile::Full] {
            let command = profiles.get(level);
            assert!(!command.program.is_empty());
            assert!(!command.argv.is_empty());
            assert!(command.timeout_ms > 0);
            assert_eq!(command.cwd, Path::new("."));
        }
    }

    #[test]
    fn ignored_local_profile_is_validated_when_present_but_is_not_authority() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("config/agent-project-profile.yaml");
        if path.is_file() {
            load_project_test_profiles(&root, Path::new("config/agent-project-profile.yaml"))
                .unwrap();
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_seatbelt_allows_only_the_declared_write_root() {
        assert_eq!(
            local_execution_platform_support(),
            LocalExecutionPlatformSupport::Supported {
                backend: "macos-seatbelt".to_string()
            }
        );
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec("/usr/bin/touch", &["out/allowed"], &["out"]),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Succeeded);
        assert_eq!(receipt.sandbox_backend, "macos-seatbelt");
        assert!(workspace.path().join("out/allowed").is_file());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_seatbelt_denies_workspace_protected_outside_symlink_child_and_grandchild_writes() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        std::fs::create_dir(workspace.path().join(".ags")).unwrap();
        std::fs::create_dir(workspace.path().join("task")).unwrap();
        std::fs::create_dir(workspace.path().join("protocol")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("out/escape")).unwrap();

        let denied = [
            workspace.path().join("workspace-denied"),
            workspace.path().join(".git/denied"),
            workspace.path().join(".ags/denied"),
            workspace.path().join("task/denied"),
            workspace.path().join("protocol/denied"),
            outside.path().join("outside-denied"),
            workspace.path().join("out/escape/symlink-denied"),
        ];
        let targets = denied
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let targets_json = serde_json::to_string(&targets).unwrap();
        let grandchild = "import sys; open(sys.argv[1], 'w').write('grandchild')";
        let child = format!(
            "import subprocess, sys; subprocess.run([sys.executable, '-c', {}, sys.argv[1]], check=False)",
            serde_json::to_string(grandchild).unwrap()
        );
        let code = format!(
            "import json, subprocess, sys\npaths=json.loads({})\nfor path in paths:\n try: open(path, 'w').write('direct')\n except OSError: pass\ntry: subprocess.run([sys.executable, '-c', {}, paths[0]], check=False)\nexcept OSError: pass",
            serde_json::to_string(&targets_json).unwrap(),
            serde_json::to_string(&child).unwrap()
        );
        let output = run_macos_seatbelt_policy_probe(
            workspace.path(),
            &spec("/usr/bin/python3", &["-c", &code], &["out"]),
        );
        assert!(output.status.success());
        for path in denied {
            assert!(
                !path.exists(),
                "Seatbelt allowed a denied write: {}",
                path.display()
            );
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_timeout_terminates_the_audited_single_process_before_late_write() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let late = workspace.path().join("out/late.txt");
        let late_literal = serde_json::to_string(&late.to_string_lossy()).unwrap();
        let code = format!("import time; time.sleep(2); open({late_literal}, 'w').write('late')");
        let mut command = spec("/usr/bin/python3", &["-c", &code], &["out"]);
        command.timeout_ms = 1_000;
        let receipt = run_project_test(workspace.path(), TestProfile::Smoke, &command).unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::TimedOut);
        assert!(receipt.timeout_descendants_terminated);
        thread::sleep(Duration::from_secs(2));
        assert!(!late.exists());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_output_capture_is_bounded_and_digests_the_full_stream() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let receipt = run_project_test(
            workspace.path(),
            TestProfile::Smoke,
            &spec(
                "/usr/bin/python3",
                &["-c", "import sys; sys.stdout.write('x' * 1048576)"],
                &[],
            ),
        )
        .unwrap();
        assert_eq!(receipt.status, TestExecutionStatus::Succeeded);
        assert!(receipt.output_truncated);
        assert_eq!(receipt.output_bytes, 1_048_576);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_external_read_only_uses_the_same_zero_write_runner() {
        let workspace = tempfile::tempdir().unwrap();
        init_git(workspace.path());
        let outside = tempfile::tempdir().unwrap();
        let workspace_write = workspace.path().join("read-only-denied");
        let outside_write = outside.path().join("read-only-denied");
        let code = format!(
            "import json, os\nfor path in json.loads({}):\n try: open(path, 'w').write('bad')\n except OSError: pass\ntry:\n pid=os.fork()\n if pid == 0: os._exit(0)\n os.waitpid(pid, 0)\n print('fork-allowed')\nexcept OSError:\n print('read-only-ok')",
            serde_json::to_string(
                &serde_json::to_string(&[
                    workspace_write.to_string_lossy(),
                    outside_write.to_string_lossy(),
                ])
                .unwrap()
            )
            .unwrap()
        );
        let output = run_read_only_command(
            workspace.path(),
            &spec("/usr/bin/python3", &["-c", &code], &[]),
        )
        .unwrap();
        assert_eq!(output.receipt.status, TestExecutionStatus::Succeeded);
        assert!(output.receipt.zero_write_preserved);
        assert!(output.receipt.observed_write_set.is_empty());
        assert_eq!(output.stdout.trim(), "read-only-ok");
        assert!(!workspace_write.exists());
        assert!(!outside_write.exists());

        std::fs::create_dir(workspace.path().join("out")).unwrap();
        let error = run_read_only_command(workspace.path(), &spec("/usr/bin/true", &[], &["out"]))
            .unwrap_err();
        assert_eq!(error.code, TestExecutionErrorCode::InvalidSpec);
        assert!(error.contains("no writable roots"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_requires_host_delegated_until_job_and_write_containment_exist() {
        match local_execution_platform_support() {
            LocalExecutionPlatformSupport::Blocked { error_code, reason } => {
                assert_eq!(error_code, TestExecutionErrorCode::SandboxUnavailable);
                assert!(reason.contains("Job Object"));
                assert!(reason.contains("filesystem-filter"));
                assert!(reason.contains("HostDelegated"));
            }
            support => panic!("unexpected Windows LocalExecution support: {support:?}"),
        }
    }

    #[test]
    fn platform_support_is_explicit_and_never_unsandboxed() {
        match local_execution_platform_support() {
            LocalExecutionPlatformSupport::Supported { backend } => match backend.as_str() {
                "macos-seatbelt" => assert_eq!(std::env::consts::OS, "macos"),
                "linux-bubblewrap" => assert_eq!(std::env::consts::OS, "linux"),
                unexpected => panic!("unexpected LocalExecution backend: {unexpected}"),
            },
            LocalExecutionPlatformSupport::Blocked { error_code, reason } => {
                assert_eq!(error_code, TestExecutionErrorCode::SandboxUnavailable);
                assert!(
                    reason.contains("blocked")
                        || reason.contains("no audited")
                        || reason.contains("unprovable")
                );
            }
        }
    }
}
