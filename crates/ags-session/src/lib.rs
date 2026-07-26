//! Workspace-scoped AGS service and session authority.
//!
//! The instance key is derived only from the canonical workspace path. Hosts
//! are client attributes; preflight bindings and one-shot action leases remain
//! isolated per client session.

mod action_store;
mod client_session;
mod workspace_service;

use std::path::PathBuf;

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
    /// Exact typed capability state accepted by preflight.
    ///
    /// `SnapshotStale` and `Unavailable` remain distinct after preflight so
    /// resource and route admission can preserve the original failure instead
    /// of degrading both states to a missing ready binding.
    pub capability: Option<CapabilityReference>,
}

/// Workspace-local capability generation accepted by one client session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityBinding {
    pub workspace_identity: String,
    pub bundle_epoch: u64,
    pub snapshot_hash: String,
}

/// Transport-neutral capability state for one preflight binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityReference {
    Ready { binding: CapabilityBinding },
    SnapshotStale,
    Unavailable { diagnostic: CapabilityDiagnostic },
}

impl CapabilityReference {
    pub fn ready_binding(&self) -> Option<&CapabilityBinding> {
        match self {
            Self::Ready { binding } => Some(binding),
            Self::SnapshotStale | Self::Unavailable { .. } => None,
        }
    }

