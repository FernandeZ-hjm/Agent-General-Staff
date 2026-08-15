//! Unified AGS contract v2 control plane.
//!
//! Adapters cross one deep Module Interface: `open`, `decide`, and `apply`.
//! Typed Operation declarations, binding seals, domain planning, receipts,
//! recovery, and risk escalation remain inside this crate.
//!
//! The production interface exposes the three behavioral operations only:
//!
//! ```
//! fn accepts_production_facade(_: &mut ags_control_plane::ProductionControlPlane) {}
//! ```
//!
//! Registry consumers can generate typed read-only adapters from the same 26
//! declarations without gaining access to execution internals:
//!
//! ```
//! macro_rules! audit_registry {
//!     ($( $variant:ident($request:ty) => $wire:literal, $cli:literal, $surface:ident,
//!         $resolver:path, [$primary:ident $(, $allowed:ident)*], $schema:literal,
//!         $summary:literal; )+) => {
//!         fn registry_rows() -> Vec<(&'static str, &'static str, &'static str)> {
//!             let mut rows = Vec::new();
//!             $(
//!                 let _ = std::any::TypeId::of::<$request>();
//!                 let _: fn(&$request, ags_control_plane::OperationKind)
//!                     -> ags_control_plane::OperationKind = $resolver;
//!                 assert!(!$schema.is_empty());
//!                 rows.push(($wire, $cli, stringify!($surface)));
//!             )+
//!             rows
//!         }
//!     };
//! }
//! ags_control_plane::for_each_operation!(audit_registry);
//! let rows = registry_rows();
//! assert_eq!(rows.len(), 26);
//! assert!(rows.iter().any(|row| row.0 == "details.read"
//!     && row.1.is_empty() && row.2 == "ControlPlaneInternal"));
//! ```
//!
//! Adapter mechanics and the generic implementation are not public seams:
//!
//! ```compile_fail
//! use ags_control_plane::control_plane::{ControlPlane, EffectAdapter};
//! ```
//!
//! ```compile_fail
//! use ags_control_plane::control_plane::ProductionEffectAdapter;
//! ```
//!
//! ```compile_fail
//! use ags_control_plane::workspace_lifecycle::plan_session_end;
//! ```

mod control_plane;
mod workspace_lifecycle;

use std::path::PathBuf;

pub use control_plane::{
    fixed_operation_kind, host_lifecycle_operation_request_schema, host_outcome_input_schema,
    operation_registry, operation_registry_for_surface, operation_request_schema, operation_schema,
    test_operation_kind, AdapterSurface, AgentProbeRequest, AgentRegisterRequest, AgentSurface,
    ApplyRequest, ApplyResult, ArtifactSource, AuthenticatedBinding, AuthenticatedHostOutcome,
    CapabilityInventoryRequest, CapabilitySnapshotRequest, CheckRequest, CheckScope,
    ContentAddressedArtifactRef, ControlPlaneError, Decision, DetailsChunk, DetailsReadRequest,
    DetailsReference, DoctorRequest, DoctorScope, EvidenceArtifactKind, EvidenceRequest,
    GateRequest, HostArtifactState, HostEvidenceKind, HostExecutionAction,
    HostExecutionInstruction, HostOutcomeEvidence, HostOutcomeInput, HostOutcomeReceipt,
    HostOutcomeStatus, HostProjectionRequest, HostReleaseMember, HostWriteArtifact, InitRequest,
    LifecycleSessionEndRequest, LifecycleSessionStartRequest, LifecycleStopGuardRequest,
    McpAdviceRequest, MemoryCloseRequest, MigrationMode, OpenRequest, OpenedSession,
    OperationContext, OperationKind, OperationName, OperationReceipt, OperationRequest,
    OperationSpec, OperationState, PlanStep, PolicyRequest, ProjectionMode, ReceiptStatus,
    Recoverability, SchemaRequest, SealedPlan, SetupRequest, SkillInstallRequest,
    SkillRemoveRequest, SkillSourceKind, SkillSourceSpec, SkillUpdatePolicy, TaskCloseRequest,
    TaskPlanRequest, TaskValidateRequest, TestExecutor, TestProfile, TestRequest, UpdateReceipt,
    UpdateRequest, VerificationSpec, CONTRACT_SCHEMA_VERSION, DETAILS_CHUNK_LIMIT,
    HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION, HOST_OUTCOME_SCHEMA_VERSION,
};
pub use workspace_lifecycle::{
    ClosurePointer, LifecycleDecision, LifecycleEnvelope, LifecycleSessionEndPlan,
    CLOSURE_POINTER_SCHEMA_VERSION, LIFECYCLE_SCHEMA_VERSION,
};

/// Production control-plane façade. Adapter mechanics and the generic core are
/// deliberately unreachable outside this crate.
pub struct ProductionControlPlane {
    inner: control_plane::ControlPlane<control_plane::ProductionEffectAdapter>,
}

impl ProductionControlPlane {
    pub fn new(runtime_home: impl Into<PathBuf>) -> Result<Self, ControlPlaneError> {
        control_plane::ControlPlane::new(control_plane::ProductionEffectAdapter::new(runtime_home))
            .map(|inner| Self { inner })
    }

    pub fn with_host_home(
        runtime_home: impl Into<PathBuf>,
        host_home: impl Into<PathBuf>,
    ) -> Result<Self, ControlPlaneError> {
        control_plane::ControlPlane::new(control_plane::ProductionEffectAdapter::with_host_home(
            runtime_home,
            host_home,
        ))
        .map(|inner| Self { inner })
    }

    pub fn with_host_home_and_sealing_key(
        runtime_home: impl Into<PathBuf>,
        host_home: impl Into<PathBuf>,
        sealing_key: String,
    ) -> Self {
        Self {
            inner: control_plane::ControlPlane::with_sealing_key(
                control_plane::ProductionEffectAdapter::with_host_home(runtime_home, host_home),
                sealing_key,
            ),
        }
    }

    pub fn open(&mut self, request: OpenRequest) -> Result<OpenedSession, ControlPlaneError> {
        self.inner.open(request)
    }

    pub fn decide(
        &mut self,
        session: &OpenedSession,
        operation: OperationRequest,
    ) -> Result<Decision, ControlPlaneError> {
        self.inner.decide(session, operation)
    }

    pub fn apply(
        &mut self,
        caller: &AuthenticatedBinding,
        request: ApplyRequest,
    ) -> Result<ApplyResult, ControlPlaneError> {
        self.inner.apply(caller, request)
    }
}
