//! Contract v2 operation control plane.
//!
//! The public behavioral Interface is deliberately limited to `open`,
//! `decide`, and `apply`. Adapters authenticate a binding before `open`; they
//! never echo binding fields back through `ApplyRequest`.

#![allow(clippy::unnecessary_cast, clippy::useless_conversion)] // stat field widths differ per platform
use ags_platform::sha256;
#[cfg(unix)]
use rustix::fd::OwnedFd;
#[cfg(unix)]
use rustix::fs::{FileType, Mode, OFlags};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

pub use ags_verification::CommandSpec;

pub const CONTRACT_SCHEMA_VERSION: &str = "ags://schema/contract/v2/operation-control-plane";
const DETAILS_INLINE_LIMIT: usize = 6 * 1024;
pub const DETAILS_CHUNK_LIMIT: u32 = 3 * 1024;
#[cfg(unix)]
const MAX_SNAPSHOT_ENTRIES: usize = 4096;
#[cfg(unix)]
const MAX_SNAPSHOT_DEPTH: usize = 64;
#[cfg(unix)]
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
#[cfg(unix)]
const MAX_SNAPSHOT_NAME_BYTES: usize = 4096;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_DIRECTORY_ENTRIES: usize = 256;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_NAME_BYTES: usize = 64 * 1024;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_MEMBER_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_ROOTS: usize = 8;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_TARGETS: usize = MAX_EFFECT_OBSERVED_WRITES;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_TRAVERSAL_GUARDS: usize = 4096;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_DIRECTORY_GUARDS: usize = 256;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_TOTAL_MEMBERS: usize = 4096;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_TOTAL_NAME_BYTES: usize = 256 * 1024;
#[cfg(unix)]
const MAX_HOST_PHYSICAL_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OperationKind {
    ReadOnly,
    Transaction,
    LocalExecution,
    HostDelegated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    Blocked,
    NoChange,
    Planned,
    AwaitingOutcome,
    Applying,
    Verifying,
    Receipted,
    Recovering,
    RiskEscalated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Succeeded,
    Failed,
    Recovered,
    RiskEscalated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct OperationContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSurface {
    Cli,
    Mcp,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestProfile {
    Smoke,
    Standard,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckScope {
    Governance,
    Changes,
    Evidence,
    Release,
    Promotion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorScope {
    Workspace,
    Runtime,
    Host,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMode {
    Reconcile,
    RemoveOwned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MigrationMode {
    None,
    ExactOwnedOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    #[serde(default)]
    pub context: OperationContext,
    #[serde(default)]
    pub approved_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub migration: MigrationMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRegisterRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub surface: AgentSurface,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentProbeRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub surface: AgentSurface,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostProjectionRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub mode: ProjectionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityInventoryRequest {
    #[serde(default)]
    pub context: OperationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSource {
    pub uri: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillInstallRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub skill_id: String,
    pub source: SkillSourceSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_metadata: Option<String>,
    pub target_hosts: Vec<String>,
    pub update_policy: SkillUpdatePolicy,
    #[serde(default)]
    pub risk_acknowledgements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillSourceSpec {
    pub kind: SkillSourceKind,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Local,
    GitHub,
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillUpdatePolicy {
    Notify,
    Manual,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillRemoveRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub skill_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySnapshotRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpAdviceRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub mcp_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskValidateRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub task_card_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskPlanRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub task_card_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCloseRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub task_card_path: String,
    pub launch_plan_path: String,
    pub delivery_report_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub task_card_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GateRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub task_card_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceArtifactKind {
    LaunchPlan,
    DeliveryReport,
    Receipt,
    TestReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub artifact_kind: EvidenceArtifactKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_card_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_plan_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryCloseRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub receipt_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoctorRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub scope: DoctorScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub scope: CheckScope,
    /// Explicit public worktree used by promotion (and release when
    /// present) verification scopes. Absent for ordinary workspace checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub profile: TestProfile,
    #[serde(default)]
    pub executor: TestExecutor,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TestExecutor {
    #[default]
    Host,
    Local,
}

impl TestExecutor {
    const fn operation_kind(self) -> OperationKind {
        match self {
            Self::Host => OperationKind::HostDelegated,
            Self::Local => OperationKind::LocalExecution,
        }
    }
}

#[doc(hidden)]
pub const fn fixed_operation_kind<T>(_request: &T, declared: OperationKind) -> OperationKind {
    declared
}

#[doc(hidden)]
pub const fn test_operation_kind(request: &TestRequest, _declared: OperationKind) -> OperationKind {
    request.executor.operation_kind()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaRequest {
    #[serde(default)]
    pub context: OperationContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSessionStartRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub host_session_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSessionEndRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub host_session_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleStopGuardRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub host_id: String,
    pub host_session_id: String,
    pub event_id: String,
    pub last_assistant_message: String,
}

/// Authenticated, digest-bound access to one immutable oversized result.
///
/// The URI is only an opaque lookup handle. Authorization comes from the
/// `OpenedSession` passed to `decide`, and the caller must repeat the digest
/// returned in the original bounded result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailsReadRequest {
    #[serde(default)]
    pub context: OperationContext,
    pub artifact: ContentAddressedArtifactRef,
    #[serde(default)]
    pub offset: u64,
    pub max_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailsChunk {
    pub artifact: ContentAddressedArtifactRef,
    pub offset: u64,
    pub next_offset: u64,
    pub byte_length: u64,
    pub eof: bool,
    /// Hex avoids a second binary dependency while keeping the JSON chunk
    /// bounded independently of arbitrary UTF-8 boundaries.
    pub encoding: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DetailsReference {
    pub details_uri: String,
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterSurface {
    ProductCli,
    HostLifecycle,
    ControlPlaneInternal,
}

enum ControlPlaneDispatch<'a> {
    External,
    DetailsRead(&'a DetailsReadRequest),
}

macro_rules! control_plane_surface_dispatch {
    (ControlPlaneInternal, $request:expr) => {
        ControlPlaneDispatch::DetailsRead($request)
    };
    ($surface:ident, $request:expr) => {{
        let _ = $request;
        ControlPlaneDispatch::External
    }};
}

macro_rules! operation_registry {
    ($( $variant:ident($request:ty) => $wire:literal, $cli:literal, $surface:ident, $resolver:path, [$primary:ident $(, $allowed:ident)*], $schema:literal, $summary:literal; )+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum OperationName {
            $(#[serde(rename = $wire)] $variant,)+
        }

        impl OperationName {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire,)+ }
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(tag = "operation", content = "request", deny_unknown_fields)]
        pub enum OperationRequest {
            $(#[serde(rename = $wire)] $variant($request),)+
        }

        impl OperationRequest {
            pub const fn name(&self) -> OperationName {
                match self { $(Self::$variant(_) => OperationName::$variant,)+ }
            }

            pub const fn kind(&self) -> OperationKind {
                match self {
                    $(Self::$variant(request) => $resolver(request, OperationKind::$primary),)+
                }
            }

            pub fn context(&self) -> &OperationContext {
                match self { $(Self::$variant(request) => &request.context,)+ }
            }

            pub fn context_mut(&mut self) -> &mut OperationContext {
                match self { $(Self::$variant(request) => &mut request.context,)+ }
            }

            pub fn spec(&self) -> &'static OperationSpec {
                let name = self.name();
                operation_registry().iter().find(|spec| spec.name == name)
                    .expect("every typed request is declared in operation_registry!")
            }

            fn dispatch_control_plane(&self) -> ControlPlaneDispatch<'_> {
                match self {
                    $(Self::$variant(request) => control_plane_surface_dispatch!($surface, request),)+
                }
            }
        }

        static OPERATION_SPECS: &[OperationSpec] = &[
            $(OperationSpec {
                name: OperationName::$variant,
                cli_path: $cli,
                adapter_surface: AdapterSurface::$surface,
                kind: OperationKind::$primary,
                allowed_kinds: &[OperationKind::$primary, $(OperationKind::$allowed,)*],
                request_schema_id: $schema,
                summary: $summary,
                request_schema: schema_for::<$request>,
            },)+
        ];
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct OperationSpec {
    pub name: OperationName,
    pub cli_path: &'static str,
    pub adapter_surface: AdapterSurface,
    pub kind: OperationKind,
    pub allowed_kinds: &'static [OperationKind],
    pub request_schema_id: &'static str,
    pub summary: &'static str,
    #[serde(skip_serializing)]
    pub request_schema: fn() -> serde_json::Value,
}

/// Single callback-style operation declaration consumed by core, CLI and MCP.
/// Consumers must filter by the declared adapter surface; in particular,
/// `ControlPlaneInternal` entries must never become product CLI help.
#[macro_export]
macro_rules! for_each_operation {
    ($consumer:ident) => {
        $consumer! {
    Setup($crate::SetupRequest) => "setup", "setup", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/setup-request", "Initialize the machine-owned AGS runtime";
    Init($crate::InitRequest) => "init", "init", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/init-request", "Attach one explicit workspace";
    AgentRegister($crate::AgentRegisterRequest) => "agent.register", "agent register", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/agent-register-request", "Register AGS-owned metadata for one generic host";
    AgentProbe($crate::AgentProbeRequest) => "agent.probe", "agent probe", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/agent-probe-request", "Probe a generic host without mutation";
    GovernHostProjection($crate::HostProjectionRequest) => "govern.host_projection", "govern host-projection", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/host-projection-request", "Reconcile AGS-owned host projection";
    GovernCapabilityInventory($crate::CapabilityInventoryRequest) => "govern.capability.inventory", "govern capability inventory", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/capability-inventory-request", "Read typed capability inventory";
    GovernSkillInstall($crate::SkillInstallRequest) => "govern.skill.install", "govern skill install", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/skill-install-request", "Install an integrity-bound Skill";
    GovernSkillRemove($crate::SkillRemoveRequest) => "govern.skill.remove", "govern skill remove", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/skill-remove-request", "Remove an exact installed Skill";
    GovernCapabilitySnapshot($crate::CapabilitySnapshotRequest) => "govern.capability.snapshot", "govern capability snapshot", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/capability-snapshot-request", "Write an exact host capability snapshot";
    GovernMcpAdvice($crate::McpAdviceRequest) => "govern.mcp.advice", "govern mcp advice", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/mcp-advice-request", "Return advice-only third-party MCP registration guidance";
    GovernTaskValidate($crate::TaskValidateRequest) => "govern.task.validate", "govern task validate", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/task-validate-request", "Validate a canonical task card";
    GovernTaskPlan($crate::TaskPlanRequest) => "govern.task.plan", "govern task plan", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/task-plan-request", "Prepare a sealed LaunchPlan without executing a host";
    GovernTaskClose($crate::TaskCloseRequest) => "govern.task.close", "govern task close", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/task-close-request", "Bind task, plan, delivery, receipt and closure memory";
    GovernPolicy($crate::PolicyRequest) => "govern.policy", "govern policy", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/policy-request", "Resolve execution policy without mutation";
    GovernGate($crate::GateRequest) => "govern.gate", "govern gate", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/gate-request", "Evaluate the fixed M1-M10 gate";
    GovernEvidence($crate::EvidenceRequest) => "govern.evidence", "govern evidence", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/evidence-request", "Verify a typed evidence artifact";
    GovernMemoryClose($crate::MemoryCloseRequest) => "govern.memory.close", "govern memory", ProductCli, $crate::fixed_operation_kind, [Transaction], "ags://schema/contract/v2/memory-close-request", "Persist a verified closure pointer";
    Update($crate::UpdateRequest) => "update", "update", ProductCli, $crate::fixed_operation_kind, [HostDelegated], "ags://schema/contract/v2/update-request", "Execute a verified runtime update through a bound host outcome";
    Doctor($crate::DoctorRequest) => "doctor", "doctor", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/doctor-request", "Inspect runtime and workspace health";
    Check($crate::CheckRequest) => "check", "check", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/check-request", "Run governance checks without project tests";
    Test($crate::TestRequest) => "test", "test", ProductCli, $crate::test_operation_kind, [HostDelegated, LocalExecution], "ags://schema/contract/v2/test-request", "Execute one structured project test command";
    Schema($crate::SchemaRequest) => "schema", "schema", ProductCli, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/schema-request", "Read contract v2 schema metadata";
    HostLifecycleSessionStart($crate::LifecycleSessionStartRequest) => "host.lifecycle.session_start", "host lifecycle session-start", HostLifecycle, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/lifecycle-session-start-request", "Read bounded startup context for one authenticated host session";
    HostLifecycleSessionEnd($crate::LifecycleSessionEndRequest) => "host.lifecycle.session_end", "host lifecycle session-end", HostLifecycle, $crate::fixed_operation_kind, [HostDelegated], "ags://schema/contract/v2/lifecycle-session-end-request", "Archive verified closures through a sealed delegated outcome";
    HostLifecycleStopGuard($crate::LifecycleStopGuardRequest) => "host.lifecycle.stop_guard", "host lifecycle stop-guard", HostLifecycle, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/lifecycle-stop-guard-request", "Check one typed assistant message for raw tool-call leakage";
    DetailsRead($crate::DetailsReadRequest) => "details.read", "", ControlPlaneInternal, $crate::fixed_operation_kind, [ReadOnly], "ags://schema/contract/v2/details-read-request", "Read one authenticated digest-verified chunk of an immutable oversized result";
        }
    };
}

for_each_operation!(operation_registry);

pub(crate) mod platform_io;
#[cfg(unix)]
use platform_io::unix::{read_regular_fd, StableReadError};
#[cfg(not(unix))]
use platform_io::{
    descriptor_host_artifact_is_directory, descriptor_read_host_artifact,
    host_physical_seal_digest, seal_host_physical_state, tree_digest, verify_host_physical_delta,
    HostPhysicalSeal,
};

#[cfg(unix)]
mod production;
#[cfg(not(unix))]
#[path = "control_plane/production_non_unix.rs"]
mod production;
pub(crate) use production::ProductionEffectAdapter;

pub fn operation_registry() -> &'static [OperationSpec] {
    OPERATION_SPECS
}

/// A filtered view over the single canonical registry. Consumers select their
/// declared adapter surface instead of re-declaring operation metadata.
pub fn operation_registry_for_surface(surface: AdapterSurface) -> Vec<&'static OperationSpec> {
    operation_registry()
        .iter()
        .filter(|spec| spec.adapter_surface == surface)
        .collect()
}

pub(crate) fn schema_read_result(
    request: &SchemaRequest,
) -> Result<serde_json::Value, ControlPlaneError> {
    if let Some(name) = request.operation.as_deref() {
        let spec = operation_registry()
            .iter()
            .find(|spec| {
                spec.adapter_surface == AdapterSurface::ProductCli && spec.name.as_str() == name
            })
            .ok_or_else(|| ControlPlaneError {
                code: "schema_operation_unknown",
                detail: name.to_string(),
            })?;
        Ok(serde_json::json!({
            "contract": CONTRACT_SCHEMA_VERSION,
            "operation": spec,
            "request_schema": (spec.request_schema)(),
        }))
    } else {
        Ok(serde_json::json!({
            "contract": CONTRACT_SCHEMA_VERSION,
            "operations": operation_registry_for_surface(AdapterSurface::ProductCli),
            "request_schema": operation_request_schema(),
        }))
    }
}

fn schema_for<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("typed operation schema serializes")
}

/// Contract-v2 Product CLI tagged union used directly by MCP tools/list and
/// product JSON input. Host lifecycle and internal details operations are
/// constructed from the same registry through their own surface selectors.
pub fn operation_request_schema() -> serde_json::Value {
    operation_request_schema_for_surface(AdapterSurface::ProductCli)
}

pub fn host_lifecycle_operation_request_schema() -> serde_json::Value {
    operation_request_schema_for_surface(AdapterSurface::HostLifecycle)
}

fn operation_request_schema_for_surface(surface: AdapterSurface) -> serde_json::Value {
    let mut definitions = serde_json::Map::new();
    let mut branches = Vec::new();
    for (index, spec) in operation_registry_for_surface(surface)
        .into_iter()
        .enumerate()
    {
        let mut request = (spec.request_schema)();
        let mut replacements = std::collections::BTreeMap::new();
        if let Some(local) = request
            .as_object_mut()
            .and_then(|root| root.remove("$defs"))
            .and_then(|value| value.as_object().cloned())
        {
            for name in local.keys() {
                replacements.insert(name.clone(), format!("s{index}_{name}"));
            }
            for (name, mut definition) in local {
                replace_definition_refs(&mut definition, &replacements);
                definitions.insert(replacements[&name].clone(), definition);
            }
        }
        replace_definition_refs(&mut request, &replacements);
        branches.push(serde_json::json!({
            "type": "object",
            "required": ["operation", "request"],
            "additionalProperties": false,
            "properties": {
                "operation": { "const": spec.name.as_str() },
                "request": request,
            }
        }));
    }
    let mut schema = serde_json::json!({
        "type": "object",
        "required": ["operation", "request"],
        "unevaluatedProperties": false,
        "oneOf": branches,
        "$defs": definitions,
    });
    compact_mcp_schema(&mut schema);
    deduplicate_definitions(&mut schema);
    compact_definition_names(&mut schema);
    if let Some(root) = schema.as_object_mut() {
        root.insert("type".to_string(), serde_json::json!("object"));
        root.insert(
            "required".to_string(),
            serde_json::json!(["operation", "request"]),
        );
        root.insert(
            "unevaluatedProperties".to_string(),
            serde_json::json!(false),
        );
        if let Some(branches) = root
            .get_mut("oneOf")
            .and_then(serde_json::Value::as_array_mut)
        {
            for branch in branches {
                if let Some(object) = branch.as_object_mut() {
                    object.remove("type");
                    object.remove("required");
                    object.remove("additionalProperties");
                }
            }
        }
    }
    schema
}

/// Typed host-outcome artifact reference exposed by CLI JSON and MCP apply.
/// The production daemon currently fails closed before consuming it until the
/// referenced receipt can be descriptor-read and binding-verified.
pub fn host_outcome_input_schema() -> serde_json::Value {
    let mut schema = schema_for::<HostOutcomeInput>();
    compact_mcp_schema(&mut schema);
    compact_definition_names(&mut schema);
    schema
}

fn compact_definition_names(schema: &mut serde_json::Value) {
    let Some(definitions) = schema
        .as_object_mut()
        .and_then(|object| object.get_mut("$defs"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let old = std::mem::take(definitions);
    let names = old.keys().cloned().collect::<Vec<_>>();
    let replacements = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            const COMPACT_NAMES: &[u8] =
                b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let compact = COMPACT_NAMES
                .get(index)
                .copied()
                .map(char::from)
                .unwrap_or_else(|| panic!("contract schema has too many definitions"));
            (name.clone(), compact.to_string())
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    *definitions = old
        .into_iter()
        .map(|(name, value)| (replacements[&name].clone(), value))
        .collect();
    replace_definition_refs(schema, &replacements);
}

fn deduplicate_definitions(schema: &mut serde_json::Value) {
    let Some(definitions) = schema
        .as_object_mut()
        .and_then(|object| object.get_mut("$defs"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let old = std::mem::take(definitions);
    let mut canonical_by_value = std::collections::BTreeMap::<String, String>::new();
    let mut replacements = std::collections::BTreeMap::new();
    for (name, value) in old {
        let fingerprint = serde_json::to_string(&value).expect("schema definition serializes");
        if let Some(canonical) = canonical_by_value.get(&fingerprint) {
            replacements.insert(name, canonical.clone());
        } else {
            canonical_by_value.insert(fingerprint, name.clone());
            definitions.insert(name, value);
        }
    }
    replace_definition_refs(schema, &replacements);
}

fn replace_definition_refs(
    value: &mut serde_json::Value,
    replacements: &std::collections::BTreeMap<String, String>,
) {
    match value {
        serde_json::Value::Object(object) => {
            let replacement = object
                .get("$ref")
                .and_then(serde_json::Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .and_then(|name| replacements.get(name))
                .map(|name| format!("#/$defs/{name}"));
            if let Some(reference) = replacement {
                object.insert("$ref".to_string(), serde_json::Value::String(reference));
            }
            for child in object.values_mut() {
                replace_definition_refs(child, replacements);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                replace_definition_refs(child, replacements);
            }
        }
        _ => {}
    }
}

fn compact_mcp_schema(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("$schema");
            object.remove("title");
            object.remove("description");
            object.remove("default");
            if object.contains_key("const") || object.contains_key("enum") {
                object.remove("type");
            }
            for child in object.values_mut() {
                compact_mcp_schema(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                compact_mcp_schema(child);
            }
        }
        _ => {}
    }
}

pub fn operation_schema(name: OperationName) -> serde_json::Value {
    let spec = operation_registry()
        .iter()
        .find(|spec| spec.name == name)
        .expect("operation name comes from the registry");
    (spec.request_schema)()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BindingSurface {
    Mcp {
        connection_id: String,
        authenticated_session: String,
    },
    Cli {
        workspace_service_identity: String,
    },
}

/// Binding facts produced by an authenticated adapter. Apply never accepts
/// caller-echoed host, connection, workspace, or session fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedBinding {
    host_id: String,
    canonical_workspace: PathBuf,
    workspace_identity: String,
    project_facts_hash: String,
    registry_key: String,
    authorized_write_roots: Vec<PathBuf>,
    surface: BindingSurface,
}

impl AuthenticatedBinding {
    // Keep every transport-authenticated security fact explicit at the call
    // site; collapsing these into optional bags makes binding omissions hard
    // to review.
    #[allow(clippy::too_many_arguments)]
    pub fn mcp(
        connection_id: impl Into<String>,
        host_id: impl Into<String>,
        canonical_workspace: impl Into<PathBuf>,
        workspace_identity: impl Into<String>,
        project_facts_hash: impl Into<String>,
        registry_key: impl Into<String>,
        authenticated_session: impl Into<String>,
        authorized_write_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            host_id: host_id.into(),
            canonical_workspace: canonical_workspace.into(),
            workspace_identity: workspace_identity.into(),
            project_facts_hash: project_facts_hash.into(),
            registry_key: registry_key.into(),
            authorized_write_roots,
            surface: BindingSurface::Mcp {
                connection_id: connection_id.into(),
                authenticated_session: authenticated_session.into(),
            },
        }
    }

    pub fn cli(
        host_id: impl Into<String>,
        canonical_workspace: impl Into<PathBuf>,
        workspace_identity: impl Into<String>,
        project_facts_hash: impl Into<String>,
        registry_key: impl Into<String>,
        workspace_service_identity: impl Into<String>,
        authorized_write_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            host_id: host_id.into(),
            canonical_workspace: canonical_workspace.into(),
            workspace_identity: workspace_identity.into(),
            project_facts_hash: project_facts_hash.into(),
            registry_key: registry_key.into(),
            authorized_write_roots,
            surface: BindingSurface::Cli {
                workspace_service_identity: workspace_service_identity.into(),
            },
        }
    }

    pub fn host_id(&self) -> &str {
        &self.host_id
    }
    pub fn canonical_workspace(&self) -> &Path {
        &self.canonical_workspace
    }
    pub fn workspace_identity(&self) -> &str {
        &self.workspace_identity
    }
    pub fn project_facts_hash(&self) -> &str {
        &self.project_facts_hash
    }
    pub fn registry_key(&self) -> &str {
        &self.registry_key
    }
    pub fn authorized_write_roots(&self) -> &[PathBuf] {
        &self.authorized_write_roots
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        let surface = match &self.surface {
            BindingSurface::Mcp {
                connection_id,
                authenticated_session,
            } => {
                format!("mcp\n{connection_id}\n{authenticated_session}")
            }
            BindingSurface::Cli {
                workspace_service_identity,
            } => {
                format!("cli\n{workspace_service_identity}")
            }
        };
        let mut write_roots = self
            .authorized_write_roots
            .iter()
            .map(root_seal)
            .collect::<Vec<_>>();
        write_roots.sort();
        let write_roots = write_roots.join("\n");
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.host_id,
            self.canonical_workspace.display(),
            self.workspace_identity,
            self.project_facts_hash,
            self.registry_key,
            surface,
            write_roots
        )
        .into_bytes()
    }

    #[cfg(test)]
    fn semantic_bytes(&self) -> Vec<u8> {
        self.canonical_bytes()
    }
}

#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub binding: AuthenticatedBinding,
    pub policy_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenedSession {
    session_ref: String,
    seal: String,
    pending_recovery_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_recovery_action_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_receipt: Option<OperationReceipt>,
    terminal_recovery_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_recovery_digest: Option<String>,
}

impl OpenedSession {
    pub fn session_ref(&self) -> &str {
        &self.session_ref
    }

    pub fn pending_recovery_action_ref(&self) -> Option<&str> {
        self.pending_recovery_action_ref.as_deref()
    }

    pub fn recovery_receipt(&self) -> Option<&OperationReceipt> {
        self.recovery_receipt.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanStep {
    pub step_id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VerificationSpec {
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Recoverability {
    NotApplicable,
    BeforeEffectOnly,
    Transactional,
    SourcePreserving,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainPlan {
    /// Digest of the complete opaque domain action (including bytes and
    /// before-state) so the public plan seal binds more than display steps.
    pub action_digest: String,
    pub steps: Vec<PlanStep>,
    pub expected_write_paths: Vec<String>,
    pub verification: VerificationSpec,
    pub recoverability: Recoverability,
    pub execution: Option<CommandSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDomain<A> {
    pub plan: DomainPlan,
    pub action: A,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
pub enum PlanDisposition<A> {
    NoChange { output_digest: String },
    Planned(Box<PlannedDomain<A>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedPlan {
    pub schema_version: String,
    pub plan_hash: String,
    pub operation: OperationName,
    pub kind: OperationKind,
    pub binding_hash: String,
    pub policy_hash: String,
    pub payload_hash: String,
    pub action_digest: String,
    pub steps: Vec<PlanStep>,
    pub expected_write_paths: Vec<String>,
    pub verification: VerificationSpec,
    pub recoverability: Recoverability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<CommandSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OperationReceipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub operation: OperationName,
    pub status: ReceiptStatus,
    pub plan_hash: String,
    pub payload_hash: String,
    pub binding_hash: String,
    pub output_digest: String,
    pub observed_write_set: Vec<String>,
    pub recovered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub state: OperationState,
    pub kind: OperationKind,
    /// Bounded, operation-typed read result. Large evidence remains external
    /// and is referenced through a verified URI; adapters never reconstruct
    /// domain output from receipt hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<SealedPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<OperationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostOutcomeStatus {
    Succeeded,
    Failed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentAddressedArtifactRef {
    pub uri: String,
    pub sha256: String,
}

/// Host-provided evidence accepted at the adapter boundary. The receipt is the
/// only fact source; status, write set, binding, plan, and optional details are
/// read from and verified against that content-addressed artifact by the
/// workspace daemon. The legacy loose-field outcome shape is intentionally not
/// accepted by contract v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostOutcomeInput {
    pub receipt: ContentAddressedArtifactRef,
}

pub const HOST_OUTCOME_SCHEMA_VERSION: &str = "ags://schema/contract/v2/authenticated-host-outcome";
pub const HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION: &str =
    "ags://schema/contract/v2/host-execution-instruction";
const MAX_ACTIVE_SESSIONS: usize = 64;
const MAX_ACTIVE_ACTIONS: usize = 256;
const MAX_DETAILS_RECORDS: usize = 128;
const MAX_DETAILS_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_DETAILS_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOST_POSTIMAGE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HOST_POSTIMAGE_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_HOST_EVIDENCE_BYTES: usize = 1024 * 1024;
const MAX_EFFECT_OBSERVED_WRITES: usize = 512;
const MAX_EFFECT_PATH_BYTES: usize = 4096;
const MAX_EFFECT_TOTAL_PATH_BYTES: usize = 512 * 1024;
const MAX_EFFECT_EVIDENCE_BYTES: usize = 1024 * 1024;

/// One content-addressed member of a sealed runtime release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostReleaseMember {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    pub mode: u32,
}

/// The complete, closed set of effects a host may execute for a
/// `HostDelegated` operation. This intentionally has no prose, shell string,
/// or generic mutation escape hatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostExecutionAction {
    Command {
        profile: TestProfile,
        program: String,
        argv: Vec<String>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
        timeout_ms: u64,
        allowed_write_paths: Vec<PathBuf>,
    },
    RuntimeUpdate {
        channel: String,
        target_version: Option<String>,
        candidate_directory: PathBuf,
        release_directory: PathBuf,
        manifest: HostReleaseMember,
        tree_digest: String,
        members: Vec<HostReleaseMember>,
        expected_write_paths: Vec<PathBuf>,
    },
    ArchiveClosures {
        event_id: String,
        receipt_ids: Vec<String>,
        pointer_paths: Vec<PathBuf>,
        expected_write_paths: Vec<PathBuf>,
    },
}

/// Canonical host work issued by the control plane before any host effect.
/// `instruction_digest` is computed over every other field plus the sealed
/// domain `action_digest`, so it cannot be transplanted to a different action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostExecutionInstruction {
    pub schema_version: String,
    pub action_ref: String,
    pub binding_hash: String,
    pub plan_hash: String,
    pub policy_hash: String,
    pub instruction_digest: String,
    pub action: HostExecutionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostArtifactState {
    Present { sha256: String },
    Directory,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HostWriteArtifact {
    pub path: String,
    #[serde(flatten)]
    pub state: HostArtifactState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostOutcomeReceipt {
    pub schema_version: String,
    pub action_ref: String,
    pub binding_hash: String,
    pub plan_hash: String,
    pub policy_hash: String,
    pub instruction_digest: String,
    pub outcome_token: String,
    pub generation: u64,
    pub status: HostOutcomeStatus,
    pub output_digest: String,
    #[serde(default)]
    pub observed_write_set: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<HostWriteArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<HostOutcomeEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostEvidenceKind {
    TestReceipt,
    LifecycleReceipt,
    UpdateReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateReceipt {
    pub schema_version: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    pub action_ref: String,
    pub binding_hash: String,
    pub plan_hash: String,
    pub observed_write_set: Vec<String>,
    pub release_manifest_sha256: String,
    pub release_tree_digest: String,
    pub output_digest: String,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HostOutcomeEvidence {
    pub kind: HostEvidenceKind,
    pub artifact: ContentAddressedArtifactRef,
    pub content_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedHostEvidence {
    kind: HostEvidenceKind,
    artifact: ContentAddressedArtifactRef,
    bytes: Vec<u8>,
}

#[cfg_attr(not(unix), allow(dead_code))]
impl VerifiedHostEvidence {
    pub fn kind(&self) -> HostEvidenceKind {
        self.kind
    }

    pub fn artifact(&self) -> &ContentAddressedArtifactRef {
        &self.artifact
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedHostOutcome {
    binding: AuthenticatedBinding,
    artifact: ContentAddressedArtifactRef,
    bytes: Vec<u8>,
}

impl AuthenticatedHostOutcome {
    /// The daemon constructs this only after descriptor-safe retrieval of the
    /// content-addressed input. Core repeats digest, schema and binding checks.
    pub fn from_artifact(
        binding: AuthenticatedBinding,
        artifact: ContentAddressedArtifactRef,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            binding,
            artifact,
            bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    pub action_ref: String,
    pub outcome: Option<AuthenticatedHostOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyResult {
    pub state: OperationState,
    pub transitions: Vec<OperationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<OperationReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub details: Option<DetailsReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_deadline_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadObservation {
    pub result: serde_json::Value,
    pub output_digest: String,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectObservation {
    succeeded: bool,
    effect_started: bool,
    output_digest: String,
    observed_write_set: Vec<String>,
    evidence: Option<serde_json::Value>,
    contract_violation: Option<String>,
}

impl EffectObservation {
    pub fn bounded(
        succeeded: bool,
        effect_started: bool,
        output_digest: String,
        observed_write_set: Vec<String>,
        evidence: Option<serde_json::Value>,
    ) -> Result<Self, EffectError> {
        if observed_write_set.len() > MAX_EFFECT_OBSERVED_WRITES
            || observed_write_set
                .iter()
                .any(|path| path.len() > MAX_EFFECT_PATH_BYTES)
            || observed_write_set.iter().map(String::len).sum::<usize>()
                > MAX_EFFECT_TOTAL_PATH_BYTES
        {
            return Err(EffectError {
                code: "effect_observation_write_set_too_large".to_string(),
                detail: "adapter observation exceeds the sealed write-set budget".to_string(),
                effect_started,
                output_digest,
                observed_write_set,
            });
        }
        if evidence
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| EffectError {
                code: "effect_observation_evidence_invalid".to_string(),
                detail: error.to_string(),
                effect_started,
                output_digest: output_digest.clone(),
                observed_write_set: observed_write_set.clone(),
            })?
            .is_some_and(|bytes| bytes.len() > MAX_EFFECT_EVIDENCE_BYTES)
        {
            return Err(EffectError {
                code: "effect_observation_evidence_too_large".to_string(),
                detail: "adapter evidence exceeds the terminal artifact budget".to_string(),
                effect_started,
                output_digest,
                observed_write_set,
            });
        }
        Ok(Self {
            succeeded,
            effect_started,
            output_digest,
            observed_write_set,
            evidence,
            contract_violation: None,
        })
    }

    fn contract_violation(error: EffectError) -> Self {
        let original_count = error.observed_write_set.len();
        let original_path_digest = sha256(error.observed_write_set.join("\n"));
        let mut observed_write_set = error
            .observed_write_set
            .into_iter()
            .take(MAX_EFFECT_OBSERVED_WRITES.saturating_sub(1))
            .map(|entry| {
                if entry.len() <= MAX_EFFECT_PATH_BYTES {
                    entry
                } else {
                    format!("ags://truncated-path/{}", sha256(entry))
                }
            })
            .collect::<Vec<_>>();
        observed_write_set.push(format!(
            "ags://contract-violation/{}",
            error.code.replace('_', "-")
        ));
        Self {
            succeeded: false,
            effect_started: error.effect_started,
            output_digest: error.output_digest,
            observed_write_set,
            evidence: Some(serde_json::json!({
                "contract_violation": error.code,
                "detail": error.detail,
                "truncated": true,
                "original_write_count": original_count,
                "original_write_set_digest": original_path_digest,
            })),
            contract_violation: Some("effect_observation_contract_violation".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationObservation {
    pub passed: bool,
    pub output_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryObservation {
    pub succeeded: bool,
    pub output_digest: String,
    pub observed_write_set: Vec<String>,
    pub evidence: Option<serde_json::Value>,
    pub original_journal_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingRecovery<A> {
    operation: OperationName,
    journal_identity_digest: String,
    journal_state_digest: String,
    expected_write_paths: Vec<String>,
    action: A,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingInspection<A> {
    active: Option<PendingRecovery<A>>,
    terminal_receipts: Vec<OperationReceipt>,
}

impl<A> Default for PendingInspection<A> {
    fn default() -> Self {
        Self {
            active: None,
            terminal_receipts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectError {
    pub code: String,
    pub detail: String,
    pub effect_started: bool,
    pub output_digest: String,
    pub observed_write_set: Vec<String>,
}

fn control_plane_effect_error(error: EffectError, fallback: &'static str) -> ControlPlaneError {
    ControlPlaneError {
        code: if error.code == platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE {
            platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE
        } else if error.code == "transaction_journal_write_set_mismatch" {
            "transaction_journal_write_set_mismatch"
        } else {
            fallback
        },
        detail: format!("{}: {}", error.code, error.detail),
    }
}

/// Production domain engines implement this internal seam. There is
/// intentionally no fake/default authority: callers must supply real handlers.
pub(crate) trait EffectAdapter {
    type Action: Clone;
    /// Reject an operation before roots are scanned, a domain handler runs, or
    /// an action can be retained when the target cannot prove its invariants.
    fn validate_platform_support(&self, _operation: &OperationRequest) -> Result<(), EffectError>;
    /// Planning must not mutate governed target/runtime state. Implementations
    /// may perform bounded read-only discovery or acquire a remote candidate
    /// into disposable scratch; the returned sealed plan remains the only
    /// authority for later governed effects.
    fn plan(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<PlanDisposition<Self::Action>, EffectError>;
    /// Complete roots whose byte-level tree state must remain unchanged for a
    /// ReadOnly operation (normally workspace plus machine runtime state).
    fn read_only_roots(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Vec<PathBuf>;
    fn read(
        &self,
        operation: &OperationRequest,
        binding: &AuthenticatedBinding,
    ) -> Result<ReadObservation, EffectError>;
    /// Recompute the adapter action footprint and compare it with the sealed
    /// public plan before crossing the mutation boundary.
    fn validate_sealed_action(
        &self,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
    ) -> Result<(), EffectError> {
        Ok(())
    }
    fn semantic_action_digest(
        &self,
        _action: &Self::Action,
    ) -> Result<Option<String>, EffectError> {
        Ok(None)
    }
    fn seals_host_physical_state(
        &self,
        _operation: &OperationRequest,
        _action: &Self::Action,
    ) -> bool {
        false
    }
    fn is_recovery_action(&self, _action: &Self::Action) -> bool {
        false
    }
    fn verify_host_outcome(
        &self,
        _operation: &OperationRequest,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
        _receipt: &HostOutcomeReceipt,
        _evidence: Option<&VerifiedHostEvidence>,
    ) -> Result<(), EffectError> {
        Err(EffectError {
            code: "host_outcome_verifier_unavailable".to_string(),
            detail: "adapter has no typed verifier for this HostDelegated operation".to_string(),
            effect_started: false,
            output_digest: String::new(),
            observed_write_set: Vec::new(),
        })
    }
    /// Pure typed projection for HostDelegated work whose sealed plan does not
    /// carry a `CommandSpec`. Implementations must derive the variant only
    /// from the already-sealed domain action and plan; no host effect is
    /// permitted while constructing an instruction.
    fn host_execution_action(
        &self,
        _operation: &OperationRequest,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
    ) -> Result<HostExecutionAction, EffectError> {
        Err(EffectError {
            code: "host_execution_instruction_unavailable".to_string(),
            detail: "adapter has no closed typed action for this HostDelegated operation"
                .to_string(),
            effect_started: false,
            output_digest: String::new(),
            observed_write_set: Vec::new(),
        })
    }
    fn apply(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        operation: Option<&OperationRequest>,
        binding: &AuthenticatedBinding,
    ) -> Result<EffectObservation, EffectError>;
    fn verify(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        observation: &EffectObservation,
    ) -> Result<VerificationObservation, EffectError>;
    fn recover(
        &mut self,
        action_ref: &str,
        plan: &SealedPlan,
        action: &Self::Action,
        observation: &EffectObservation,
    ) -> Result<RecoveryObservation, EffectError>;
    /// Pure inspection only. The returned internal action is sealed by open;
    /// no recovery write may occur before its explicit apply.
    fn inspect_pending(
        &self,
        _binding: &AuthenticatedBinding,
    ) -> Result<PendingInspection<Self::Action>, EffectError> {
        Ok(PendingInspection::default())
    }
    fn finalize_recovery(
        &mut self,
        _action_ref: &str,
        _plan: &SealedPlan,
        _action: &Self::Action,
        _binding: &AuthenticatedBinding,
        _receipt: &OperationReceipt,
    ) -> Result<(), EffectError> {
        Err(EffectError {
            code: "recovery_finalizer_unavailable".to_string(),
            detail: "adapter has no durable recovery terminal writer".to_string(),
            effect_started: false,
            output_digest: String::new(),
            observed_write_set: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneError {
    pub code: &'static str,
    pub detail: String,
}

impl fmt::Display for ControlPlaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl std::error::Error for ControlPlaneError {}

#[derive(Clone)]
struct SessionRecord {
    binding: AuthenticatedBinding,
    binding_hash: String,
    policy_hash: String,
    seal: String,
    generation_key: String,
    generation: u64,
    pending_recovery_action_ref: Option<String>,
}

#[derive(Clone)]
struct ActionRecord<A> {
    session_ref: String,
    plan: SealedPlan,
    operation: ActionOperation,
    domain_action: A,
    host_physical_seal: Option<HostPhysicalSeal>,
    consumed: bool,
    nonce: u64,
}

#[derive(Clone)]
enum ActionOperation {
    External(Box<OperationRequest>),
    Recovery {
        original_operation: OperationName,
        journal_identity_digest: String,
        journal_state_digest: String,
    },
}

#[derive(Clone)]
#[cfg(unix)]
struct HostPhysicalSeal {
    roots: Vec<SealedHostRoot>,
    traversal_guards: Vec<HostTraversalGuard>,
    directory_guards: Vec<HostDirectoryGuard>,
    targets: Vec<SealedHostTarget>,
}

#[derive(Clone)]
#[cfg(unix)]
struct SealedHostRoot {
    path: PathBuf,
    descriptor: std::sync::Arc<OwnedFd>,
    device: u64,
    inode: u64,
    mode: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg(unix)]
struct SealedHostTarget {
    path: String,
    root_index: usize,
    relative: PathBuf,
    before: HostTargetState,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg(unix)]
struct HostDirectoryGuard {
    // Deliberately one level only: each planned target contributes its basename
    // to the existing parent guard, while every planned directory contributes
    // its own allowed direct children. This proves bounded terminal residuals
    // in owned containers; it does not claim OS-level history or recursively
    // audit unrelated subtrees.
    root_index: usize,
    relative: PathBuf,
    path: String,
    baseline: Option<Vec<HostDirectMember>>,
    allowed_direct_children: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg(unix)]
struct HostTraversalGuard {
    root_index: usize,
    relative: PathBuf,
    device: u64,
    inode: u64,
    mode: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg(unix)]
enum HostTargetState {
    Missing,
    Directory {
        device: u64,
        inode: u64,
        mode: u32,
    },
    File {
        device: u64,
        inode: u64,
        mode: u32,
        size: u64,
        sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg(unix)]
struct HostDirectMember {
    name: String,
    kind: String,
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    sha256: Option<String>,
}

#[derive(Clone)]
struct DetailsRecord {
    session_ref: String,
    binding_hash: String,
    _action_ref: Option<String>,
    artifact: ContentAddressedArtifactRef,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct OutcomeGrantRecord {
    session_ref: String,
    binding_hash: String,
    action_ref: String,
    host_id: String,
    workspace: PathBuf,
    plan_hash: String,
    policy_hash: String,
    instruction_digest: String,
    /// Monotonic per authenticated host/workspace binding. Opening a newer
    /// binding generation makes every older, otherwise-valid outcome grant
    /// stale; stale receipts are rejected and never consume the current grant.
    generation_key: String,
    generation: u64,
    deadline_unix_ms: u64,
    token: String,
    details: DetailsReference,
    consumed: bool,
}

pub(crate) struct ControlPlane<A: EffectAdapter> {
    adapter: A,
    sealing_key: String,
    next_id: u64,
    sessions: HashMap<String, SessionRecord>,
    actions: HashMap<String, ActionRecord<A::Action>>,
    details: HashMap<String, DetailsRecord>,
    binding_generations: HashMap<String, u64>,
    outcome_grants: HashMap<String, OutcomeGrantRecord>,
    session_lru: VecDeque<String>,
    action_lru: VecDeque<String>,
    details_lru: VecDeque<String>,
    details_total_bytes: usize,
}

impl<A: EffectAdapter> ControlPlane<A> {
    pub fn new(adapter: A) -> Result<Self, ControlPlaneError> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|error| ControlPlaneError {
            code: "control_plane_entropy_unavailable",
            detail: error.to_string(),
        })?;
        Ok(Self::with_sealing_key(adapter, sha256(random)))
    }

    /// Restore a process-independent CLI flow with a key authenticated by the
    /// workspace service. The key is never accepted from product argv.
    pub fn with_sealing_key(adapter: A, sealing_key: String) -> Self {
        Self {
            adapter,
            sealing_key,
            next_id: 0,
            sessions: HashMap::new(),
            actions: HashMap::new(),
            details: HashMap::new(),
            binding_generations: HashMap::new(),
            outcome_grants: HashMap::new(),
            session_lru: VecDeque::new(),
            action_lru: VecDeque::new(),
            details_lru: VecDeque::new(),
            details_total_bytes: 0,
        }
    }

    pub fn open(&mut self, request: OpenRequest) -> Result<OpenedSession, ControlPlaneError> {
        validate_binding(&request.binding)?;
        validate_hash("policy_hash", &request.policy_hash)?;
        let inspection = self
            .adapter
            .inspect_pending(&request.binding)
            .map_err(|error| ControlPlaneError {
                code: "pending_transaction_inspection_failed",
                detail: format!("{}: {}", error.code, error.detail),
            })?;
        let binding_hash = sha256(request.binding.canonical_bytes());
        if matches!(&request.binding.surface, BindingSurface::Cli { .. }) {
            let reusable = self
                .sessions
                .iter()
                .find(|(_, session)| {
                    session.binding_hash == binding_hash
                        && session.policy_hash == request.policy_hash
                        && (inspection.active.is_none()
                            || session.pending_recovery_action_ref.is_some())
                })
                .map(|(session_ref, session)| (session_ref.clone(), session.clone()));
            if let Some((session_ref, session)) = reusable {
                touch_lru(&mut self.session_lru, &session_ref);
                return Ok(OpenedSession {
                    session_ref,
                    seal: session.seal,
                    pending_recovery_required: session.pending_recovery_action_ref.is_some(),
                    pending_recovery_action_ref: session.pending_recovery_action_ref,
                    recovery_receipt: inspection.terminal_receipts.first().cloned(),
                    terminal_recovery_count: inspection.terminal_receipts.len() as u32,
                    terminal_recovery_digest: (!inspection.terminal_receipts.is_empty()).then(
                        || {
                            sha256(
                                serde_json::to_vec(&inspection.terminal_receipts)
                                    .expect("terminal recovery receipts serialize"),
                            )
                        },
                    ),
                });
            }
        }
        let generation_key = sha256(format!(
            "{}\n{}\n{}",
            request.binding.host_id,
            request.binding.canonical_workspace.display(),
            request.binding.workspace_identity
        ));
        let generation = self
            .binding_generations
            .entry(generation_key.clone())
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(1)
            .to_owned();
        while self.sessions.len() >= MAX_ACTIVE_SESSIONS {
            self.evict_oldest_session();
        }
        self.next_id = self.next_id.saturating_add(1);
        let session_ref = short_id(
            "session-v2",
            &sha256(format!(
                "{}\n{}\n{}\n{}",
                self.sealing_key, binding_hash, request.policy_hash, self.next_id
            )),
        );
        let seal = sha256(format!(
            "{}\n{}\n{}\n{}",
            self.sealing_key, session_ref, binding_hash, request.policy_hash
        ));
        self.sessions.insert(
            session_ref.clone(),
            SessionRecord {
                binding: request.binding,
                binding_hash,
                policy_hash: request.policy_hash,
                seal: seal.clone(),
                generation_key,
                generation,
                pending_recovery_action_ref: None,
            },
        );
        touch_lru(&mut self.session_lru, &session_ref);
        let pending_recovery_action_ref = if let Some(pending) = inspection.active {
            let payload_hash = sha256(format!(
                "internal-recovery\n{}\n{}\n{}",
                pending.operation.as_str(),
                pending.journal_identity_digest,
                pending.journal_state_digest,
            ));
            let action_digest = sha256(format!(
                "ags-control-plane/recovery-action/v2\n{}\n{}\n{}",
                pending.journal_identity_digest,
                pending.journal_state_digest,
                pending.expected_write_paths.join("\n")
            ));
            let binding = self
                .sessions
                .get(&session_ref)
                .expect("session was just inserted")
                .clone();
            let steps = vec![PlanStep {
                step_id: "recover-pending-transaction".to_string(),
                description: "Recover one integrity-bound pending transaction".to_string(),
            }];
            let plan_hash = plan_hash(
                pending.operation,
                OperationKind::Transaction,
                &binding,
                &payload_hash,
                &steps,
                &pending.expected_write_paths,
                Recoverability::Transactional,
                None,
                Some(&VerificationSpec {
                    checks: vec!["journal-pre-or-post-state-only".to_string()],
                }),
                &action_digest,
            );
            let plan = SealedPlan {
                schema_version: CONTRACT_SCHEMA_VERSION.to_string(),
                plan_hash,
                operation: pending.operation,
                kind: OperationKind::Transaction,
                binding_hash: binding.binding_hash.clone(),
                policy_hash: binding.policy_hash.clone(),
                payload_hash,
                action_digest,
                steps,
                expected_write_paths: pending.expected_write_paths,
                verification: VerificationSpec {
                    checks: vec!["journal-pre-or-post-state-only".to_string()],
                },
                recoverability: Recoverability::Transactional,
                execution: None,
            };
            self.next_id = self.next_id.saturating_add(1);
            let nonce = self.next_id;
            let action_ref = short_id(
                "action-v2",
                &sha256(format!(
                    "{}\n{}\n{}\n{}\n{}\n{}",
                    self.sealing_key,
                    session_ref,
                    plan.plan_hash,
                    plan.payload_hash,
                    plan.binding_hash,
                    nonce
                )),
            );
            self.actions.insert(
                action_ref.clone(),
                ActionRecord {
                    session_ref: session_ref.clone(),
                    plan,
                    operation: ActionOperation::Recovery {
                        original_operation: pending.operation,
                        journal_identity_digest: pending.journal_identity_digest,
                        journal_state_digest: pending.journal_state_digest,
                    },
                    domain_action: pending.action,
                    host_physical_seal: None,
                    consumed: false,
                    nonce,
                },
            );
            touch_lru(&mut self.action_lru, &action_ref);
            self.sessions
                .get_mut(&session_ref)
                .expect("session was just inserted")
                .pending_recovery_action_ref = Some(action_ref.clone());
            Some(action_ref)
        } else {
            None
        };
        Ok(OpenedSession {
            session_ref,
            seal,
            pending_recovery_required: pending_recovery_action_ref.is_some(),
            pending_recovery_action_ref,
            recovery_receipt: inspection.terminal_receipts.first().cloned(),
            terminal_recovery_count: inspection.terminal_receipts.len() as u32,
            terminal_recovery_digest: (!inspection.terminal_receipts.is_empty()).then(|| {
                sha256(
                    serde_json::to_vec(&inspection.terminal_receipts)
                        .expect("terminal recovery receipts serialize"),
                )
            }),
        })
    }

    pub fn decide(
        &mut self,
        session: &OpenedSession,
        operation: OperationRequest,
    ) -> Result<Decision, ControlPlaneError> {
        let binding = self.session(session)?.clone();
        touch_lru(&mut self.session_lru, &session.session_ref);
        verify_operation_workspace(&operation, &binding.binding.canonical_workspace)?;
        let payload = serde_json::to_vec(&operation).map_err(|error| ControlPlaneError {
            code: "operation_payload_invalid",
            detail: error.to_string(),
        })?;
        let payload_hash = sha256(payload);
        let kind = operation.kind();
        if let ControlPlaneDispatch::DetailsRead(request) = operation.dispatch_control_plane() {
            let result = serde_json::to_value(self.read_details_record(
                &session.session_ref,
                &binding,
                request,
            )?)
            .map_err(|error| ControlPlaneError {
                code: "details_chunk_encode_failed",
                detail: error.to_string(),
            })?;
            let output_digest = sha256(
                serde_json::to_vec(&result).expect("DetailsChunk always serializes to JSON"),
            );
            let plan_hash = plan_hash(
                operation.name(),
                kind,
                &binding,
                &payload_hash,
                &[],
                &[],
                Recoverability::NotApplicable,
                None,
                None,
                &sha256("details-read"),
            );
            return Ok(Decision {
                state: OperationState::NoChange,
                kind,
                result: Some(result),
                plan: None,
                action_ref: None,
                receipt: Some(receipt(
                    operation.name(),
                    ReceiptStatus::Succeeded,
                    &plan_hash,
                    &payload_hash,
                    &binding.binding_hash,
                    &output_digest,
                    Vec::new(),
                    false,
                )),
            });
        }
        if binding.pending_recovery_action_ref.is_some()
            && operation.name() != OperationName::Schema
        {
            return Err(ControlPlaneError {
                code: "pending_transaction_recovery_required",
                detail:
                    "apply the sealed pending recovery action before planning another operation"
                        .to_string(),
            });
        }
        self.adapter
            .validate_platform_support(&operation)
            .map_err(|error| control_plane_effect_error(error, "operation_platform_unsupported"))?;
        if kind == OperationKind::ReadOnly {
            let roots = self.adapter.read_only_roots(&operation, &binding.binding);
            let before = tree_digest(&roots)?;
            let observation = self
                .adapter
                .read(&operation, &binding.binding)
                .map_err(|error| control_plane_effect_error(error, "read_only_failed"))?;
            let after = tree_digest(&roots)?;
            if before != after {
                return Err(ControlPlaneError {
                    code: "read_only_write_detected",
                    detail: "ReadOnly operation changed a protected tree".to_string(),
                });
            }
            let plan_hash = plan_hash(
                operation.name(),
                kind,
                &binding,
                &payload_hash,
                &[],
                &[],
                Recoverability::NotApplicable,
                None,
                None,
                &sha256("read-only"),
            );
            let receipt = receipt(
                operation.name(),
                if observation.succeeded {
                    ReceiptStatus::Succeeded
                } else {
                    ReceiptStatus::Failed
                },
                &plan_hash,
                &payload_hash,
                &binding.binding_hash,
                &observation.output_digest,
                Vec::new(),
                false,
            );
            let result = self.bound_details_result(
                &session.session_ref,
                &binding.binding_hash,
                observation.result,
            )?;
            return Ok(Decision {
                state: OperationState::NoChange,
                kind,
                result: Some(result),
                plan: None,
                action_ref: None,
                receipt: Some(receipt),
            });
        }

        let mut domain = match self
            .adapter
            .plan(&operation, &binding.binding)
            .map_err(|error| control_plane_effect_error(error, "operation_plan_failed"))?
        {
            PlanDisposition::NoChange { output_digest } => {
                let plan_hash = plan_hash(
                    operation.name(),
                    kind,
                    &binding,
                    &payload_hash,
                    &[],
                    &[],
                    Recoverability::NotApplicable,
                    None,
                    None,
                    &sha256("no-change"),
                );
                let receipt = receipt(
                    operation.name(),
                    ReceiptStatus::Succeeded,
                    &plan_hash,
                    &payload_hash,
                    &binding.binding_hash,
                    &output_digest,
                    Vec::new(),
                    false,
                );
                return Ok(Decision {
                    state: OperationState::NoChange,
                    kind,
                    result: None,
                    plan: None,
                    action_ref: None,
                    receipt: Some(receipt),
                });
            }
            PlanDisposition::Planned(planned) => *planned,
        };
        validate_domain_plan(kind, &domain.plan, &binding.binding)?;
        let host_physical_seal = if kind == OperationKind::HostDelegated
            && self
                .adapter
                .seals_host_physical_state(&operation, &domain.action)
        {
            let seal =
                seal_host_physical_state(&domain.plan.expected_write_paths, &binding.binding)?;
            let physical_digest = host_physical_seal_digest(&seal)?;
            domain.plan.action_digest = sha256(format!(
                "{}\n{}",
                domain.plan.action_digest, physical_digest
            ));
            Some(seal)
        } else {
            None
        };
        let plan_hash = plan_hash(
            operation.name(),
            kind,
            &binding,
            &payload_hash,
            &domain.plan.steps,
            &domain.plan.expected_write_paths,
            domain.plan.recoverability,
            domain.plan.execution.as_ref(),
            Some(&domain.plan.verification),
            &domain.plan.action_digest,
        );
        let plan = SealedPlan {
            schema_version: CONTRACT_SCHEMA_VERSION.to_string(),
            plan_hash: plan_hash.clone(),
            operation: operation.name(),
            kind,
            binding_hash: binding.binding_hash.clone(),
            policy_hash: binding.policy_hash.clone(),
            payload_hash,
            action_digest: domain.plan.action_digest,
            steps: domain.plan.steps,
            expected_write_paths: domain.plan.expected_write_paths,
            verification: domain.plan.verification,
            recoverability: domain.plan.recoverability,
            execution: domain.plan.execution,
        };
        self.next_id = self.next_id.saturating_add(1);
        let action_nonce = self.next_id;
        let action_ref = short_id(
            "action-v2",
            &sha256(format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                self.sealing_key,
                session.session_ref,
                plan.plan_hash,
                plan.payload_hash,
                plan.binding_hash,
                action_nonce
            )),
        );
        while self.actions.len() >= MAX_ACTIVE_ACTIONS {
            self.evict_oldest_action();
        }
        self.actions.insert(
            action_ref.clone(),
            ActionRecord {
                session_ref: session.session_ref.clone(),
                plan: plan.clone(),
                operation: ActionOperation::External(Box::new(operation)),
                domain_action: domain.action,
                host_physical_seal,
                consumed: false,
                nonce: action_nonce,
            },
        );
        touch_lru(&mut self.action_lru, &action_ref);
        Ok(Decision {
            state: OperationState::Planned,
            kind,
            result: None,
            plan: Some(plan),
            action_ref: Some(action_ref),
            receipt: None,
        })
    }

    /// Consume an action using binding facts authenticated by the current
    /// transport. CLI callers send only `action_ref`; MCP callers are checked
    /// against their original connection and authenticated session here.
    pub fn apply(
        &mut self,
        caller: &AuthenticatedBinding,
        request: ApplyRequest,
    ) -> Result<ApplyResult, ControlPlaneError> {
        let action_ref = request.action_ref.clone();
        let action = self
            .actions
            .get(&action_ref)
            .ok_or_else(|| ControlPlaneError {
                code: "action_ref_invalid",
                detail: "unknown, retired, or evicted action_ref".to_string(),
            })?;
        let session_ref = action.session_ref.clone();
        let binding_hash = action.plan.binding_hash.clone();
        touch_lru(&mut self.action_lru, &action_ref);
        touch_lru(&mut self.session_lru, &session_ref);
        let result = self.apply_unbounded(caller, request)?;
        let terminal = result.state != OperationState::AwaitingOutcome;
        let recovery_terminal = matches!(
            result.reason_code.as_deref(),
            Some(
                "transaction_recovered"
                    | "recovery_finalize_failed"
                    | "transaction_recovery_failed"
            )
        );
        let bounded = self.bound_apply_result(&session_ref, &binding_hash, &action_ref, result)?;
        if terminal {
            let recovery_session = self.sessions.get(&session_ref).is_some_and(|session| {
                session.pending_recovery_action_ref.as_deref() == Some(action_ref.as_str())
            });
            if recovery_session || recovery_terminal {
                self.evict_session(&session_ref);
            } else {
                self.retire_action(&action_ref);
            }
        }
        Ok(bounded)
    }

    fn apply_unbounded(
        &mut self,
        caller: &AuthenticatedBinding,
        request: ApplyRequest,
    ) -> Result<ApplyResult, ControlPlaneError> {
        let action = self
            .actions
            .get(&request.action_ref)
            .cloned()
            .ok_or_else(|| ControlPlaneError {
                code: "action_ref_invalid",
                detail: "unknown or tampered action_ref".to_string(),
            })?;
        let binding = self
            .sessions
            .get(&action.session_ref)
            .cloned()
            .ok_or_else(|| ControlPlaneError {
                code: "action_ref_invalid",
                detail: "action_ref session no longer exists".to_string(),
            })?;
        let caller_hash = sha256(caller.canonical_bytes());
        if action.plan.binding_hash != binding.binding_hash || caller_hash != binding.binding_hash {
            return Err(ControlPlaneError { code: "action_ref_cross_binding", detail: "action_ref is sealed to a different connection, host, workspace, authenticated session, or CLI flow".to_string() });
        }
        if action.consumed {
            return Err(ControlPlaneError {
                code: "action_ref_replayed",
                detail: "action_ref was already consumed".to_string(),
            });
        }
        let expected_ref = short_id(
            "action-v2",
            &sha256(format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                self.sealing_key,
                action.session_ref,
                action.plan.plan_hash,
                action.plan.payload_hash,
                action.plan.binding_hash,
                action.nonce
            )),
        );
        if expected_ref != request.action_ref {
            return Err(ControlPlaneError {
                code: "action_ref_tampered",
                detail: "action_ref seal does not match the stored plan".to_string(),
            });
        }

        if action.plan.schema_version != CONTRACT_SCHEMA_VERSION
            || action.plan.binding_hash != binding.binding_hash
            || action.plan.policy_hash != binding.policy_hash
        {
            return Err(ControlPlaneError {
                code: "sealed_action_mismatch",
                detail: "stored plan authority fields do not match the authenticated session"
                    .to_string(),
            });
        }
        match &action.operation {
            ActionOperation::External(operation) => {
                let payload =
                    serde_json::to_vec(operation.as_ref()).map_err(|error| ControlPlaneError {
                        code: "sealed_action_mismatch",
                        detail: format!("stored operation cannot be encoded: {error}"),
                    })?;
                if action.plan.payload_hash != sha256(payload)
                    || action.plan.operation != operation.name()
                    || action.plan.kind != operation.kind()
                {
                    return Err(ControlPlaneError {
                        code: "sealed_action_mismatch",
                        detail: "stored operation identity differs from the sealed plan"
                            .to_string(),
                    });
                }
            }
            ActionOperation::Recovery {
                original_operation,
                journal_identity_digest,
                journal_state_digest,
            } => {
                if *original_operation != action.plan.operation
                    || action.plan.kind != OperationKind::Transaction
                    || action.plan.payload_hash
                        != sha256(format!(
                            "internal-recovery\n{}\n{}\n{}",
                            original_operation.as_str(),
                            journal_identity_digest,
                            journal_state_digest,
                        ))
                {
                    return Err(ControlPlaneError {
                        code: "recovery_action_identity_mismatch",
                        detail: "typed recovery identity does not match its sealed plan"
                            .to_string(),
                    });
                }
            }
        }
        if recompute_sealed_plan_hash(&action.plan, &binding) != action.plan.plan_hash {
            return Err(ControlPlaneError {
                code: "sealed_action_mismatch",
                detail: "stored plan hash does not match its canonical fields".to_string(),
            });
        }
        if let Some(base_digest) = self
            .adapter
            .semantic_action_digest(&action.domain_action)
            .map_err(|error| control_plane_effect_error(error, "sealed_action_mismatch"))?
        {
            let effective_digest = if let Some(seal) = &action.host_physical_seal {
                sha256(format!(
                    "{}\n{}",
                    base_digest,
                    host_physical_seal_digest(seal)?
                ))
            } else {
                base_digest
            };
            if effective_digest != action.plan.action_digest {
                return Err(ControlPlaneError {
                    code: "sealed_action_mismatch",
                    detail: "stored domain action differs from the sealed action digest"
                        .to_string(),
                });
            }
        }
        self.adapter
            .validate_sealed_action(&action.plan, &action.domain_action, &binding.binding)
            .map_err(|error| control_plane_effect_error(error, "sealed_action_mismatch"))?;

        if action.plan.kind == OperationKind::HostDelegated {
            let Some(outcome) = request.outcome else {
                return self.issue_outcome_grant(&action, &binding, &request.action_ref);
            };
            if outcome.bytes.len() > MAX_DETAILS_ARTIFACT_BYTES {
                return Err(ControlPlaneError {
                    code: "host_outcome_artifact_too_large",
                    detail: format!(
                        "host outcome artifact exceeds {} bytes",
                        MAX_DETAILS_ARTIFACT_BYTES
                    ),
                });
            }
            if outcome.binding != binding.binding {
                return Err(ControlPlaneError {
                    code: "host_outcome_cross_binding",
                    detail: "host outcome was authenticated for a different binding".to_string(),
                });
            }
            let grant = self
                .outcome_grants
                .get(&request.action_ref)
                .cloned()
                .ok_or_else(|| ControlPlaneError {
                    code: "host_outcome_token_not_issued",
                    detail: "first apply must issue an outcome token".to_string(),
                })?;
            self.validate_outcome_grant(&grant, &action, &binding)?;
            if sha256(&outcome.bytes) != outcome.artifact.sha256 {
                return Err(ControlPlaneError {
                    code: "host_outcome_artifact_digest_mismatch",
                    detail: "host outcome bytes do not match the content-addressed reference"
                        .to_string(),
                });
            }
            validate_hash("host_outcome.artifact.sha256", &outcome.artifact.sha256)?;
            let receipt_value: HostOutcomeReceipt = serde_json::from_slice(&outcome.bytes)
                .map_err(|error| ControlPlaneError {
                    code: "host_outcome_artifact_invalid",
                    detail: error.to_string(),
                })?;
            let failure_terminal = matches!(
                receipt_value.status,
                HostOutcomeStatus::Failed | HostOutcomeStatus::Abandoned
            );
            let pre_scan = if failure_terminal {
                action
                    .host_physical_seal
                    .as_ref()
                    .map(|seal| verify_host_physical_delta(seal, &receipt_value))
            } else {
                None
            };
            let terminal_verification = (|| {
                self.validate_host_outcome_receipt(&grant, &action, &binding, &receipt_value)?;
                let verified_evidence = verify_host_evidence(receipt_value.evidence.as_ref())?;
                let ActionOperation::External(operation) = &action.operation else {
                    return Err(ControlPlaneError {
                        code: "recovery_action_kind_mismatch",
                        detail: "internal recovery action reached host verification".to_string(),
                    });
                };
                self.adapter
                    .verify_host_outcome(
                        operation,
                        &action.plan,
                        &action.domain_action,
                        &binding.binding,
                        &receipt_value,
                        verified_evidence.as_ref(),
                    )
                    .map_err(|error| ControlPlaneError {
                        code: "host_outcome_verification_failed",
                        detail: format!("{}: {}", error.code, error.detail),
                    })?;
                verify_host_write_artifacts(
                    &action.plan,
                    &receipt_value,
                    &binding.binding,
                    !matches!(operation.as_ref(), OperationRequest::Test(_)),
                )
            })();
            let final_scan = if failure_terminal {
                action
                    .host_physical_seal
                    .as_ref()
                    .map(|seal| verify_host_physical_delta(seal, &receipt_value))
            } else {
                None
            };
            if failure_terminal {
                let (authoritative_paths, final_scan_error) = final_scan
                    .map(host_terminal_delta_paths)
                    .unwrap_or_else(|| {
                        (
                            vec!["ags://unprovable-host-delta/no-physical-seal".to_string()],
                            Some(ControlPlaneError {
                                code: "host_outcome_physical_seal_missing",
                                detail: "failed host outcome has no physical-state seal"
                                    .to_string(),
                            }),
                        )
                    });
                let pre_scan_error = pre_scan.and_then(|delta| host_terminal_delta_paths(delta).1);
                let verification_failed = terminal_verification.is_err()
                    || pre_scan_error.is_some()
                    || final_scan_error.is_some();
                if let Some(record) = self.outcome_grants.get_mut(&request.action_ref) {
                    record.consumed = true;
                }
                self.consume(&request.action_ref);
                let receipt = receipt(
                    action.plan.operation,
                    if verification_failed {
                        ReceiptStatus::RiskEscalated
                    } else {
                        ReceiptStatus::Failed
                    },
                    &action.plan.plan_hash,
                    &action.plan.payload_hash,
                    &action.plan.binding_hash,
                    if ags_platform::is_sha256(&receipt_value.output_digest) {
                        &receipt_value.output_digest
                    } else {
                        &outcome.artifact.sha256
                    },
                    authoritative_paths,
                    false,
                );
                return Ok(ApplyResult {
                    state: if verification_failed {
                        OperationState::RiskEscalated
                    } else {
                        OperationState::Receipted
                    },
                    transitions: vec![
                        OperationState::Verifying,
                        if verification_failed {
                            OperationState::RiskEscalated
                        } else {
                            OperationState::Receipted
                        },
                    ],
                    receipt: Some(receipt),
                    reason_code: verification_failed
                        .then(|| "host_outcome_unprovable_failure".to_string()),
                    details: None,
                    outcome_token: None,
                    outcome_generation: None,
                    outcome_deadline_unix_ms: None,
                });
            }
            terminal_verification?;
            if let Some(record) = self.outcome_grants.get_mut(&request.action_ref) {
                record.consumed = true;
            }
            self.consume(&request.action_ref);
            let receipt = receipt(
                action.plan.operation,
                ReceiptStatus::Succeeded,
                &action.plan.plan_hash,
                &action.plan.payload_hash,
                &action.plan.binding_hash,
                &receipt_value.output_digest,
                receipt_value.observed_write_set,
                false,
            );
            return Ok(ApplyResult {
                state: OperationState::Receipted,
                transitions: vec![OperationState::Verifying, OperationState::Receipted],
                receipt: Some(receipt),
                reason_code: None,
                details: None,
                outcome_token: None,
                outcome_generation: None,
                outcome_deadline_unix_ms: None,
            });
        }

        if request.outcome.is_some() {
            return Err(ControlPlaneError {
                code: "unexpected_host_outcome",
                detail: "only HostDelegated operations accept a host outcome".to_string(),
            });
        }

        let mut transitions = vec![OperationState::Applying];
        let observation = match self.adapter.apply(
            &request.action_ref,
            &action.plan,
            &action.domain_action,
            match &action.operation {
                ActionOperation::External(operation) => Some(operation.as_ref()),
                ActionOperation::Recovery { .. } => None,
            },
            &binding.binding,
        ) {
            Ok(observation) => observation,
            Err(mut error) => {
                // Crossing the adapter call boundary means effects may have
                // begun. A fallible adapter cannot downgrade that fact and
                // thereby suppress Transaction recovery.
                error.effect_started = true;
                EffectObservation::bounded(
                    false,
                    error.effect_started,
                    if error.output_digest.is_empty() {
                        sha256(format!("{}:{}", error.code, error.detail))
                    } else {
                        error.output_digest
                    },
                    error.observed_write_set,
                    None,
                )
                .unwrap_or_else(EffectObservation::contract_violation)
            }
        };

        if let Some(reason) = observation.contract_violation.clone() {
            self.consume(&request.action_ref);
            transitions.push(OperationState::RiskEscalated);
            let receipt = receipt_with_evidence(
                action.plan.operation,
                ReceiptStatus::RiskEscalated,
                &action.plan.plan_hash,
                &action.plan.payload_hash,
                &action.plan.binding_hash,
                &observation.output_digest,
                observation.observed_write_set,
                false,
                observation.evidence,
            );
            return Ok(ApplyResult {
                state: OperationState::RiskEscalated,
                transitions,
                receipt: Some(receipt),
                reason_code: Some(reason),
                details: None,
                outcome_token: None,
                outcome_generation: None,
                outcome_deadline_unix_ms: None,
            });
        }

        if action.plan.kind == OperationKind::LocalExecution {
            self.consume(&request.action_ref);
            if has_unexpected_writes(
                &action.plan.expected_write_paths,
                &observation.observed_write_set,
            ) {
                let receipt = receipt_with_evidence(
                    action.plan.operation,
                    ReceiptStatus::RiskEscalated,
                    &action.plan.plan_hash,
                    &action.plan.payload_hash,
                    &action.plan.binding_hash,
                    &observation.output_digest,
                    observation.observed_write_set,
                    false,
                    observation.evidence,
                );
                transitions.push(OperationState::RiskEscalated);
                return Ok(ApplyResult {
                    state: OperationState::RiskEscalated,
                    transitions,
                    receipt: Some(receipt),
                    reason_code: Some("unexpected_write_set".to_string()),
                    details: None,
                    outcome_token: None,
                    outcome_generation: None,
                    outcome_deadline_unix_ms: None,
                });
            }
            transitions.push(OperationState::Verifying);
            let verified = observation.succeeded
                && self
                    .adapter
                    .verify(
                        &request.action_ref,
                        &action.plan,
                        &action.domain_action,
                        &observation,
                    )
                    .map(|value| value.passed)
                    .unwrap_or(false);
            transitions.push(OperationState::Receipted);
            let receipt = receipt_with_evidence(
                action.plan.operation,
                if verified {
                    ReceiptStatus::Succeeded
                } else {
                    ReceiptStatus::Failed
                },
                &action.plan.plan_hash,
                &action.plan.payload_hash,
                &action.plan.binding_hash,
                &observation.output_digest,
                observation.observed_write_set,
                false,
                observation.evidence,
            );
            return Ok(ApplyResult {
                state: OperationState::Receipted,
                transitions,
                receipt: Some(receipt),
                reason_code: None,
                details: None,
                outcome_token: None,
                outcome_generation: None,
                outcome_deadline_unix_ms: None,
            });
        }

        if has_unexpected_writes(
            &action.plan.expected_write_paths,
            &observation.observed_write_set,
        ) {
            self.consume(&request.action_ref);
            transitions.push(OperationState::RiskEscalated);
            let receipt = receipt_with_evidence(
                action.plan.operation,
                ReceiptStatus::RiskEscalated,
                &action.plan.plan_hash,
                &action.plan.payload_hash,
                &action.plan.binding_hash,
                &observation.output_digest,
                observation.observed_write_set,
                false,
                observation.evidence,
            );
            return Ok(ApplyResult {
                state: OperationState::RiskEscalated,
                transitions,
                receipt: Some(receipt),
                reason_code: Some("unexpected_write_set".to_string()),
                details: None,
                outcome_token: None,
                outcome_generation: None,
                outcome_deadline_unix_ms: None,
            });
        }
        transitions.push(OperationState::Verifying);
        let verified = observation.succeeded
            && self
                .adapter
                .verify(
                    &request.action_ref,
                    &action.plan,
                    &action.domain_action,
                    &observation,
                )
                .map(|value| value.passed)
                .unwrap_or(false);
        if verified {
            let recovered = self.adapter.is_recovery_action(&action.domain_action);
            let evidence = if recovered {
                let ActionOperation::Recovery {
                    journal_identity_digest,
                    journal_state_digest,
                    ..
                } = &action.operation
                else {
                    unreachable!("typed recovery action must carry its original journal digest")
                };
                Some(durable_recovery_evidence(
                    &request.action_ref,
                    &action.plan.policy_hash,
                    journal_identity_digest,
                    journal_state_digest,
                    observation.evidence,
                ))
            } else {
                observation.evidence
            };
            let receipt = receipt_with_evidence(
                action.plan.operation,
                if recovered {
                    ReceiptStatus::Recovered
                } else {
                    ReceiptStatus::Succeeded
                },
                &action.plan.plan_hash,
                &action.plan.payload_hash,
                &action.plan.binding_hash,
                &observation.output_digest,
                observation.observed_write_set,
                recovered,
                evidence,
            );
            if recovered {
                if let Err(error) = self.adapter.finalize_recovery(
                    &request.action_ref,
                    &action.plan,
                    &action.domain_action,
                    &binding.binding,
                    &receipt,
                ) {
                    self.consume(&request.action_ref);
                    transitions.push(OperationState::RiskEscalated);
                    let mut observed = receipt.observed_write_set.clone();
                    observed.extend(error.observed_write_set);
                    observed.sort();
                    observed.dedup();
                    let output_digest = if error.output_digest.is_empty() {
                        sha256(format!("{}:{}", error.code, error.detail))
                    } else {
                        error.output_digest.clone()
                    };
                    return Ok(ApplyResult {
                        state: OperationState::RiskEscalated,
                        transitions,
                        receipt: Some(receipt_with_evidence(
                            action.plan.operation,
                            ReceiptStatus::RiskEscalated,
                            &action.plan.plan_hash,
                            &action.plan.payload_hash,
                            &action.plan.binding_hash,
                            &output_digest,
                            observed,
                            false,
                            Some(serde_json::json!({
                                "recovery_finalize_error": error.code,
                                "detail": error.detail,
                            })),
                        )),
                        reason_code: Some("recovery_finalize_failed".to_string()),
                        details: None,
                        outcome_token: None,
                        outcome_generation: None,
                        outcome_deadline_unix_ms: None,
                    });
                }
            }
            self.consume(&request.action_ref);
            transitions.push(OperationState::Receipted);
            return Ok(ApplyResult {
                state: OperationState::Receipted,
                transitions,
                receipt: Some(receipt),
                reason_code: recovered.then(|| "transaction_recovered".to_string()),
                details: None,
                outcome_token: None,
                outcome_generation: None,
                outcome_deadline_unix_ms: None,
            });
        }

        transitions.push(OperationState::Recovering);
        let recovery = if self.adapter.is_recovery_action(&action.domain_action) {
            Err(EffectError {
                code: "recovery_action_not_recursively_recoverable".to_string(),
                detail: "a failed recovery action cannot enter ordinary recovery".to_string(),
                effect_started: observation.effect_started,
                output_digest: observation.output_digest.clone(),
                observed_write_set: observation.observed_write_set.clone(),
            })
        } else if observation.effect_started {
            self.adapter.recover(
                &request.action_ref,
                &action.plan,
                &action.domain_action,
                &observation,
            )
        } else {
            Ok(RecoveryObservation {
                succeeded: true,
                output_digest: sha256("transaction-recovery-noop"),
                observed_write_set: Vec::new(),
                evidence: None,
                original_journal_digest: None,
            })
        };
        let mut observed = observation.observed_write_set.clone();
        match recovery {
            Ok(recovery) if recovery.succeeded => {
                observed.extend(recovery.observed_write_set);
                observed.sort();
                observed.dedup();
                let original_journal_digest = recovery
                    .original_journal_digest
                    .unwrap_or_else(|| sha256("adapter-recovery-without-journal"));
                let evidence = Some(durable_recovery_evidence(
                    &request.action_ref,
                    &action.plan.policy_hash,
                    &original_journal_digest,
                    &original_journal_digest,
                    recovery.evidence.or(observation.evidence.clone()),
                ));
                let receipt = receipt_with_evidence(
                    action.plan.operation,
                    ReceiptStatus::Recovered,
                    &action.plan.plan_hash,
                    &action.plan.payload_hash,
                    &action.plan.binding_hash,
                    &recovery.output_digest,
                    observed,
                    true,
                    evidence,
                );
                if let Err(error) = self.adapter.finalize_recovery(
                    &request.action_ref,
                    &action.plan,
                    &action.domain_action,
                    &binding.binding,
                    &receipt,
                ) {
                    self.consume(&request.action_ref);
                    transitions.push(OperationState::RiskEscalated);
                    let mut failed_writes = receipt.observed_write_set.clone();
                    failed_writes.extend(error.observed_write_set);
                    failed_writes.sort();
                    failed_writes.dedup();
                    let output_digest = if error.output_digest.is_empty() {
                        sha256(format!("{}:{}", error.code, error.detail))
                    } else {
                        error.output_digest.clone()
                    };
                    return Ok(ApplyResult {
                        state: OperationState::RiskEscalated,
                        transitions,
                        receipt: Some(receipt_with_evidence(
                            action.plan.operation,
                            ReceiptStatus::RiskEscalated,
                            &action.plan.plan_hash,
                            &action.plan.payload_hash,
                            &action.plan.binding_hash,
                            &output_digest,
                            failed_writes,
                            false,
                            Some(serde_json::json!({
                                "recovery_finalize_error": error.code,
                                "detail": error.detail,
                            })),
                        )),
                        reason_code: Some("recovery_finalize_failed".to_string()),
                        details: None,
                        outcome_token: None,
                        outcome_generation: None,
                        outcome_deadline_unix_ms: None,
                    });
                }
                self.consume(&request.action_ref);
                transitions.push(OperationState::Receipted);
                Ok(ApplyResult {
                    state: OperationState::Receipted,
                    transitions,
                    receipt: Some(receipt),
                    reason_code: Some("transaction_recovered".to_string()),
                    details: None,
                    outcome_token: None,
                    outcome_generation: None,
                    outcome_deadline_unix_ms: None,
                })
            }
            recovery => {
                let (output_digest, evidence) = match recovery {
                    Ok(recovery) => {
                        observed.extend(recovery.observed_write_set);
                        (recovery.output_digest, recovery.evidence)
                    }
                    Err(error) => {
                        observed.extend(error.observed_write_set);
                        let digest = if error.output_digest.is_empty() {
                            sha256(format!("{}:{}", error.code, error.detail))
                        } else {
                            error.output_digest
                        };
                        (
                            digest,
                            Some(serde_json::json!({
                                "recovery_error": error.code,
                                "detail": error.detail,
                            })),
                        )
                    }
                };
                observed.sort();
                observed.dedup();
                self.consume(&request.action_ref);
                transitions.push(OperationState::RiskEscalated);
                Ok(ApplyResult {
                    state: OperationState::RiskEscalated,
                    transitions,
                    receipt: Some(receipt_with_evidence(
                        action.plan.operation,
                        ReceiptStatus::RiskEscalated,
                        &action.plan.plan_hash,
                        &action.plan.payload_hash,
                        &action.plan.binding_hash,
                        &output_digest,
                        observed,
                        false,
                        evidence,
                    )),
                    reason_code: Some("transaction_recovery_failed".to_string()),
                    details: None,
                    outcome_token: None,
                    outcome_generation: None,
                    outcome_deadline_unix_ms: None,
                })
            }
        }
    }

    fn session(&self, session: &OpenedSession) -> Result<&SessionRecord, ControlPlaneError> {
        let record = self
            .sessions
            .get(&session.session_ref)
            .ok_or_else(|| ControlPlaneError {
                code: "session_unknown",
                detail: "opened session is not owned by this control plane".to_string(),
            })?;
        if record.seal != session.seal {
            return Err(ControlPlaneError {
                code: "session_tampered",
                detail: "opened session seal mismatch".to_string(),
            });
        }
        Ok(record)
    }

    fn consume(&mut self, action_ref: &str) {
        if let Some(action) = self.actions.get_mut(action_ref) {
            action.consumed = true;
        }
    }

    fn retire_action(&mut self, action_ref: &str) {
        self.actions.remove(action_ref);
        self.outcome_grants.remove(action_ref);
        remove_lru(&mut self.action_lru, action_ref);
    }

    fn evict_oldest_action(&mut self) {
        if let Some(action_ref) = self.action_lru.pop_front() {
            self.actions.remove(&action_ref);
            self.outcome_grants.remove(&action_ref);
        }
    }

    fn evict_oldest_details(&mut self) {
        if let Some(uri) = self.details_lru.pop_front() {
            if let Some(record) = self.details.remove(&uri) {
                self.details_total_bytes =
                    self.details_total_bytes.saturating_sub(record.bytes.len());
            }
        }
    }

    fn evict_oldest_session(&mut self) {
        let Some(session_ref) = self.session_lru.pop_front() else {
            return;
        };
        self.evict_session(&session_ref);
    }

    fn evict_session(&mut self, session_ref: &str) {
        remove_lru(&mut self.session_lru, session_ref);
        let Some(session) = self.sessions.remove(session_ref) else {
            return;
        };
        let actions = self
            .actions
            .iter()
            .filter(|(_, action)| action.session_ref == session_ref)
            .map(|(action_ref, _)| action_ref.clone())
            .collect::<Vec<_>>();
        for action_ref in actions {
            self.retire_action(&action_ref);
        }
        self.outcome_grants
            .retain(|_, grant| grant.session_ref != session_ref);
        let details = self
            .details
            .iter()
            .filter(|(_, record)| record.session_ref == session_ref)
            .map(|(uri, _)| uri.clone())
            .collect::<Vec<_>>();
        for uri in details {
            remove_lru(&mut self.details_lru, &uri);
            if let Some(record) = self.details.remove(&uri) {
                self.details_total_bytes =
                    self.details_total_bytes.saturating_sub(record.bytes.len());
            }
        }
        if !self
            .sessions
            .values()
            .any(|candidate| candidate.generation_key == session.generation_key)
        {
            self.binding_generations.remove(&session.generation_key);
        }
    }

    fn derive_host_execution_instruction(
        &self,
        action: &ActionRecord<A::Action>,
        binding: &SessionRecord,
        action_ref: &str,
    ) -> Result<HostExecutionInstruction, ControlPlaneError> {
        let ActionOperation::External(operation) = &action.operation else {
            return Err(ControlPlaneError {
                code: "host_execution_instruction_invalid",
                detail: "internal recovery action cannot produce host execution work".to_string(),
            });
        };
        let host_action = if let Some(command) = &action.plan.execution {
            let OperationRequest::Test(request) = operation.as_ref() else {
                return Err(ControlPlaneError {
                    code: "host_execution_instruction_invalid",
                    detail: "sealed command is not paired with a typed Test operation".to_string(),
                });
            };
            if request.executor != TestExecutor::Host {
                return Err(ControlPlaneError {
                    code: "host_execution_instruction_invalid",
                    detail: "HostDelegated command requires the host Test executor".to_string(),
                });
            }
            HostExecutionAction::Command {
                profile: request.profile.clone(),
                program: command.program.clone(),
                argv: command.argv.clone(),
                cwd: command.cwd.clone(),
                env: command.env.clone(),
                timeout_ms: command.timeout_ms,
                allowed_write_paths: command.allowed_write_paths.clone(),
            }
        } else {
            self.adapter
                .host_execution_action(
                    operation,
                    &action.plan,
                    &action.domain_action,
                    &binding.binding,
                )
                .map_err(|error| {
                    control_plane_effect_error(error, "host_execution_instruction_invalid")
                })?
        };
        validate_host_execution_action(&host_action, &action.plan)?;
        let mut instruction = HostExecutionInstruction {
            schema_version: HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION.to_string(),
            action_ref: action_ref.to_string(),
            binding_hash: binding.binding_hash.clone(),
            plan_hash: action.plan.plan_hash.clone(),
            policy_hash: action.plan.policy_hash.clone(),
            instruction_digest: String::new(),
            action: host_action,
        };
        instruction.instruction_digest =
            canonical_host_execution_instruction_digest(&instruction, &action.plan.action_digest)?;
        Ok(instruction)
    }

    fn issue_outcome_grant(
        &mut self,
        action: &ActionRecord<A::Action>,
        binding: &SessionRecord,
        action_ref: &str,
    ) -> Result<ApplyResult, ControlPlaneError> {
        if let Some(existing) = self.outcome_grants.get(action_ref).cloned() {
            self.validate_outcome_grant(&existing, action, binding)?;
            return Ok(ApplyResult {
                state: OperationState::AwaitingOutcome,
                transitions: vec![OperationState::AwaitingOutcome],
                receipt: None,
                reason_code: Some("host_outcome_required".to_string()),
                details: Some(existing.details),
                outcome_token: Some(existing.token),
                outcome_generation: Some(existing.generation),
                outcome_deadline_unix_ms: Some(existing.deadline_unix_ms),
            });
        }
        let instruction = self.derive_host_execution_instruction(action, binding, action_ref)?;
        let instruction_bytes =
            serde_json::to_vec(&instruction).map_err(|error| ControlPlaneError {
                code: "host_execution_instruction_encode_failed",
                detail: error.to_string(),
            })?;
        let details = self.store_details(
            &action.session_ref,
            &binding.binding_hash,
            Some(action_ref),
            instruction_bytes,
        )?;
        let grant_ttl_ms = match &instruction.action {
            HostExecutionAction::Command { timeout_ms, .. } => {
                timeout_ms.saturating_add(10 * 60 * 1000).max(5 * 60 * 1000)
            }
            HostExecutionAction::RuntimeUpdate { .. }
            | HostExecutionAction::ArchiveClosures { .. } => 5 * 60 * 1000,
        };
        let deadline_unix_ms = now_unix_ms().saturating_add(grant_ttl_ms);
        let token = short_id(
            "outcome-v2",
            &sha256(format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                self.sealing_key,
                action_ref,
                binding.binding_hash,
                binding.binding.host_id,
                binding.binding.canonical_workspace.display(),
                action.plan.plan_hash,
                action.plan.policy_hash,
                binding.generation,
                deadline_unix_ms,
                instruction.instruction_digest,
            )),
        );
        self.outcome_grants.insert(
            action_ref.to_string(),
            OutcomeGrantRecord {
                session_ref: action.session_ref.clone(),
                binding_hash: binding.binding_hash.clone(),
                action_ref: action_ref.to_string(),
                host_id: binding.binding.host_id.clone(),
                workspace: binding.binding.canonical_workspace.clone(),
                plan_hash: action.plan.plan_hash.clone(),
                policy_hash: action.plan.policy_hash.clone(),
                instruction_digest: instruction.instruction_digest,
                generation_key: binding.generation_key.clone(),
                generation: binding.generation,
                deadline_unix_ms,
                token: token.clone(),
                details: details.clone(),
                consumed: false,
            },
        );
        Ok(ApplyResult {
            state: OperationState::AwaitingOutcome,
            transitions: vec![OperationState::AwaitingOutcome],
            receipt: None,
            reason_code: Some("host_outcome_required".to_string()),
            details: Some(details),
            outcome_token: Some(token),
            outcome_generation: Some(binding.generation),
            outcome_deadline_unix_ms: Some(deadline_unix_ms),
        })
    }

    fn validate_outcome_grant(
        &self,
        grant: &OutcomeGrantRecord,
        action: &ActionRecord<A::Action>,
        binding: &SessionRecord,
    ) -> Result<(), ControlPlaneError> {
        if grant.consumed {
            return Err(ControlPlaneError {
                code: "host_outcome_token_replayed",
                detail: "outcome token was already consumed".to_string(),
            });
        }
        if now_unix_ms() > grant.deadline_unix_ms {
            return Err(ControlPlaneError {
                code: "host_outcome_token_expired",
                detail: "outcome token deadline elapsed".to_string(),
            });
        }
        if self.binding_generations.get(&grant.generation_key).copied() != Some(grant.generation) {
            return Err(ControlPlaneError {
                code: "host_outcome_generation_stale",
                detail: "a newer authenticated binding generation is active".to_string(),
            });
        }
        if grant.session_ref != action.session_ref
            || grant.binding_hash != binding.binding_hash
            || grant.action_ref.is_empty()
            || grant.host_id != binding.binding.host_id
            || grant.workspace != binding.binding.canonical_workspace
            || grant.plan_hash != action.plan.plan_hash
            || grant.policy_hash != action.plan.policy_hash
            || grant.generation != binding.generation
        {
            return Err(ControlPlaneError {
                code: "host_outcome_token_binding_mismatch",
                detail: "outcome grant no longer matches its sealed action and binding".to_string(),
            });
        }
        let instruction =
            self.derive_host_execution_instruction(action, binding, &grant.action_ref)?;
        if instruction.instruction_digest != grant.instruction_digest {
            return Err(ControlPlaneError {
                code: "host_execution_instruction_binding_mismatch",
                detail: "issued instruction no longer matches the sealed action and binding"
                    .to_string(),
            });
        }
        Ok(())
    }

    fn validate_host_outcome_receipt(
        &self,
        grant: &OutcomeGrantRecord,
        action: &ActionRecord<A::Action>,
        binding: &SessionRecord,
        receipt: &HostOutcomeReceipt,
    ) -> Result<(), ControlPlaneError> {
        if receipt.schema_version != HOST_OUTCOME_SCHEMA_VERSION {
            return Err(ControlPlaneError {
                code: "host_outcome_schema_mismatch",
                detail: "host outcome receipt has the wrong schema".to_string(),
            });
        }
        if receipt.action_ref != grant.action_ref
            || receipt.binding_hash != binding.binding_hash
            || receipt.plan_hash != action.plan.plan_hash
            || receipt.policy_hash != action.plan.policy_hash
            || receipt.instruction_digest != grant.instruction_digest
            || receipt.outcome_token != grant.token
        {
            return Err(ControlPlaneError {
                code: if receipt.instruction_digest != grant.instruction_digest {
                    "host_outcome_instruction_digest_mismatch"
                } else {
                    "host_outcome_token_binding_mismatch"
                },
                detail: "host outcome receipt does not match the sealed grant".to_string(),
            });
        }
        if receipt.generation != grant.generation {
            return Err(ControlPlaneError {
                code: "host_outcome_generation_stale",
                detail: "host outcome receipt generation is stale".to_string(),
            });
        }
        if receipt.observed_write_set.len() > MAX_EFFECT_OBSERVED_WRITES
            || receipt.artifacts.len() > MAX_EFFECT_OBSERVED_WRITES
            || receipt
                .observed_write_set
                .iter()
                .any(|path| path.len() > MAX_EFFECT_PATH_BYTES)
            || receipt
                .artifacts
                .iter()
                .any(|artifact| artifact.path.len() > MAX_EFFECT_PATH_BYTES)
            || receipt
                .observed_write_set
                .iter()
                .map(String::len)
                .sum::<usize>()
                > MAX_EFFECT_TOTAL_PATH_BYTES
        {
            return Err(ControlPlaneError {
                code: "host_outcome_budget_exceeded",
                detail: "host outcome exceeds the sealed terminal result budget".to_string(),
            });
        }
        validate_hash("host_outcome.output_digest", &receipt.output_digest)?;
        Ok(())
    }

    fn bound_details_result(
        &mut self,
        session_ref: &str,
        binding_hash: &str,
        result: serde_json::Value,
    ) -> Result<serde_json::Value, ControlPlaneError> {
        let bytes = serde_json::to_vec(&result).map_err(|error| ControlPlaneError {
            code: "details_artifact_encode_failed",
            detail: error.to_string(),
        })?;
        if bytes.len() <= DETAILS_INLINE_LIMIT {
            return Ok(result);
        }
        let reference = self.store_details(session_ref, binding_hash, None, bytes)?;
        Ok(serde_json::json!({
            "schema_version": "ags://schema/contract/v2/details-reference",
            "status": "details_available",
            "details_uri": reference.details_uri,
            "sha256": reference.sha256,
            "byte_length": reference.byte_length,
        }))
    }

    fn bound_apply_result(
        &mut self,
        session_ref: &str,
        binding_hash: &str,
        action_ref: &str,
        mut result: ApplyResult,
    ) -> Result<ApplyResult, ControlPlaneError> {
        let bytes = serde_json::to_vec(&result).map_err(|error| ControlPlaneError {
            code: "details_artifact_encode_failed",
            detail: error.to_string(),
        })?;
        if bytes.len() <= DETAILS_INLINE_LIMIT {
            return Ok(result);
        }
        result.receipt = None;
        result.details =
            Some(self.store_details(session_ref, binding_hash, Some(action_ref), bytes)?);
        Ok(result)
    }

    fn store_details(
        &mut self,
        session_ref: &str,
        binding_hash: &str,
        action_ref: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<DetailsReference, ControlPlaneError> {
        if bytes.len() > MAX_DETAILS_ARTIFACT_BYTES {
            return Err(ControlPlaneError {
                code: "details_artifact_too_large",
                detail: format!(
                    "details artifact exceeds {} bytes",
                    MAX_DETAILS_ARTIFACT_BYTES
                ),
            });
        }
        while self.details.len() >= MAX_DETAILS_RECORDS
            || self.details_total_bytes.saturating_add(bytes.len()) > MAX_DETAILS_TOTAL_BYTES
        {
            self.evict_oldest_details();
        }
        let digest = sha256(&bytes);
        self.next_id = self.next_id.saturating_add(1);
        let handle = short_id(
            "details-v2",
            &sha256(format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                self.sealing_key,
                session_ref,
                binding_hash,
                action_ref.unwrap_or("read-only"),
                digest,
                self.next_id
            )),
        );
        let artifact = ContentAddressedArtifactRef {
            uri: format!("ags://details/{handle}"),
            sha256: digest,
        };
        let byte_length = bytes.len();
        self.details.insert(
            artifact.uri.clone(),
            DetailsRecord {
                session_ref: session_ref.to_string(),
                binding_hash: binding_hash.to_string(),
                _action_ref: action_ref.map(str::to_string),
                artifact: artifact.clone(),
                bytes,
            },
        );
        self.details_total_bytes = self.details_total_bytes.saturating_add(byte_length);
        touch_lru(&mut self.details_lru, &artifact.uri);
        Ok(DetailsReference {
            details_uri: artifact.uri,
            sha256: artifact.sha256,
            byte_length: byte_length as u64,
        })
    }

    fn read_details_record(
        &mut self,
        session_ref: &str,
        binding: &SessionRecord,
        request: &DetailsReadRequest,
    ) -> Result<DetailsChunk, ControlPlaneError> {
        validate_hash("details.sha256", &request.artifact.sha256)?;
        if request.max_bytes == 0 || request.max_bytes > DETAILS_CHUNK_LIMIT {
            return Err(ControlPlaneError {
                code: "details_range_invalid",
                detail: format!("max_bytes must be between 1 and {DETAILS_CHUNK_LIMIT}"),
            });
        }
        let record = self
            .details
            .get(&request.artifact.uri)
            .ok_or_else(|| ControlPlaneError {
                code: "details_artifact_unknown",
                detail: "details URI does not name an immutable artifact in this daemon"
                    .to_string(),
            })?;
        let record = record.clone();
        touch_lru(&mut self.details_lru, &request.artifact.uri);
        if record.session_ref != session_ref || record.binding_hash != binding.binding_hash {
            return Err(ControlPlaneError {
                code: "details_artifact_cross_binding",
                detail:
                    "details artifact is sealed to a different authenticated session or binding"
                        .to_string(),
            });
        }
        if request.artifact != record.artifact {
            return Err(ControlPlaneError {
                code: "details_artifact_digest_mismatch",
                detail: "details digest does not match the immutable record".to_string(),
            });
        }
        let actual_digest = sha256(&record.bytes);
        if actual_digest != record.artifact.sha256 {
            return Err(ControlPlaneError {
                code: "details_artifact_integrity_failed",
                detail: "stored details bytes no longer match the sealed digest".to_string(),
            });
        }
        let offset = usize::try_from(request.offset).map_err(|_| ControlPlaneError {
            code: "details_range_invalid",
            detail: "offset does not fit this platform".to_string(),
        })?;
        if offset > record.bytes.len() {
            return Err(ControlPlaneError {
                code: "details_range_invalid",
                detail: "offset exceeds immutable artifact length".to_string(),
            });
        }
        let end = offset
            .saturating_add(request.max_bytes as usize)
            .min(record.bytes.len());
        Ok(DetailsChunk {
            artifact: record.artifact.clone(),
            offset: request.offset,
            next_offset: end as u64,
            byte_length: record.bytes.len() as u64,
            eof: end == record.bytes.len(),
            encoding: "hex".to_string(),
            data: hex_bytes(&record.bytes[offset..end]),
        })
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn remove_lru(queue: &mut VecDeque<String>, key: &str) {
    if let Some(index) = queue.iter().position(|candidate| candidate == key) {
        queue.remove(index);
    }
}

fn touch_lru(queue: &mut VecDeque<String>, key: &str) {
    remove_lru(queue, key);
    queue.push_back(key.to_string());
}

fn verify_host_write_artifacts(
    plan: &SealedPlan,
    receipt: &HostOutcomeReceipt,
    binding: &AuthenticatedBinding,
    require_postimages: bool,
) -> Result<(), ControlPlaneError> {
    if has_unexpected_writes(&plan.expected_write_paths, &receipt.observed_write_set) {
        return Err(ControlPlaneError {
            code: "host_outcome_unexpected_write_set",
            detail: "host receipt contains writes outside the sealed plan".to_string(),
        });
    }
    let observed = receipt
        .observed_write_set
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let artifact_paths = receipt
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if observed.len() != receipt.observed_write_set.len() {
        return Err(ControlPlaneError {
            code: "host_outcome_write_set_duplicate",
            detail: "host outcome write paths must be unique".to_string(),
        });
    }
    if !require_postimages {
        if receipt.artifacts.is_empty() {
            return Ok(());
        }
        return Err(ControlPlaneError {
            code: "host_test_artifacts_forbidden",
            detail:
                "TestReceipt proves directory-level output; transaction postimages are not accepted"
                    .to_string(),
        });
    }
    if artifact_paths.len() != receipt.artifacts.len() || observed != artifact_paths {
        return Err(ControlPlaneError {
            code: "host_outcome_artifact_set_mismatch",
            detail: "every unique observed write requires exactly one postimage artifact"
                .to_string(),
        });
    }
    let mut verified_postimage_bytes = 0_u64;
    for artifact in &receipt.artifacts {
        let path = Path::new(&artifact.path);
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(ControlPlaneError {
                code: "host_outcome_artifact_path_invalid",
                detail: artifact.path.clone(),
            });
        }
        let current = if matches!(artifact.state, HostArtifactState::Directory) {
            None
        } else {
            let remaining = MAX_HOST_POSTIMAGE_TOTAL_BYTES.saturating_sub(verified_postimage_bytes);
            descriptor_read_host_artifact(binding, path, MAX_HOST_POSTIMAGE_BYTES.min(remaining))?
        };
        if let Some(bytes) = &current {
            verified_postimage_bytes = verified_postimage_bytes.saturating_add(bytes.len() as u64);
        }
        match &artifact.state {
            HostArtifactState::Present { sha256: expected } => {
                if !ags_platform::is_sha256(expected) {
                    return Err(ControlPlaneError {
                        code: "host_outcome_artifact_digest_invalid",
                        detail: artifact.path.clone(),
                    });
                }
                let bytes = current.ok_or_else(|| ControlPlaneError {
                    code: "host_outcome_artifact_missing",
                    detail: artifact.path.clone(),
                })?;
                let actual = sha256(bytes);
                if &actual != expected {
                    return Err(ControlPlaneError {
                        code: "host_outcome_artifact_digest_mismatch",
                        detail: artifact.path.clone(),
                    });
                }
            }
            HostArtifactState::Absent if current.is_none() => {}
            HostArtifactState::Absent => {
                return Err(ControlPlaneError {
                    code: "host_outcome_artifact_expected_absent",
                    detail: artifact.path.clone(),
                });
            }
            HostArtifactState::Directory => {
                if !descriptor_host_artifact_is_directory(binding, path)? {
                    return Err(ControlPlaneError {
                        code: "host_outcome_artifact_not_directory",
                        detail: artifact.path.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn seal_host_physical_state(
    expected_write_paths: &[String],
    binding: &AuthenticatedBinding,
) -> Result<HostPhysicalSeal, ControlPlaneError> {
    if expected_write_paths.len() > MAX_HOST_PHYSICAL_TARGETS {
        return Err(ControlPlaneError {
            code: "host_before_state_budget_exceeded",
            detail: "target count exceeds physical seal budget".to_string(),
        });
    }
    let mut root_paths = Vec::<PathBuf>::new();
    let mut target_roots = Vec::with_capacity(expected_write_paths.len());
    let mut child_scopes = BTreeMap::<String, BTreeSet<String>>::new();
    let expected = expected_write_paths
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for parent in &expected {
        let mut children = BTreeSet::new();
        for child in &expected {
            if child.parent() == Some(parent.as_path()) {
                let name = child
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .ok_or_else(|| ControlPlaneError {
                        code: "host_before_state_path_invalid",
                        detail: child.display().to_string(),
                    })?;
                children.insert(name.to_string());
            }
        }
        child_scopes.insert(parent.display().to_string(), children);
    }
    for path in &expected {
        let root = binding
            .authorized_write_roots
            .iter()
            .filter(|root| path.strip_prefix(root).is_ok())
            .max_by_key(|root| root.components().count())
            .ok_or_else(|| ControlPlaneError {
                code: "host_before_state_outside_binding",
                detail: path.display().to_string(),
            })?;
        let root_index = root_paths
            .iter()
            .position(|candidate| candidate == root)
            .unwrap_or_else(|| {
                root_paths.push(root.clone());
                root_paths.len() - 1
            });
        target_roots.push((root_index, path.strip_prefix(root).unwrap().to_path_buf()));
    }
    let roots = root_paths
        .into_iter()
        .map(|path| {
            let descriptor = rustix::fs::open(
                &path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| ControlPlaneError {
                code: "host_before_state_root_invalid",
                detail: format!("{}: {error}", path.display()),
            })?;
            let stat = rustix::fs::fstat(&descriptor).map_err(|error| ControlPlaneError {
                code: "host_before_state_root_invalid",
                detail: format!("{}: {error}", path.display()),
            })?;
            Ok(SealedHostRoot {
                path,
                descriptor: std::sync::Arc::new(descriptor),
                device: stat.st_dev as u64,
                inode: stat.st_ino,
                mode: stat.st_mode as u32,
            })
        })
        .collect::<Result<Vec<_>, ControlPlaneError>>()?;
    if roots.len() > MAX_HOST_PHYSICAL_ROOTS {
        return Err(ControlPlaneError {
            code: "host_before_state_budget_exceeded",
            detail: "root count exceeds physical seal budget".to_string(),
        });
    }
    let expected_paths = expected
        .iter()
        .map(|path| path.display().to_string())
        .collect::<BTreeSet<_>>();
    let mut physical_budget = HostPhysicalBudget::default();
    let mut traversal_guards = BTreeSet::new();
    let mut directory_guards = BTreeMap::<(usize, PathBuf), HostDirectoryGuard>::new();
    for (root_index, relative) in &target_roots {
        let root = &roots[*root_index];
        let mut directory =
            rustix::io::dup(root.descriptor.as_ref()).map_err(|error| ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", root.path.display()),
            })?;
        let mut traversed = PathBuf::new();
        let mut missing_ancestor = false;
        for component in relative
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .components()
        {
            let std::path::Component::Normal(name) = component else {
                unreachable!("expected target relative path was already validated")
            };
            traversed.push(name);
            if missing_ancestor {
                let missing = root.path.join(&traversed).display().to_string();
                if !expected_paths.contains(&missing) {
                    return Err(ControlPlaneError {
                        code: "host_before_state_unplanned_missing_ancestor",
                        detail: missing,
                    });
                }
                continue;
            }
            directory = match rustix::fs::openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ) {
                Ok(directory) => directory,
                Err(error) if error == rustix::io::Errno::NOENT => {
                    let missing = root.path.join(&traversed).display().to_string();
                    if !expected_paths.contains(&missing) {
                        return Err(ControlPlaneError {
                            code: "host_before_state_unplanned_missing_ancestor",
                            detail: missing,
                        });
                    }
                    missing_ancestor = true;
                    continue;
                }
                Err(error) => {
                    return Err(ControlPlaneError {
                        code: "host_before_state_parent_invalid",
                        detail: format!("{}: {error}", root.path.join(&traversed).display()),
                    });
                }
            };
            let stat = rustix::fs::fstat(&directory).map_err(|error| ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", root.path.join(&traversed).display()),
            })?;
            traversal_guards.insert(HostTraversalGuard {
                root_index: *root_index,
                relative: traversed.clone(),
                device: stat.st_dev as u64,
                inode: stat.st_ino,
                mode: stat.st_mode as u32,
            });
        }
        if !missing_ancestor {
            let parent_relative = relative
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf();
            let child = relative
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .ok_or_else(|| ControlPlaneError {
                    code: "host_before_state_path_invalid",
                    detail: root.path.join(relative).display().to_string(),
                })?
                .to_string();
            let key = (*root_index, parent_relative.clone());
            if let Some(guard) = directory_guards.get_mut(&key) {
                guard.allowed_direct_children.insert(child);
            } else {
                directory_guards.insert(
                    key,
                    HostDirectoryGuard {
                        root_index: *root_index,
                        relative: parent_relative.clone(),
                        path: root.path.join(&parent_relative).display().to_string(),
                        baseline: Some(scan_host_directory_members(
                            root,
                            &parent_relative,
                            &mut physical_budget,
                            "host_before_state_budget_exceeded",
                        )?),
                        allowed_direct_children: BTreeSet::from([child]),
                    },
                );
            }
        }
    }
    let mut targets = Vec::with_capacity(expected.len());
    for (path, (root_index, relative)) in expected.into_iter().zip(target_roots) {
        let before = scan_host_target(
            &roots[root_index],
            &relative,
            &mut physical_budget,
            "host_before_state_budget_exceeded",
        )?;
        let allowed_direct_children = child_scopes
            .remove(&path.display().to_string())
            .unwrap_or_default();
        if !allowed_direct_children.is_empty()
            || matches!(before, HostTargetState::Directory { .. })
        {
            let baseline = match before {
                HostTargetState::Directory { .. } => Some(scan_host_directory_members(
                    &roots[root_index],
                    &relative,
                    &mut physical_budget,
                    "host_before_state_budget_exceeded",
                )?),
                HostTargetState::Missing => None,
                HostTargetState::File { .. } => {
                    return Err(ControlPlaneError {
                        code: "host_before_state_parent_invalid",
                        detail: format!("planned directory is a file: {}", path.display()),
                    });
                }
            };
            let key = (root_index, relative.clone());
            if let Some(guard) = directory_guards.get_mut(&key) {
                guard
                    .allowed_direct_children
                    .extend(allowed_direct_children);
            } else {
                directory_guards.insert(
                    key,
                    HostDirectoryGuard {
                        root_index,
                        relative: relative.clone(),
                        path: path.display().to_string(),
                        baseline,
                        allowed_direct_children,
                    },
                );
            }
        }
        targets.push(SealedHostTarget {
            path: path.display().to_string(),
            root_index,
            relative,
            before,
        });
    }
    let seal = HostPhysicalSeal {
        roots,
        traversal_guards: traversal_guards.into_iter().collect(),
        directory_guards: directory_guards.into_values().collect(),
        targets,
    };
    validate_host_physical_budget(&seal)?;
    Ok(seal)
}

#[cfg(unix)]
fn host_physical_seal_digest(seal: &HostPhysicalSeal) -> Result<String, ControlPlaneError> {
    serde_json::to_vec(&serde_json::json!({
        "roots": seal.roots.iter().map(|root| serde_json::json!({
            "path": root.path,
            "device": root.device,
            "inode": root.inode,
            "mode": root.mode,
        })).collect::<Vec<_>>(),
        "traversal_guards": seal.traversal_guards,
        "directory_guards": seal.directory_guards,
        "targets": seal.targets,
    }))
    .map(sha256)
    .map_err(|error| ControlPlaneError {
        code: "host_before_state_encode_failed",
        detail: error.to_string(),
    })
}

#[cfg(unix)]
fn validate_host_physical_budget(seal: &HostPhysicalSeal) -> Result<(), ControlPlaneError> {
    let member_count = seal
        .directory_guards
        .iter()
        .filter_map(|guard| guard.baseline.as_ref())
        .map(Vec::len)
        .sum::<usize>();
    let name_bytes = seal
        .directory_guards
        .iter()
        .filter_map(|guard| guard.baseline.as_ref())
        .flatten()
        .map(|member| member.name.len())
        .sum::<usize>();
    let member_bytes = seal
        .directory_guards
        .iter()
        .filter_map(|guard| guard.baseline.as_ref())
        .flatten()
        .filter(|member| member.kind == "file")
        .map(|member| member.size)
        .sum::<u64>();
    let target_bytes = seal
        .targets
        .iter()
        .filter_map(|target| match target.before {
            HostTargetState::File { size, .. } => Some(size),
            HostTargetState::Missing | HostTargetState::Directory { .. } => None,
        })
        .sum::<u64>();
    if seal.traversal_guards.len() > MAX_HOST_PHYSICAL_TRAVERSAL_GUARDS
        || seal.directory_guards.len() > MAX_HOST_PHYSICAL_DIRECTORY_GUARDS
        || member_count > MAX_HOST_PHYSICAL_TOTAL_MEMBERS
        || name_bytes > MAX_HOST_PHYSICAL_TOTAL_NAME_BYTES
        || member_bytes.saturating_add(target_bytes) > MAX_HOST_PHYSICAL_TOTAL_BYTES
    {
        return Err(ControlPlaneError {
            code: "host_before_state_budget_exceeded",
            detail: "aggregate physical seal budget exceeded".to_string(),
        });
    }
    Ok(())
}

#[derive(Default)]
#[cfg(unix)]
struct HostPhysicalBudget {
    members: usize,
    name_bytes: usize,
    bytes: u64,
}

#[cfg(unix)]
impl HostPhysicalBudget {
    fn reserve_member(&mut self, name: &str, code: &'static str) -> Result<(), ControlPlaneError> {
        if self.members >= MAX_HOST_PHYSICAL_TOTAL_MEMBERS
            || self.name_bytes.saturating_add(name.len()) > MAX_HOST_PHYSICAL_TOTAL_NAME_BYTES
        {
            return Err(ControlPlaneError {
                code,
                detail: "aggregate physical-state member budget exceeded".to_string(),
            });
        }
        self.members += 1;
        self.name_bytes += name.len();
        Ok(())
    }

    fn remaining_bytes(&self) -> u64 {
        MAX_HOST_PHYSICAL_TOTAL_BYTES.saturating_sub(self.bytes)
    }

    fn record_bytes(&mut self, bytes: u64, code: &'static str) -> Result<(), ControlPlaneError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| ControlPlaneError {
                code,
                detail: "aggregate physical-state byte budget overflow".to_string(),
            })?;
        if self.bytes > MAX_HOST_PHYSICAL_TOTAL_BYTES {
            return Err(ControlPlaneError {
                code,
                detail: "aggregate physical-state byte budget exceeded".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(unix)]
fn scan_host_target(
    root: &SealedHostRoot,
    relative: &Path,
    budget: &mut HostPhysicalBudget,
    budget_code: &'static str,
) -> Result<HostTargetState, ControlPlaneError> {
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ControlPlaneError {
            code: "host_before_state_path_invalid",
            detail: root.path.join(relative).display().to_string(),
        });
    }
    let mut parent =
        rustix::io::dup(root.descriptor.as_ref()).map_err(|error| ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", root.path.display()),
        })?;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            unreachable!("relative path components were validated")
        };
        parent = match rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(parent) => parent,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(HostTargetState::Missing),
            Err(error) => {
                return Err(ControlPlaneError {
                    code: "host_before_state_parent_invalid",
                    detail: format!("{}: {error}", root.path.join(relative).display()),
                });
            }
        };
    }
    let name = relative
        .file_name()
        .expect("validated non-empty relative path");
    let stat = match rustix::fs::statat(&parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => stat,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(HostTargetState::Missing),
        Err(error) => {
            return Err(ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", root.path.join(relative).display()),
            });
        }
    };
    let preopen_type = FileType::from_raw_mode(stat.st_mode);
    let (flags, expected_directory) = if preopen_type.is_dir() {
        (
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            true,
        )
    } else if preopen_type.is_file() {
        (
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            false,
        )
    } else {
        return Err(ControlPlaneError {
            code: "host_before_state_type_invalid",
            detail: root.path.join(relative).display().to_string(),
        });
    };
    let descriptor = rustix::fs::openat(&parent, name, flags, Mode::empty()).map_err(|error| {
        ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", root.path.join(relative).display()),
        }
    })?;
    let opened = rustix::fs::fstat(&descriptor).map_err(|error| ControlPlaneError {
        code: "host_before_state_read_failed",
        detail: format!("{}: {error}", root.path.join(relative).display()),
    })?;
    let opened_type = FileType::from_raw_mode(opened.st_mode);
    if expected_directory != opened_type.is_dir() || (!expected_directory && !opened_type.is_file())
    {
        return Err(ControlPlaneError {
            code: "host_before_state_type_changed",
            detail: root.path.join(relative).display().to_string(),
        });
    }
    if expected_directory {
        Ok(HostTargetState::Directory {
            device: opened.st_dev as u64,
            inode: opened.st_ino,
            mode: opened.st_mode as u32,
        })
    } else {
        let path = root.path.join(relative);
        let remaining = budget.remaining_bytes().min(MAX_HOST_POSTIMAGE_BYTES);
        let stable = read_regular_fd(&descriptor, remaining, || {
            #[cfg(all(test, unix))]
            tests::run_stable_read_same_inode_rewrite_test_hook(&path);
        })
        .map_err(|error| match error {
            StableReadError::TooLarge => ControlPlaneError {
                code: budget_code,
                detail: path.display().to_string(),
            },
            StableReadError::Changed => ControlPlaneError {
                code: "host_before_state_file_changed_during_read",
                detail: path.display().to_string(),
            },
            StableReadError::NotRegular => ControlPlaneError {
                code: "host_before_state_type_changed",
                detail: path.display().to_string(),
            },
            StableReadError::Io(error) => ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", path.display()),
            },
        })?;
        budget.record_bytes(stable.bytes.len() as u64, budget_code)?;
        Ok(HostTargetState::File {
            device: stable.stable_stat.device,
            inode: stable.stable_stat.inode,
            mode: stable.stable_stat.mode,
            size: stable.stable_stat.size,
            sha256: sha256(stable.bytes),
        })
    }
}

#[cfg(unix)]
fn scan_host_directory_members(
    root: &SealedHostRoot,
    relative: &Path,
    budget: &mut HostPhysicalBudget,
    budget_code: &'static str,
) -> Result<Vec<HostDirectMember>, ControlPlaneError> {
    let mut directory =
        rustix::io::dup(root.descriptor.as_ref()).map_err(|error| ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", root.path.display()),
        })?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(ControlPlaneError {
                code: "host_before_state_path_invalid",
                detail: root.path.join(relative).display().to_string(),
            });
        };
        directory = rustix::fs::openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", root.path.join(relative).display()),
        })?;
    }
    let opened = rustix::fs::fstat(&directory).map_err(|error| ControlPlaneError {
        code: "host_before_state_read_failed",
        detail: format!("{}: {error}", root.path.join(relative).display()),
    })?;
    if !FileType::from_raw_mode(opened.st_mode).is_dir() {
        return Err(ControlPlaneError {
            code: "host_before_state_type_changed",
            detail: root.path.join(relative).display().to_string(),
        });
    }
    let mut direct_members = Vec::new();
    let mut name_bytes = 0usize;
    let mut member_bytes = 0u64;
    for entry in rustix::fs::Dir::read_from(&directory).map_err(|error| ControlPlaneError {
        code: "host_before_state_read_failed",
        detail: format!("{}: {error}", root.path.join(relative).display()),
    })? {
        let entry = entry.map_err(|error| ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", root.path.join(relative).display()),
        })?;
        let member = entry
            .file_name()
            .to_str()
            .map_err(|error| ControlPlaneError {
                code: "host_before_state_name_invalid",
                detail: error.to_string(),
            })?
            .to_string();
        if member == "." || member == ".." {
            continue;
        }
        if direct_members.len() >= MAX_HOST_PHYSICAL_DIRECTORY_ENTRIES {
            return Err(ControlPlaneError {
                code: "host_before_state_budget_exceeded",
                detail: "direct member count exceeds physical seal budget".to_string(),
            });
        }
        name_bytes = name_bytes.saturating_add(member.len());
        if name_bytes > MAX_HOST_PHYSICAL_NAME_BYTES {
            return Err(ControlPlaneError {
                code: "host_before_state_budget_exceeded",
                detail: "direct member names exceed physical seal budget".to_string(),
            });
        }
        budget.reserve_member(&member, budget_code)?;
        direct_members.push(scan_host_direct_member(
            &directory,
            &member,
            &mut member_bytes,
            &root.path.join(relative),
            budget,
            budget_code,
        )?);
    }
    direct_members.sort();
    Ok(direct_members)
}

#[cfg(unix)]
fn scan_host_direct_member(
    directory: &OwnedFd,
    name: &str,
    total_bytes: &mut u64,
    display_directory: &Path,
    budget: &mut HostPhysicalBudget,
    budget_code: &'static str,
) -> Result<HostDirectMember, ControlPlaneError> {
    #[cfg(test)]
    tests::note_physical_direct_member_scan();
    let stat = rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).map_err(
        |error| ControlPlaneError {
            code: "host_before_state_read_failed",
            detail: format!("{}: {error}", display_directory.join(name).display()),
        },
    )?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let (kind, flags) = if file_type.is_dir() {
        (
            "directory",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        )
    } else if file_type.is_file() {
        (
            "file",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        )
    } else {
        return Err(ControlPlaneError {
            code: "host_before_state_type_invalid",
            detail: display_directory.join(name).display().to_string(),
        });
    };
    let descriptor =
        rustix::fs::openat(directory, name, flags, Mode::empty()).map_err(|error| {
            ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", display_directory.join(name).display()),
            }
        })?;
    let opened = rustix::fs::fstat(&descriptor).map_err(|error| ControlPlaneError {
        code: "host_before_state_read_failed",
        detail: format!("{}: {error}", display_directory.join(name).display()),
    })?;
    let opened_type = FileType::from_raw_mode(opened.st_mode);
    if (kind == "directory" && !opened_type.is_dir()) || (kind == "file" && !opened_type.is_file())
    {
        return Err(ControlPlaneError {
            code: "host_before_state_type_changed",
            detail: display_directory.join(name).display().to_string(),
        });
    }
    let (device, inode, mode, size, sha256) = if kind == "file" {
        let path = display_directory.join(name);
        let remaining = MAX_HOST_PHYSICAL_MEMBER_BYTES
            .saturating_sub(*total_bytes)
            .min(budget.remaining_bytes());
        let stable = read_regular_fd(&descriptor, remaining, || {
            #[cfg(all(test, unix))]
            tests::run_stable_read_same_inode_rewrite_test_hook(&path);
        })
        .map_err(|error| match error {
            StableReadError::TooLarge => ControlPlaneError {
                code: budget_code,
                detail: "direct member bytes exceed physical-state budget".to_string(),
            },
            StableReadError::Changed => ControlPlaneError {
                code: "host_before_state_file_changed_during_read",
                detail: path.display().to_string(),
            },
            StableReadError::NotRegular => ControlPlaneError {
                code: "host_before_state_type_changed",
                detail: path.display().to_string(),
            },
            StableReadError::Io(error) => ControlPlaneError {
                code: "host_before_state_read_failed",
                detail: format!("{}: {error}", path.display()),
            },
        })?;
        *total_bytes = (*total_bytes).saturating_add(stable.bytes.len() as u64);
        budget.record_bytes(stable.bytes.len() as u64, budget_code)?;
        (
            stable.stable_stat.device,
            stable.stable_stat.inode,
            stable.stable_stat.mode,
            stable.stable_stat.size,
            Some(sha256(stable.bytes)),
        )
    } else {
        (
            opened.st_dev as u64,
            opened.st_ino,
            opened.st_mode as u32,
            opened.st_size as u64,
            None,
        )
    };
    Ok(HostDirectMember {
        name: name.to_string(),
        kind: kind.to_string(),
        device,
        inode,
        mode,
        size,
        sha256,
    })
}

pub(crate) enum HostTerminalDelta {
    #[cfg_attr(not(unix), allow(dead_code))]
    Exact { changed: Vec<String> },
    Risk {
        known_residuals: Vec<String>,
        unexpected: Vec<String>,
        proof_error: ControlPlaneError,
    },
}

fn host_terminal_delta_paths(delta: HostTerminalDelta) -> (Vec<String>, Option<ControlPlaneError>) {
    match delta {
        HostTerminalDelta::Exact { mut changed } => {
            changed.sort();
            changed.dedup();
            (changed, None)
        }
        HostTerminalDelta::Risk {
            mut known_residuals,
            unexpected,
            proof_error,
        } => {
            known_residuals.extend(unexpected);
            known_residuals.sort();
            known_residuals.dedup();
            if known_residuals.is_empty() {
                known_residuals.push(format!(
                    "ags://unprovable-host-delta/{}",
                    proof_error.code.replace('_', "-")
                ));
            }
            (known_residuals, Some(proof_error))
        }
    }
}

#[cfg(unix)]
fn verify_host_physical_delta(
    seal: &HostPhysicalSeal,
    receipt: &HostOutcomeReceipt,
) -> HostTerminalDelta {
    let mut changed = BTreeSet::new();
    match verify_host_physical_delta_inner(seal, receipt, &mut changed) {
        Ok(()) => HostTerminalDelta::Exact {
            changed: changed.into_iter().collect(),
        },
        Err(proof_error) => {
            let unexpected = if Path::new(&proof_error.detail).is_absolute() {
                vec![proof_error.detail.clone()]
            } else {
                Vec::new()
            };
            HostTerminalDelta::Risk {
                known_residuals: changed.into_iter().collect(),
                unexpected,
                proof_error,
            }
        }
    }
}

#[cfg(unix)]
fn verify_host_physical_delta_inner(
    seal: &HostPhysicalSeal,
    receipt: &HostOutcomeReceipt,
    changed: &mut BTreeSet<String>,
) -> Result<(), ControlPlaneError> {
    let mut physical_budget = HostPhysicalBudget::default();
    verify_sealed_host_root_paths(seal)?;
    for guard in &seal.traversal_guards {
        let root = &seal.roots[guard.root_index];
        let mut directory =
            rustix::io::dup(root.descriptor.as_ref()).map_err(|error| ControlPlaneError {
                code: "host_outcome_traversal_guard_changed",
                detail: format!("{}: {error}", root.path.display()),
            })?;
        for component in guard.relative.components() {
            let std::path::Component::Normal(name) = component else {
                unreachable!("sealed traversal guard is normalized")
            };
            directory = rustix::fs::openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| ControlPlaneError {
                code: "host_outcome_traversal_guard_changed",
                detail: format!("{}: {error}", root.path.join(&guard.relative).display()),
            })?;
        }
        let stat = rustix::fs::fstat(&directory).map_err(|error| ControlPlaneError {
            code: "host_outcome_traversal_guard_changed",
            detail: format!("{}: {error}", root.path.join(&guard.relative).display()),
        })?;
        if stat.st_dev as u64 != guard.device
            || stat.st_ino != guard.inode
            || stat.st_mode as u32 != guard.mode
        {
            return Err(ControlPlaneError {
                code: "host_outcome_traversal_guard_changed",
                detail: root.path.join(&guard.relative).display().to_string(),
            });
        }
    }
    for target in &seal.targets {
        let after = scan_host_target(
            &seal.roots[target.root_index],
            &target.relative,
            &mut physical_budget,
            "host_outcome_physical_budget_exceeded",
        )?;
        if target.before != after {
            changed.insert(target.path.clone());
        }
    }
    for guard in &seal.directory_guards {
        let state = if guard.relative.as_os_str().is_empty() {
            HostTargetState::Directory {
                device: seal.roots[guard.root_index].device,
                inode: seal.roots[guard.root_index].inode,
                mode: seal.roots[guard.root_index].mode,
            }
        } else {
            scan_host_target(
                &seal.roots[guard.root_index],
                &guard.relative,
                &mut physical_budget,
                "host_outcome_physical_budget_exceeded",
            )?
        };
        let after = match state {
            HostTargetState::Directory { .. } => scan_host_directory_members(
                &seal.roots[guard.root_index],
                &guard.relative,
                &mut physical_budget,
                "host_outcome_physical_budget_exceeded",
            )?,
            HostTargetState::Missing => Vec::new(),
            HostTargetState::File { .. } => {
                return Err(ControlPlaneError {
                    code: "host_outcome_directory_guard_changed",
                    detail: guard.path.clone(),
                });
            }
        };
        let before = guard
            .baseline
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|member| (member.name.as_str(), member))
            .collect::<BTreeMap<_, _>>();
        let after = after
            .iter()
            .map(|member| (member.name.as_str(), member))
            .collect::<BTreeMap<_, _>>();
        let names = before
            .keys()
            .chain(after.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        if let Some(unexpected) = names.into_iter().find(|name| {
            before.get(name) != after.get(name) && !guard.allowed_direct_children.contains(*name)
        }) {
            let unexpected_path = Path::new(&guard.path)
                .join(unexpected)
                .display()
                .to_string();
            changed.insert(unexpected_path.clone());
            return Err(ControlPlaneError {
                code: "host_outcome_unreported_directory_member",
                detail: unexpected_path,
            });
        };
    }
    #[cfg(all(test, unix))]
    tests::run_root_after_scan_test_hook();
    verify_sealed_host_root_paths(seal)?;
    let reported = receipt
        .observed_write_set
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if *changed != reported || reported.len() != receipt.observed_write_set.len() {
        return Err(ControlPlaneError {
            code: "host_outcome_physical_delta_mismatch",
            detail: format!("physical {changed:?}, reported {reported:?}"),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn verify_sealed_host_root_paths(seal: &HostPhysicalSeal) -> Result<(), ControlPlaneError> {
    for root in &seal.roots {
        let reopened = rustix::fs::open(
            &root.path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .and_then(|descriptor| rustix::fs::fstat(&descriptor))
        .map_err(|error| ControlPlaneError {
            code: "host_outcome_root_binding_changed",
            detail: format!("{}: {error}", root.path.display()),
        })?;
        if reopened.st_dev as u64 != root.device
            || reopened.st_ino != root.inode
            || reopened.st_mode as u32 != root.mode
        {
            return Err(ControlPlaneError {
                code: "host_outcome_root_binding_changed",
                detail: root.path.display().to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn descriptor_host_artifact_is_directory(
    binding: &AuthenticatedBinding,
    path: &Path,
) -> Result<bool, ControlPlaneError> {
    let (root, relative) = binding
        .authorized_write_roots
        .iter()
        .filter_map(|root| {
            path.strip_prefix(root)
                .ok()
                .map(|relative| (root, relative))
        })
        .max_by_key(|(root, _)| root.components().count())
        .ok_or_else(|| ControlPlaneError {
            code: "host_outcome_artifact_outside_binding",
            detail: path.display().to_string(),
        })?;
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ControlPlaneError {
            code: "host_outcome_artifact_path_invalid",
            detail: path.display().to_string(),
        });
    }
    let mut parent = rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ControlPlaneError {
        code: "host_outcome_artifact_root_invalid",
        detail: format!("{}: {error}", root.display()),
    })?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(ControlPlaneError {
                code: "host_outcome_artifact_path_invalid",
                detail: path.display().to_string(),
            });
        };
        parent = match rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(false),
            Err(error) => {
                return Err(ControlPlaneError {
                    code: "host_outcome_artifact_not_directory",
                    detail: format!("{}: {error}", path.display()),
                });
            }
        };
    }
    Ok(true)
}

fn verify_host_evidence(
    evidence: Option<&HostOutcomeEvidence>,
) -> Result<Option<VerifiedHostEvidence>, ControlPlaneError> {
    let Some(evidence) = evidence else {
        return Ok(None);
    };
    validate_hash("host_outcome.evidence.sha256", &evidence.artifact.sha256)?;
    if evidence.artifact.uri.trim().is_empty() {
        return Err(ControlPlaneError {
            code: "host_outcome_evidence_uri_invalid",
            detail: "evidence artifact URI must not be empty".to_string(),
        });
    }
    if evidence.content_hex.len() > MAX_HOST_EVIDENCE_BYTES.saturating_mul(2) {
        return Err(ControlPlaneError {
            code: "host_outcome_evidence_too_large",
            detail: format!("evidence exceeds {} bytes", MAX_HOST_EVIDENCE_BYTES),
        });
    }
    let bytes = decode_hex_evidence(&evidence.content_hex)?;
    if sha256(&bytes) != evidence.artifact.sha256 {
        return Err(ControlPlaneError {
            code: "host_outcome_evidence_digest_mismatch",
            detail: evidence.artifact.uri.clone(),
        });
    }
    Ok(Some(VerifiedHostEvidence {
        kind: evidence.kind,
        artifact: evidence.artifact.clone(),
        bytes,
    }))
}

fn decode_hex_evidence(value: &str) -> Result<Vec<u8>, ControlPlaneError> {
    if !value.len().is_multiple_of(2) {
        return Err(ControlPlaneError {
            code: "host_outcome_evidence_invalid",
            detail: "hex evidence has odd length".to_string(),
        });
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            let high = digit(pair[0]).ok_or_else(|| ControlPlaneError {
                code: "host_outcome_evidence_invalid",
                detail: "invalid hex evidence".to_string(),
            })?;
            let low = digit(pair[1]).ok_or_else(|| ControlPlaneError {
                code: "host_outcome_evidence_invalid",
                detail: "invalid hex evidence".to_string(),
            })?;
            Ok((high << 4) | low)
        })
        .collect()
}

#[cfg(unix)]
fn descriptor_read_host_artifact(
    binding: &AuthenticatedBinding,
    path: &Path,
    limit: u64,
) -> Result<Option<Vec<u8>>, ControlPlaneError> {
    let (root, relative) = binding
        .authorized_write_roots
        .iter()
        .filter_map(|root| {
            path.strip_prefix(root)
                .ok()
                .map(|relative| (root, relative))
        })
        .max_by_key(|(root, _)| root.components().count())
        .ok_or_else(|| ControlPlaneError {
            code: "host_outcome_artifact_outside_binding",
            detail: path.display().to_string(),
        })?;
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ControlPlaneError {
            code: "host_outcome_artifact_path_invalid",
            detail: path.display().to_string(),
        });
    }
    let mut parent = rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| ControlPlaneError {
        code: "host_outcome_artifact_root_invalid",
        detail: format!("{}: {error}", root.display()),
    })?;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            return Err(ControlPlaneError {
                code: "host_outcome_artifact_path_invalid",
                detail: path.display().to_string(),
            });
        };
        parent = match rustix::fs::openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(parent) => parent,
            Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
            Err(error) => {
                return Err(ControlPlaneError {
                    code: "host_outcome_artifact_parent_invalid",
                    detail: format!("{}: {error}", path.display()),
                });
            }
        };
    }
    let name = relative.file_name().ok_or_else(|| ControlPlaneError {
        code: "host_outcome_artifact_path_invalid",
        detail: path.display().to_string(),
    })?;
    let file: OwnedFd = match rustix::fs::openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => file,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(None),
        Err(error) => {
            return Err(ControlPlaneError {
                code: "host_outcome_artifact_read_failed",
                detail: format!("{}: {error}", path.display()),
            });
        }
    };
    let stable = read_regular_fd(&file, limit, || {
        #[cfg(all(test, unix))]
        tests::run_stable_read_same_inode_rewrite_test_hook(path);
    })
    .map_err(|error| match error {
        StableReadError::NotRegular => ControlPlaneError {
            code: "host_outcome_artifact_not_regular",
            detail: path.display().to_string(),
        },
        StableReadError::TooLarge => ControlPlaneError {
            code: "host_outcome_artifact_too_large",
            detail: path.display().to_string(),
        },
        StableReadError::Changed => ControlPlaneError {
            code: "host_outcome_artifact_changed_during_read",
            detail: path.display().to_string(),
        },
        StableReadError::Io(error) => ControlPlaneError {
            code: "host_outcome_artifact_read_failed",
            detail: format!("{}: {error}", path.display()),
        },
    })?;
    Ok(Some(stable.bytes))
}

#[cfg(unix)]
fn root_seal(path: &PathBuf) -> String {
    let identity = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .and_then(|descriptor| rustix::fs::fstat(&descriptor))
    .map(|stat| format!("{}:{}", stat.st_dev as u64, stat.st_ino))
    .unwrap_or_else(|error| format!("unavailable:{error}"));
    format!("{}|{identity}", path.display())
}

#[cfg(not(unix))]
fn root_seal(path: &PathBuf) -> String {
    format!(
        "{}|{}",
        path.display(),
        platform_io::DESCRIPTOR_SEMANTICS_UNAVAILABLE
    )
}

fn validate_binding(binding: &AuthenticatedBinding) -> Result<(), ControlPlaneError> {
    for (field, value) in [
        ("host_id", binding.host_id.as_str()),
        ("workspace_identity", binding.workspace_identity.as_str()),
        ("project_facts_hash", binding.project_facts_hash.as_str()),
        ("registry_key", binding.registry_key.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(ControlPlaneError {
                code: "binding_invalid",
                detail: format!("{field} must not be empty"),
            });
        }
    }
    validate_hash("project_facts_hash", &binding.project_facts_hash)?;
    if binding.authorized_write_roots.is_empty()
        || binding
            .authorized_write_roots
            .iter()
            .any(|root| !root.is_absolute())
    {
        return Err(ControlPlaneError {
            code: "binding_invalid",
            detail: "binding requires absolute authenticated write roots".to_string(),
        });
    }
    if binding
        .authorized_write_roots
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != binding.authorized_write_roots.len()
    {
        return Err(ControlPlaneError {
            code: "binding_invalid",
            detail: "authenticated write roots must be unique".to_string(),
        });
    }
    for root in &binding.authorized_write_roots {
        let metadata = std::fs::symlink_metadata(root).map_err(|error| ControlPlaneError {
            code: "binding_invalid",
            detail: format!("authorized root {}: {error}", root.display()),
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ControlPlaneError {
                code: "binding_invalid",
                detail: format!(
                    "authorized write root must be a real directory: {}",
                    root.display()
                ),
            });
        }
        let canonical = root.canonicalize().map_err(|error| ControlPlaneError {
            code: "binding_invalid",
            detail: format!("authorized root {}: {error}", root.display()),
        })?;
        if canonical != *root {
            return Err(ControlPlaneError {
                code: "binding_invalid",
                detail: format!(
                    "authorized write root must already be canonical: {}",
                    root.display()
                ),
            });
        }
    }
    let canonical = binding
        .canonical_workspace
        .canonicalize()
        .map_err(|error| ControlPlaneError {
            code: "workspace_invalid",
            detail: error.to_string(),
        })?;
    if canonical != binding.canonical_workspace {
        return Err(ControlPlaneError {
            code: "workspace_not_canonical",
            detail: "binding workspace must already be canonical".to_string(),
        });
    }
    match &binding.surface {
        BindingSurface::Mcp {
            connection_id,
            authenticated_session,
        } if connection_id.is_empty() || authenticated_session.is_empty() => {
            Err(ControlPlaneError {
                code: "binding_invalid",
                detail: "MCP binding requires connection and authenticated session".to_string(),
            })
        }
        BindingSurface::Cli {
            workspace_service_identity,
        } if workspace_service_identity.is_empty() => Err(ControlPlaneError {
            code: "binding_invalid",
            detail: "CLI binding requires authenticated workspace-service identity".to_string(),
        }),
        _ => Ok(()),
    }
}

fn validate_hash(field: &'static str, value: &str) -> Result<(), ControlPlaneError> {
    if ags_platform::is_sha256(value) {
        Ok(())
    } else {
        Err(ControlPlaneError {
            code: "binding_invalid",
            detail: format!("{field} must be sha256:<hex>"),
        })
    }
}

#[derive(Serialize)]
struct HostExecutionInstructionDigestPreimage<'a> {
    domain: &'static str,
    schema_version: &'a str,
    action_ref: &'a str,
    binding_hash: &'a str,
    plan_hash: &'a str,
    policy_hash: &'a str,
    sealed_action_digest: &'a str,
    action: &'a HostExecutionAction,
}

fn canonical_host_execution_instruction_digest(
    instruction: &HostExecutionInstruction,
    sealed_action_digest: &str,
) -> Result<String, ControlPlaneError> {
    validate_hash("host_instruction.binding_hash", &instruction.binding_hash)?;
    validate_hash("host_instruction.plan_hash", &instruction.plan_hash)?;
    validate_hash("host_instruction.policy_hash", &instruction.policy_hash)?;
    validate_hash(
        "host_instruction.sealed_action_digest",
        sealed_action_digest,
    )?;
    let bytes = serde_json::to_vec(&HostExecutionInstructionDigestPreimage {
        domain: "ags-control-plane/host-execution-instruction/v2",
        schema_version: &instruction.schema_version,
        action_ref: &instruction.action_ref,
        binding_hash: &instruction.binding_hash,
        plan_hash: &instruction.plan_hash,
        policy_hash: &instruction.policy_hash,
        sealed_action_digest,
        action: &instruction.action,
    })
    .map_err(|error| ControlPlaneError {
        code: "host_execution_instruction_encode_failed",
        detail: error.to_string(),
    })?;
    Ok(sha256(bytes))
}

fn validate_host_execution_action(
    action: &HostExecutionAction,
    plan: &SealedPlan,
) -> Result<(), ControlPlaneError> {
    let invalid = |detail: String| ControlPlaneError {
        code: "host_execution_instruction_invalid",
        detail,
    };
    let expected_write_paths = match action {
        HostExecutionAction::Command {
            profile: _,
            program,
            argv,
            cwd,
            env,
            timeout_ms,
            allowed_write_paths,
        } => {
            if program.is_empty()
                || program.len() > MAX_EFFECT_PATH_BYTES
                || argv.len() > MAX_EFFECT_OBSERVED_WRITES
                || env.len() > MAX_EFFECT_OBSERVED_WRITES
                || *timeout_ms == 0
                || !cwd.is_absolute()
                || argv.iter().any(|value| value.len() > MAX_EFFECT_PATH_BYTES)
                || env.iter().any(|(key, value)| {
                    key.is_empty()
                        || key.len() > MAX_EFFECT_PATH_BYTES
                        || value.len() > MAX_EFFECT_PATH_BYTES
                })
            {
                return Err(invalid(
                    "command fields exceed the closed instruction budget".to_string(),
                ));
            }
            allowed_write_paths
        }
        HostExecutionAction::RuntimeUpdate {
            channel,
            target_version,
            candidate_directory,
            release_directory,
            manifest,
            tree_digest,
            members,
            expected_write_paths,
        } => {
            if channel.is_empty()
                || channel.len() > MAX_EFFECT_PATH_BYTES
                || target_version
                    .as_ref()
                    .is_some_and(|value| value.is_empty() || value.len() > MAX_EFFECT_PATH_BYTES)
                || !candidate_directory.is_absolute()
                || !release_directory.is_absolute()
                || candidate_directory == release_directory
                || members.len() > MAX_EFFECT_OBSERVED_WRITES
                || manifest.name.is_empty()
                || members.iter().any(|member| member.name.is_empty())
            {
                return Err(invalid(
                    "runtime update fields are invalid or oversized".to_string(),
                ));
            }
            validate_hash(
                "host_instruction.runtime_update.manifest.sha256",
                &manifest.sha256,
            )?;
            validate_hash("host_instruction.runtime_update.tree_digest", tree_digest)?;
            for member in members {
                validate_hash(
                    "host_instruction.runtime_update.member.sha256",
                    &member.sha256,
                )?;
            }
            expected_write_paths
        }
        HostExecutionAction::ArchiveClosures {
            event_id,
            receipt_ids,
            pointer_paths,
            expected_write_paths,
        } => {
            if event_id.is_empty()
                || event_id.len() > MAX_EFFECT_PATH_BYTES
                || receipt_ids.is_empty()
                || receipt_ids.len() > MAX_EFFECT_OBSERVED_WRITES
                || pointer_paths.len() != receipt_ids.len()
                || receipt_ids.iter().any(|receipt_id| {
                    receipt_id.is_empty() || receipt_id.len() > MAX_EFFECT_PATH_BYTES
                })
                || pointer_paths.iter().any(|path| !path.is_absolute())
            {
                return Err(invalid(
                    "closure archive fields are invalid or oversized".to_string(),
                ));
            }
            expected_write_paths
        }
    };
    if expected_write_paths.len() > MAX_EFFECT_OBSERVED_WRITES
        || expected_write_paths
            .iter()
            .any(|path| !path.is_absolute() || path.as_os_str().len() > MAX_EFFECT_PATH_BYTES)
        || expected_write_paths
            .iter()
            .map(|path| path.as_os_str().len())
            .sum::<usize>()
            > MAX_EFFECT_TOTAL_PATH_BYTES
    {
        return Err(invalid(
            "expected write paths exceed the instruction budget".to_string(),
        ));
    }
    let actual = expected_write_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if actual != plan.expected_write_paths {
        return Err(invalid(
            "typed action write paths differ from the sealed plan".to_string(),
        ));
    }
    Ok(())
}

fn verify_operation_workspace(
    operation: &OperationRequest,
    workspace: &Path,
) -> Result<(), ControlPlaneError> {
    if let Some(explicit) = &operation.context().workspace {
        let explicit = Path::new(explicit)
            .canonicalize()
            .map_err(|error| ControlPlaneError {
                code: "operation_workspace_invalid",
                detail: error.to_string(),
            })?;
        if explicit != workspace {
            return Err(ControlPlaneError {
                code: "operation_workspace_cross_binding",
                detail: "operation workspace differs from the authenticated binding".to_string(),
            });
        }
    }
    Ok(())
}

// These are the complete independently auditable semantic plan inputs. The
// transport binding is deliberately kept beside the semantic seal instead of
// inside it: action_ref and every apply path bind `binding_hash` separately,
// while an identical typed request/policy/domain plan has one stable plan hash
// across CLI and MCP adapters.
#[allow(clippy::too_many_arguments)]
fn plan_hash(
    name: OperationName,
    kind: OperationKind,
    binding: &SessionRecord,
    payload_hash: &str,
    steps: &[PlanStep],
    writes: &[String],
    recoverability: Recoverability,
    execution: Option<&CommandSpec>,
    verification: Option<&VerificationSpec>,
    action_digest: &str,
) -> String {
    sha256(
        serde_json::to_vec(&(
            "ags-sealed-plan-v2\n",
            CONTRACT_SCHEMA_VERSION,
            name,
            kind,
            &binding.policy_hash,
            payload_hash,
            action_digest,
            steps,
            writes,
            verification,
            recoverability,
            execution,
        ))
        .expect("sealed plan fields are JSON serializable"),
    )
}

fn recompute_sealed_plan_hash(plan: &SealedPlan, binding: &SessionRecord) -> String {
    plan_hash(
        plan.operation,
        plan.kind,
        binding,
        &plan.payload_hash,
        &plan.steps,
        &plan.expected_write_paths,
        plan.recoverability,
        plan.execution.as_ref(),
        Some(&plan.verification),
        &plan.action_digest,
    )
}

fn validate_domain_plan(
    kind: OperationKind,
    plan: &DomainPlan,
    binding: &AuthenticatedBinding,
) -> Result<(), ControlPlaneError> {
    if plan.steps.is_empty() {
        return Err(ControlPlaneError {
            code: "domain_plan_invalid",
            detail: "effectful plan must contain at least one typed step".to_string(),
        });
    }
    validate_hash("action_digest", &plan.action_digest)?;
    if plan.expected_write_paths.len() > MAX_EFFECT_OBSERVED_WRITES
        || plan
            .expected_write_paths
            .iter()
            .any(|path| path.len() > MAX_EFFECT_PATH_BYTES)
        || plan
            .expected_write_paths
            .iter()
            .map(String::len)
            .sum::<usize>()
            > MAX_EFFECT_TOTAL_PATH_BYTES
    {
        return Err(ControlPlaneError {
            code: "domain_plan_invalid",
            detail: "expected write set exceeds terminal result budget".to_string(),
        });
    }
    if plan.verification.checks.is_empty() {
        return Err(ControlPlaneError {
            code: "domain_plan_invalid",
            detail: "effectful plan must define verification".to_string(),
        });
    }
    for path in &plan.expected_write_paths {
        let path = Path::new(path);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(ControlPlaneError {
                code: "domain_plan_invalid",
                detail: format!(
                    "expected write path must be absolute and contained: {}",
                    path.display()
                ),
            });
        }
        require_authorized_containment(path, &binding.authorized_write_roots)?;
    }
    match (kind, plan.recoverability) {
        (
            OperationKind::Transaction,
            Recoverability::Transactional | Recoverability::BeforeEffectOnly,
        )
        | (OperationKind::LocalExecution, Recoverability::SourcePreserving)
        | (OperationKind::HostDelegated, Recoverability::NotApplicable) => Ok(()),
        _ => Err(ControlPlaneError {
            code: "domain_plan_invalid",
            detail: "recoverability does not match OperationKind".to_string(),
        }),
    }?;
    if (kind == OperationKind::LocalExecution && plan.execution.is_none())
        || (matches!(kind, OperationKind::ReadOnly | OperationKind::Transaction)
            && plan.execution.is_some())
    {
        return Err(ControlPlaneError {
            code: "domain_plan_invalid",
            detail: "LocalExecution requires and HostDelegated may bind one derived CommandSpec"
                .to_string(),
        });
    }
    Ok(())
}

fn require_authorized_containment(
    path: &Path,
    authorized_roots: &[PathBuf],
) -> Result<(), ControlPlaneError> {
    // The exact target may itself be a link that a typed domain operation
    // removes without following (for example canonical Skill deactivation).
    // Every ancestor remains link-free, and effect/postimage adapters must
    // still operate on the basename with NOFOLLOW semantics.
    for ancestor in path
        .ancestors()
        .skip(1)
        .filter(|ancestor| ancestor.exists())
    {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| ControlPlaneError {
            code: "domain_plan_write_path_invalid",
            detail: format!("{}: {error}", ancestor.display()),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ControlPlaneError {
                code: "domain_plan_write_symlink",
                detail: format!(
                    "write path resolves through a symlink ancestor: {}",
                    ancestor.display()
                ),
            });
        }
    }
    let mut existing = path;
    while !existing.exists() {
        existing = existing.parent().ok_or_else(|| ControlPlaneError {
            code: "domain_plan_write_outside_binding",
            detail: format!("write path has no existing ancestor: {}", path.display()),
        })?;
    }
    let existing = existing.canonicalize().map_err(|error| ControlPlaneError {
        code: "domain_plan_write_path_invalid",
        detail: format!("{}: {error}", existing.display()),
    })?;
    let allowed = authorized_roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|canonical| existing.starts_with(canonical))
    });
    if allowed {
        Ok(())
    } else {
        Err(ControlPlaneError {
            code: "domain_plan_write_outside_binding",
            detail: format!(
                "write path is outside authenticated roots: {}",
                path.display()
            ),
        })
    }
}

#[cfg(unix)]
fn tree_digest(roots: &[PathBuf]) -> Result<String, ControlPlaneError> {
    let mut entries = Vec::new();
    let mut budget = SnapshotBudget::default();
    for root in roots {
        collect_tree(root, &mut entries, &mut budget)?;
    }
    entries.sort();
    Ok(sha256(entries.join("\n")))
}

#[derive(Default)]
#[cfg(unix)]
struct SnapshotBudget {
    entries: usize,
    enumerated_entries: usize,
    name_bytes: usize,
    bytes: usize,
}

#[cfg(unix)]
fn collect_tree(
    root: &Path,
    entries: &mut Vec<String>,
    budget: &mut SnapshotBudget,
) -> Result<(), ControlPlaneError> {
    let (parent, name) = match open_snapshot_parent(root) {
        Err(error) if error.code == "read_only_snapshot_missing" => {
            entries.push(format!("{}\tmissing", root.display()));
            return Ok(());
        }
        Err(error) => return Err(error),
        Ok(value) => value,
    };
    match snapshot_entry(&parent, &name, Path::new(""), 0, entries, budget) {
        Err(error) if error.code == "read_only_snapshot_missing" => {
            entries.push(format!("{}\tmissing", root.display()));
            Ok(())
        }
        result => result,
    }
}

#[cfg(unix)]
fn open_snapshot_parent(root: &Path) -> Result<(OwnedFd, std::ffi::OsString), ControlPlaneError> {
    if !root.is_absolute() {
        return Err(ControlPlaneError {
            code: "read_only_snapshot_failed",
            detail: format!("snapshot root is not absolute: {}", root.display()),
        });
    }
    let name = root.file_name().ok_or_else(|| ControlPlaneError {
        code: "read_only_snapshot_failed",
        detail: format!("snapshot root has no basename: {}", root.display()),
    })?;
    let mut directory = rustix::fs::open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(snapshot_io_error)?;
    for component in root.parent().unwrap_or_else(|| Path::new("/")).components() {
        let std::path::Component::Normal(component) = component else {
            continue;
        };
        directory = rustix::fs::openat(
            &directory,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            if error == rustix::io::Errno::NOENT {
                ControlPlaneError {
                    code: "read_only_snapshot_missing",
                    detail: root.display().to_string(),
                }
            } else {
                snapshot_io_error(error)
            }
        })?;
    }
    Ok((directory, name.to_os_string()))
}

#[cfg(unix)]
fn snapshot_entry(
    parent: &OwnedFd,
    name: &std::ffi::OsStr,
    relative: &Path,
    depth: usize,
    entries: &mut Vec<String>,
    budget: &mut SnapshotBudget,
) -> Result<(), ControlPlaneError> {
    if depth > MAX_SNAPSHOT_DEPTH {
        return Err(snapshot_budget_error("depth"));
    }
    budget.entries = budget.entries.saturating_add(1);
    if budget.entries > MAX_SNAPSHOT_ENTRIES {
        return Err(snapshot_budget_error("entry count"));
    }
    let stat = rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW).map_err(
        |error| {
            if error == rustix::io::Errno::NOENT {
                ControlPlaneError {
                    code: "read_only_snapshot_missing",
                    detail: relative.display().to_string(),
                }
            } else {
                snapshot_io_error(error)
            }
        },
    )?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    #[cfg(all(test, unix))]
    tests::run_snapshot_after_stat_test_hook(relative);
    let display = relative.display();
    if file_type.is_symlink() {
        let target = rustix::fs::readlinkat(parent, name, Vec::new()).map_err(snapshot_io_error)?;
        let target = target.as_bytes();
        let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(budget.bytes);
        if target.len() > remaining {
            return Err(snapshot_budget_error("total bytes"));
        }
        budget.bytes += target.len();
        entries.push(format!("{display}\tsymlink\t{}", sha256(target)));
        return Ok(());
    }
    if file_type.is_file() {
        let descriptor = rustix::fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(snapshot_io_error)?;
        let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(budget.bytes);
        let stable = read_regular_fd(&descriptor, remaining as u64, || {
            #[cfg(all(test, unix))]
            tests::run_snapshot_after_read_rewrite_test_hook(relative);
        })
        .map_err(|error| match error {
            StableReadError::TooLarge => snapshot_budget_error("total bytes"),
            StableReadError::Changed => ControlPlaneError {
                code: "read_only_snapshot_failed",
                detail: format!("file changed during read: {display}"),
            },
            StableReadError::NotRegular => ControlPlaneError {
                code: "read_only_snapshot_failed",
                detail: format!("post-open type is not a regular file: {display}"),
            },
            StableReadError::Io(error) => ControlPlaneError {
                code: "read_only_snapshot_failed",
                detail: error,
            },
        })?;
        budget.bytes += stable.bytes.len();
        entries.push(format!("{display}\tfile\t{}", sha256(stable.bytes)));
        return Ok(());
    }
    if file_type.is_dir() {
        let directory = rustix::fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(snapshot_io_error)?;
        let opened = rustix::fs::fstat(&directory).map_err(snapshot_io_error)?;
        if !FileType::from_raw_mode(opened.st_mode).is_dir() {
            return Err(ControlPlaneError {
                code: "read_only_snapshot_failed",
                detail: format!("post-open type is not a directory: {display}"),
            });
        }
        entries.push(format!("{display}\tdir"));
        let mut names = Vec::new();
        for entry in rustix::fs::Dir::read_from(&directory).map_err(snapshot_io_error)? {
            let entry = entry.map_err(snapshot_io_error)?;
            let name = entry
                .file_name()
                .to_str()
                .map_err(snapshot_io_error)?
                .to_string();
            if name == "." || name == ".." {
                continue;
            }
            budget.enumerated_entries = budget.enumerated_entries.saturating_add(1);
            if budget.enumerated_entries > MAX_SNAPSHOT_ENTRIES {
                return Err(snapshot_budget_error("entry count"));
            }
            budget.name_bytes = budget.name_bytes.saturating_add(name.len());
            if budget.name_bytes > MAX_SNAPSHOT_NAME_BYTES {
                return Err(snapshot_budget_error("name bytes"));
            }
            names.push(name);
        }
        names.sort();
        for child in names {
            let child_relative = relative.join(&child);
            snapshot_entry(
                &directory,
                std::ffi::OsStr::new(&child),
                &child_relative,
                depth + 1,
                entries,
                budget,
            )?;
        }
        return Ok(());
    }
    Err(ControlPlaneError {
        code: "read_only_snapshot_failed",
        detail: format!("unsupported special file: {display}"),
    })
}

#[cfg(unix)]
fn snapshot_io_error(error: impl fmt::Display) -> ControlPlaneError {
    ControlPlaneError {
        code: "read_only_snapshot_failed",
        detail: error.to_string(),
    }
}

#[cfg(unix)]
fn snapshot_budget_error(kind: &str) -> ControlPlaneError {
    ControlPlaneError {
        code: "read_only_snapshot_budget_exceeded",
        detail: format!("mutation snapshot exceeded {kind} budget"),
    }
}

// Receipt identity intentionally lists every sealed terminal fact.
#[allow(clippy::too_many_arguments)]
fn receipt(
    operation: OperationName,
    status: ReceiptStatus,
    plan_hash: &str,
    payload_hash: &str,
    binding_hash: &str,
    output_digest: &str,
    observed_write_set: Vec<String>,
    recovered: bool,
) -> OperationReceipt {
    receipt_with_evidence(
        operation,
        status,
        plan_hash,
        payload_hash,
        binding_hash,
        output_digest,
        observed_write_set,
        recovered,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn receipt_with_evidence(
    operation: OperationName,
    status: ReceiptStatus,
    plan_hash: &str,
    payload_hash: &str,
    binding_hash: &str,
    output_digest: &str,
    observed_write_set: Vec<String>,
    recovered: bool,
    evidence: Option<serde_json::Value>,
) -> OperationReceipt {
    let evidence_digest = evidence
        .as_ref()
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(sha256)
        .unwrap_or_else(|| sha256("no-evidence"));
    let receipt_id = short_id(
        "receipt-v2",
        &sha256(format!(
            "{}\n{:?}\n{}\n{}\n{}\n{:?}\n{}\n{}",
            operation.as_str(),
            status,
            plan_hash,
            payload_hash,
            output_digest,
            observed_write_set,
            recovered,
            evidence_digest
        )),
    );
    OperationReceipt {
        schema_version: CONTRACT_SCHEMA_VERSION.to_string(),
        receipt_id,
        operation,
        status,
        plan_hash: plan_hash.to_string(),
        payload_hash: payload_hash.to_string(),
        binding_hash: binding_hash.to_string(),
        output_digest: output_digest.to_string(),
        observed_write_set,
        recovered,
        evidence,
    }
}

fn durable_recovery_evidence(
    recovery_action_ref: &str,
    recovery_policy_hash: &str,
    journal_identity_digest: &str,
    journal_state_digest_at_open: &str,
    adapter_evidence: Option<serde_json::Value>,
) -> serde_json::Value {
    let final_identity_digest = adapter_evidence
        .as_ref()
        .and_then(|evidence| evidence.get("journal_identity_digest"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(journal_identity_digest);
    let final_state_digest = adapter_evidence
        .as_ref()
        .and_then(|evidence| evidence.get("journal_state_digest"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(journal_state_digest_at_open);
    serde_json::json!({
        "recovery_action_ref": recovery_action_ref,
        "recovery_policy_hash": recovery_policy_hash,
        "original_journal_digest": journal_state_digest_at_open,
        "journal_identity_digest": final_identity_digest,
        "journal_state_digest": final_state_digest,
        "adapter_evidence": adapter_evidence,
    })
}

fn has_unexpected_writes(allowed: &[String], observed: &[String]) -> bool {
    observed.iter().any(|path| {
        let path = Path::new(path);
        if !safe_absolute_path(path) {
            return true;
        }
        !allowed.iter().any(|root| {
            let root = Path::new(root);
            safe_absolute_path(root) && (path == root || path.starts_with(root))
        })
    })
}

fn safe_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
                    | std::path::Component::Normal(_)
            )
        })
}

fn short_id(prefix: &str, digest: &str) -> String {
    format!(
        "{prefix}-{}",
        digest
            .trim_start_matches("sha256:")
            .get(..32)
            .unwrap_or("invalid")
    )
}

#[cfg(test)]
mod tests;