    pub fn as_failure(&self) -> Option<CapabilityLoadFailure> {
        match self {
            Self::Ready { .. } => None,
            Self::SnapshotStale => Some(CapabilityLoadFailure::SnapshotStale),
            Self::Unavailable { diagnostic } => {
                Some(CapabilityLoadFailure::Unavailable(diagnostic.clone()))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDiagnosticCode {
    AuthorityUnavailable,
    SnapshotBuildFailed,
    SnapshotReadFailed,
    SnapshotCorrupt,
    SnapshotIntegrityFailed,
    SnapshotInvalid,
    WorkspaceTargetInvalid,
    StateLockUnavailable,
    StatePersistenceFailed,
    SourceUnavailable,
}

impl CapabilityDiagnosticCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuthorityUnavailable => "capability_authority_unavailable",
            Self::SnapshotBuildFailed => "capability_snapshot_build_failed",
            Self::SnapshotReadFailed => "capability_snapshot_read_failed",
            Self::SnapshotCorrupt => "capability_snapshot_corrupt",
            Self::SnapshotIntegrityFailed => "capability_snapshot_integrity_failed",
            Self::SnapshotInvalid => "capability_snapshot_invalid",
            Self::WorkspaceTargetInvalid => "capability_workspace_target_invalid",
            Self::StateLockUnavailable => "capability_state_lock_unavailable",
            Self::StatePersistenceFailed => "capability_state_persistence_failed",
            Self::SourceUnavailable => "capability_source_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDiagnostic {
    pub code: CapabilityDiagnosticCode,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityLoadFailure {
    SnapshotStale,
    Unavailable(CapabilityDiagnostic),
}

impl CapabilityLoadFailure {
    pub fn into_reference(self) -> CapabilityReference {
        match self {
            Self::SnapshotStale => CapabilityReference::SnapshotStale,
            Self::Unavailable(diagnostic) => CapabilityReference::Unavailable { diagnostic },
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::SnapshotStale => "skill_snapshot_stale",
            Self::Unavailable(diagnostic) => diagnostic.code.as_str(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::SnapshotStale => {
                "the preflight-bound host capability snapshot is stale".to_string()
            }
            Self::Unavailable(diagnostic) => diagnostic.detail.clone(),
        }
    }

    pub fn into_legacy_error(self) -> String {
        match self {
            Self::SnapshotStale => "skill_snapshot_stale".to_string(),
            Self::Unavailable(diagnostic) => {
                format!("{}: {}", diagnostic.code.as_str(), diagnostic.detail)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedCapabilityCatalog {
    pub snapshot: ags_capability_governance::HostCapabilitySnapshot,
    pub table: ags_capability_governance::ActiveSkillTable,
    pub binding: CapabilityBinding,
}

/// Capability access supplied by the workspace service to a client session.
///
/// The session layer returns domain state only. Protocol adapters are solely
/// responsible for URI, JSON, remediation argv, and other presentation.
pub trait CapabilityCatalogSource {
    fn capability_reference(&self, binding: &PreflightBinding) -> CapabilityReference {
        match self.load_validated_snapshot(binding) {
            Ok(catalog) => CapabilityReference::Ready {
                binding: catalog.binding,
            },
            Err(error) => error.into_reference(),
        }
    }

    fn load_validated_snapshot(
        &self,
        binding: &PreflightBinding,
    ) -> Result<ValidatedCapabilityCatalog, CapabilityLoadFailure>;
}

/// Filesystem-backed adapter for standalone sessions that are not connected to
/// a workspace daemon. It shares the same typed source interface as the daemon
/// and therefore cannot create a second MCP presentation path.
#[derive(Debug, Clone)]
pub struct LocalCapabilityCatalogSource {
    runtime_home: PathBuf,
}

impl LocalCapabilityCatalogSource {
    pub fn new(runtime_home: PathBuf) -> Self {
        Self { runtime_home }
    }
}

impl CapabilityCatalogSource for LocalCapabilityCatalogSource {
    fn capability_reference(&self, binding: &PreflightBinding) -> CapabilityReference {
        match self.load_catalog(binding) {
            Ok(catalog) => CapabilityReference::Ready {
                binding: catalog.binding,
            },
            Err(error) => error.into_reference(),
        }
    }

    fn load_validated_snapshot(
        &self,
        binding: &PreflightBinding,
    ) -> Result<ValidatedCapabilityCatalog, CapabilityLoadFailure> {
        self.load_catalog(binding)
    }
}

impl LocalCapabilityCatalogSource {
    fn load_catalog(
        &self,
        binding: &PreflightBinding,
    ) -> Result<ValidatedCapabilityCatalog, CapabilityLoadFailure> {
        let authority = ags_capability_governance::resolve_capability_authority_root(
            &binding.target,
            &self.runtime_home,
            std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from),
        )
        .map_err(|error| {
            unavailable(
                CapabilityDiagnosticCode::AuthorityUnavailable,
                error.to_string(),
            )
        })?;
        let (snapshot, table) = ags_capability_governance::load_validated_snapshot_with_roots(
            &authority,
            &self.runtime_home,
            &binding.host,
            &binding.host_home,
        )
        .map_err(|error| classify_snapshot_load_error(error, &self.runtime_home, &binding.host))?;
        let workspace_identity = ags_platform::sha256(
            format!(
                "standalone-capability\n{}",
                binding.target.to_string_lossy()
            )
            .as_bytes(),
        );
        Ok(ValidatedCapabilityCatalog {
            binding: CapabilityBinding {
                workspace_identity,
                bundle_epoch: 0,
                snapshot_hash: snapshot.snapshot_hash.clone(),
            },
            snapshot,
            table,
        })
    }
}

pub(crate) fn unavailable(
    code: CapabilityDiagnosticCode,
    detail: impl Into<String>,
) -> CapabilityLoadFailure {
    CapabilityLoadFailure::Unavailable(CapabilityDiagnostic {
        code,
        detail: detail.into(),
    })
}

pub(crate) fn classify_snapshot_load_error(
    error: ags_capability_governance::SnapshotLoadError,
    runtime_home: &std::path::Path,
    host: &str,
) -> CapabilityLoadFailure {
    use ags_capability_governance::{SnapshotError, SnapshotLoadError};

    match error {
        SnapshotLoadError::SkillSnapshotStale => {
            let path = ags_capability_governance::snapshot_path(runtime_home, host);
            match std::fs::read_to_string(&path) {
                Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {
                    CapabilityLoadFailure::SnapshotStale
                }
                Err(io_error) => unavailable(
                    CapabilityDiagnosticCode::SnapshotReadFailed,
                    format!("cannot read {}: {io_error}", path.display()),
                ),
                Ok(content) => {
                    if serde_json::from_str::<ags_capability_governance::HostCapabilitySnapshot>(
                        &content,
                    )
                    .is_err()
                    {
                        unavailable(
                            CapabilityDiagnosticCode::SnapshotCorrupt,
                            format!("snapshot JSON is corrupt at {}", path.display()),
                        )
                    } else {
                        CapabilityLoadFailure::SnapshotStale
                    }
                }
            }
        }
        SnapshotLoadError::Build(build_error) => unavailable(
            CapabilityDiagnosticCode::SnapshotBuildFailed,
            format!("{build_error:?}"),
        ),
        SnapshotLoadError::Snapshot(SnapshotError::SkillSnapshotStale) => {
            CapabilityLoadFailure::SnapshotStale
        }
        SnapshotLoadError::Snapshot(SnapshotError::SnapshotIntegrityFailed) => unavailable(
            CapabilityDiagnosticCode::SnapshotIntegrityFailed,
            "persisted snapshot integrity validation failed",
        ),
        SnapshotLoadError::Snapshot(SnapshotError::InvalidActiveTable(error)) => unavailable(
            CapabilityDiagnosticCode::SnapshotInvalid,
            format!("active capability table is invalid: {error:?}"),
        ),
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[test]
    fn missing_snapshot_is_stale_but_corrupt_snapshot_is_unavailable() {
        let runtime = tempfile::tempdir().unwrap();
        let missing = classify_snapshot_load_error(
            ags_capability_governance::SnapshotLoadError::SkillSnapshotStale,
            runtime.path(),
            "codex",
        );
        assert!(matches!(missing, CapabilityLoadFailure::SnapshotStale));

        let snapshot_path = ags_capability_governance::snapshot_path(runtime.path(), "codex");
        std::fs::create_dir_all(snapshot_path.parent().unwrap()).unwrap();
        std::fs::write(&snapshot_path, b"{not-json").unwrap();
        let corrupt = classify_snapshot_load_error(
            ags_capability_governance::SnapshotLoadError::SkillSnapshotStale,
            runtime.path(),
            "codex",
        );
        assert!(matches!(
            corrupt,
            CapabilityLoadFailure::Unavailable(CapabilityDiagnostic {
                code: CapabilityDiagnosticCode::SnapshotCorrupt,
                ..
            })
        ));

        let mismatch = classify_snapshot_load_error(
            ags_capability_governance::SnapshotLoadError::Snapshot(
                ags_capability_governance::SnapshotError::SkillSnapshotStale,
            ),
            runtime.path(),
            "codex",
        );
        assert!(matches!(mismatch, CapabilityLoadFailure::SnapshotStale));
    }

    #[test]
    fn build_and_integrity_failures_are_not_refreshable_snapshot_state() {
        let runtime = tempfile::tempdir().unwrap();
        let build = classify_snapshot_load_error(
            ags_capability_governance::SnapshotLoadError::Build(
                ags_capability_governance::SnapshotBuildError::Read(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "registry denied",
                )),
            ),
            runtime.path(),
            "codex",
        );
        assert!(matches!(
            build,
            CapabilityLoadFailure::Unavailable(CapabilityDiagnostic {
                code: CapabilityDiagnosticCode::SnapshotBuildFailed,
                ..
            })
        ));

        let integrity = classify_snapshot_load_error(
            ags_capability_governance::SnapshotLoadError::Snapshot(
                ags_capability_governance::SnapshotError::SnapshotIntegrityFailed,
            ),
            runtime.path(),
            "codex",
        );
        assert!(matches!(
            integrity,
            CapabilityLoadFailure::Unavailable(CapabilityDiagnostic {
                code: CapabilityDiagnosticCode::SnapshotIntegrityFailed,
                ..
            })
        ));
    }
}
