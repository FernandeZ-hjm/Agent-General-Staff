use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::{
    classify_snapshot_load_error, unavailable, CapabilityBinding, CapabilityCatalogSource,
    CapabilityDiagnosticCode, CapabilityLoadFailure, CapabilityReference, PreflightBinding,
    ValidatedCapabilityCatalog,
};
use ags_platform::canonical_workspace_root;

use super::registry_ownership::{
    atomic_write_json, ensure_private_dir, workspace_key, ServicePaths,
};

const CAPABILITY_BUNDLE_SCHEMA: &str = "0.3.0-workspace-capabilities";

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WorkspaceCapabilityBundle {
    schema_version: String,
    workspace: PathBuf,
    #[serde(default)]
    workspace_identity: String,
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    host_epochs: HashMap<String, u64>,
    snapshots: HashMap<String, ags_capability_governance::HostCapabilitySnapshot>,
}

#[derive(Debug, Clone, Default)]
struct WorkspaceCapabilityState {
    epoch: u64,
    host_epochs: HashMap<String, u64>,
    snapshots: HashMap<String, ags_capability_governance::HostCapabilitySnapshot>,
}

/// Shared state owned by the unique daemon for one canonical workspace.
#[derive(Debug)]
pub struct WorkspaceState {
    root: PathBuf,
    instance_key: String,
    runtime_home: PathBuf,
    enforce_root: bool,
    capabilities: RwLock<WorkspaceCapabilityState>,
}

impl WorkspaceState {
    pub fn new(root: PathBuf, runtime_home: PathBuf) -> Result<Self, String> {
        let instance_key = workspace_key(&root);
        let capabilities = load_capability_bundle(&runtime_home, &root)?;
        Ok(Self {
            root,
            instance_key,
            runtime_home,
            enforce_root: true,
            capabilities: RwLock::new(capabilities),
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
            capabilities: RwLock::new(WorkspaceCapabilityState::default()),
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
        let expected = ags_capability_governance::build_capability_snapshot_with_roots(
            &authority,
            &binding.host,
            &self.runtime_home,
            &binding.host_home,
        )
        .map_err(|error| {
            unavailable(
                CapabilityDiagnosticCode::SnapshotBuildFailed,
                format!("{error:?}"),
            )
        })?;

        let mut capabilities = self.capabilities.write().map_err(|_| {
            unavailable(
                CapabilityDiagnosticCode::StateLockUnavailable,
                "workspace snapshot lock poisoned",
            )
        })?;
        let cached_is_current = capabilities
            .snapshots
            .get(&binding.host)
            .is_some_and(|cached| {
                cached
                    .validate(
                        &binding.host,
                        &expected.registry_hash,
                        &expected.overlay_hash,
                        &expected.runtime_hash,
                    )
                    .is_ok()
                    && cached.catalog_hash == expected.catalog_hash
                    && cached.active_table_hash == expected.active_table_hash
                    && cached.snapshot_hash == expected.snapshot_hash
            });
        if cached_is_current {
            let snapshot = capabilities
                .snapshots
                .get(&binding.host)
                .expect("validated cached workspace snapshot")
                .clone();
            let table = ags_capability_governance::ActiveSkillTable::new(
                snapshot.host.clone(),
                snapshot.snapshot_hash.clone(),
                snapshot.active_skills.clone(),
            )
            .map_err(|error| {
                unavailable(
                    CapabilityDiagnosticCode::SnapshotInvalid,
                    format!("active capability table is invalid: {error:?}"),
                )
            })?;
            return Ok(ValidatedCapabilityCatalog {
                binding: self.capability_binding(
                    capabilities
                        .host_epochs
                        .get(&binding.host)
                        .copied()
                        .unwrap_or(0),
                    &snapshot.snapshot_hash,
                ),
                snapshot,
                table,
            });
        }

        let (snapshot, table) = ags_capability_governance::load_validated_snapshot_with_roots(
            &authority,
            &self.runtime_home,
            &binding.host,
            &binding.host_home,
        )
        .map_err(|error| classify_snapshot_load_error(error, &self.runtime_home, &binding.host))?;
        let mut candidate = capabilities.clone();
        candidate.epoch = candidate.epoch.saturating_add(1);
        let host_epoch = candidate
            .host_epochs
            .entry(binding.host.clone())
            .or_insert(0);
        *host_epoch = host_epoch.saturating_add(1);
        candidate
            .snapshots
            .insert(binding.host.clone(), snapshot.clone());
        self.persist_capability_bundle(&candidate)
            .map_err(|error| {
                unavailable(CapabilityDiagnosticCode::StatePersistenceFailed, error)
            })?;
        *capabilities = candidate;
        Ok(ValidatedCapabilityCatalog {
            binding: self.capability_binding(
                capabilities
                    .host_epochs
                    .get(&binding.host)
                    .copied()
                    .unwrap_or(0),
                &snapshot.snapshot_hash,
            ),
            snapshot,
            table,
        })
    }

    fn capability_binding(&self, bundle_epoch: u64, snapshot_hash: &str) -> CapabilityBinding {
        CapabilityBinding {
            workspace_identity: self.instance_key.clone(),
            bundle_epoch,
            snapshot_hash: snapshot_hash.to_string(),
        }
    }

    fn persist_capability_bundle(
        &self,
        capabilities: &WorkspaceCapabilityState,
    ) -> Result<(), String> {
        let paths = ServicePaths::new(&self.runtime_home, &self.root);
        ensure_private_dir(&paths.dir)?;
        let bundle = WorkspaceCapabilityBundle {
            schema_version: CAPABILITY_BUNDLE_SCHEMA.to_string(),
            workspace: self.root.clone(),
            workspace_identity: self.instance_key.clone(),
            epoch: capabilities.epoch,
            host_epochs: capabilities.host_epochs.clone(),
            snapshots: capabilities.snapshots.clone(),
        };
        atomic_write_json(&paths.capabilities, &bundle)
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

fn load_capability_bundle(
    runtime_home: &Path,
    workspace: &Path,
) -> Result<WorkspaceCapabilityState, String> {
    let paths = ServicePaths::new(runtime_home, workspace);
    let bytes = match fs::read(&paths.capabilities) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceCapabilityState::default());
        }
        Err(error) => {
            return Err(format!(
                "workspace capability bundle read failed at {}: {error}",
                paths.capabilities.display()
            ));
        }
    };
    let bundle = serde_json::from_slice::<WorkspaceCapabilityBundle>(&bytes).map_err(|error| {
        format!(
            "workspace capability bundle corrupt at {}: {error}",
            paths.capabilities.display()
        )
    })?;
    let identity = workspace_key(workspace);
    if bundle.schema_version != CAPABILITY_BUNDLE_SCHEMA
        || bundle.workspace != workspace
        || (!bundle.workspace_identity.is_empty() && bundle.workspace_identity != identity)
    {
        return Err(format!(
            "workspace capability bundle binding invalid at {}",
            paths.capabilities.display()
        ));
    }
    let mut host_epochs = bundle.host_epochs;
    if host_epochs.is_empty() {
        let migrated_epoch = bundle.epoch.max(1);
        host_epochs.extend(
            bundle
                .snapshots
                .keys()
                .cloned()
                .map(|host| (host, migrated_epoch)),
        );
    }
    Ok(WorkspaceCapabilityState {
        epoch: bundle.epoch,
        host_epochs,
        snapshots: bundle.snapshots,
    })
}
