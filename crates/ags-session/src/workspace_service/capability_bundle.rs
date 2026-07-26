use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{
    classify_snapshot_load_error, unavailable, CapabilityBinding, CapabilityCatalogSource,
    CapabilityDiagnosticCode, CapabilityLoadFailure, CapabilityReference, PreflightBinding,
    ValidatedCapabilityCatalog,
};
use ags_platform::canonical_workspace_root;

use super::registry_ownership::workspace_key;

/// One immutable capability snapshot per host, loaded once by the workspace daemon.
#[derive(Debug)]
pub struct WorkspaceState {
    root: PathBuf,
    instance_key: String,
    runtime_home: PathBuf,
    enforce_root: bool,
    snapshots: RwLock<HashMap<String, ags_capability_governance::HostCapabilitySnapshot>>,
}

impl WorkspaceState {
    pub fn new(root: PathBuf, runtime_home: PathBuf) -> Result<Self, String> {
        let instance_key = workspace_key(&root);
        Ok(Self {
            root,
            instance_key,
            runtime_home,
            enforce_root: true,
            snapshots: RwLock::new(HashMap::new()),
        })
    }

    #[doc(hidden)]
    pub fn standalone() -> Arc<Self> {
        let root = canonical_workspace_root(
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        )
        .unwrap_or_else(|_| PathBuf::from("."));
        Arc::new(Self {
            instance_key: workspace_key(&root),
            root,
            runtime_home: ags_capability_governance::locate_runtime_home(),
            enforce_root: false,
            snapshots: RwLock::new(HashMap::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn instance_key(&self) -> &str {
        &self.instance_key
    }

    pub fn is_daemon_owned(&self) -> bool {
        self.enforce_root
    }

    pub fn target_matches(&self, target: &Path) -> bool {
        !self.enforce_root
            || canonical_workspace_root(target).is_ok_and(|target| target == self.root)
    }

    pub fn read_catalog(
        &self,
        binding: &PreflightBinding,
    ) -> Result<ags_capability_governance::HostCapabilitySnapshot, String> {
        let Some(reference) = binding.capability.as_ref() else {
            return Err(CapabilityLoadFailure::SnapshotStale.into_legacy_error());
        };
        if let Some(failure) = reference.as_failure() {
            return Err(failure.into_legacy_error());
        }
        let accepted = reference
            .ready_binding()
            .expect("ready capability reference has a binding");
        self.load_validated_catalog(binding)
            .and_then(|catalog| {
                if accepted == &catalog.binding {
                    Ok(catalog.snapshot)
                } else {
                    Err(CapabilityLoadFailure::SnapshotStale)
                }
            })
            .map_err(CapabilityLoadFailure::into_legacy_error)
    }

    fn load_validated_catalog(
        &self,
        binding: &PreflightBinding,
    ) -> Result<ValidatedCapabilityCatalog, CapabilityLoadFailure> {
        let target = canonical_workspace_root(&binding.target).map_err(|error| {
            unavailable(
                CapabilityDiagnosticCode::WorkspaceTargetInvalid,
                error.to_string(),
            )
        })?;
        if self.enforce_root && target != self.root {
            return Err(unavailable(
                CapabilityDiagnosticCode::WorkspaceTargetInvalid,
                format!(
                    "workspace_target_mismatch: service={} requested={}",
                    self.root.display(),
                    target.display()
                ),
            ));
        }
        let mut snapshots = self.snapshots.write().map_err(|_| {
            unavailable(
                CapabilityDiagnosticCode::StateLockUnavailable,
                "workspace snapshot lock poisoned",
            )
        })?;
        if !snapshots.contains_key(&binding.host) {
            let (snapshot, _) =
                ags_capability_governance::load_static_snapshot(&self.runtime_home, &binding.host)
                    .map_err(|error| {
                        classify_snapshot_load_error(error, &self.runtime_home, &binding.host)
                    })?;
            snapshots.insert(binding.host.clone(), snapshot);
        }
        let snapshot = snapshots
            .get(&binding.host)
            .expect("static host snapshot inserted before read")
            .clone();
        let table = snapshot
            .validate_integrity(&binding.host)
            .map_err(|error| {
                unavailable(
                    CapabilityDiagnosticCode::SnapshotInvalid,
                    format!("active capability table is invalid: {error:?}"),
                )
            })?;
        Ok(ValidatedCapabilityCatalog {
            binding: self.capability_binding(&snapshot.snapshot_hash),
            snapshot,
            table,
        })
    }

    fn capability_binding(&self, snapshot_hash: &str) -> CapabilityBinding {
        CapabilityBinding {
            workspace_identity: self.instance_key.clone(),
            snapshot_hash: snapshot_hash.to_string(),
        }
    }
}

impl CapabilityCatalogSource for WorkspaceState {
    fn capability_reference(&self, binding: &PreflightBinding) -> CapabilityReference {
        match self.load_validated_catalog(binding) {
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
        self.load_validated_catalog(binding)
    }
}
