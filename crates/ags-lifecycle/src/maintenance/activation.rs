use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Closed domain request for activating already-published Host snapshots.
/// Transport and daemon ownership stay above `ags-lifecycle` behind this port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRuntimeActivationRequest {
    pub workspace: PathBuf,
    pub runtime_home: PathBuf,
    pub active_snapshot_hashes: BTreeMap<String, String>,
    pub retired_hosts: Vec<String>,
    pub replace_all: bool,
}

impl CapabilityRuntimeActivationRequest {
    pub fn from_runtime(
        workspace: &Path,
        runtime_home: &Path,
        affected_hosts: &[String],
        replace_all: bool,
    ) -> Result<Self, String> {
        let hosts = affected_hosts.iter().cloned().collect::<BTreeSet<_>>();
        if hosts.len() != affected_hosts.len() {
            return Err("capability activation contains duplicate Hosts".to_string());
        }
        let mut active_snapshot_hashes = BTreeMap::new();
        let mut retired_hosts = Vec::new();
        for host in hosts {
            let path = ags_capability_governance::snapshot_path(runtime_home, &host);
            if !path.is_file() {
                retired_hosts.push(host);
                continue;
            }
            let (snapshot, _) =
                ags_capability_governance::load_static_snapshot(runtime_home, &host).map_err(
                    |error| format!("cannot load `{host}` activation snapshot: {error:?}"),
                )?;
            active_snapshot_hashes.insert(host, snapshot.snapshot_hash);
        }
        Ok(Self {
            workspace: workspace.to_path_buf(),
            runtime_home: runtime_home.to_path_buf(),
            active_snapshot_hashes,
            retired_hosts,
            replace_all,
        })
    }

    pub fn active_hosts(&self) -> Vec<String> {
        self.active_snapshot_hashes.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRuntimeActivationResult {
    pub activated_snapshot_hashes: BTreeMap<String, String>,
    /// `None` means no daemon was running, so the next Host connection will
    /// load the verified disk state. `Some` is the complete live daemon table.
    pub loaded_snapshot_hashes: Option<BTreeMap<String, String>>,
    pub runtime_identity: Option<String>,
}

pub trait CapabilityRuntimeActivator: Send + Sync {
    fn activate(
        &self,
        request: &CapabilityRuntimeActivationRequest,
    ) -> Result<CapabilityRuntimeActivationResult, String>;
}

/// Hermetic adapter for lifecycle tests and source-only callers. Production
/// CLI/MCP surfaces inject the workspace-session adapter from `ags-mcp`.
#[derive(Debug, Default)]
pub struct OfflineCapabilityRuntimeActivator;

impl CapabilityRuntimeActivator for OfflineCapabilityRuntimeActivator {
    fn activate(
        &self,
        request: &CapabilityRuntimeActivationRequest,
    ) -> Result<CapabilityRuntimeActivationResult, String> {
        Ok(CapabilityRuntimeActivationResult {
            activated_snapshot_hashes: request.active_snapshot_hashes.clone(),
            loaded_snapshot_hashes: None,
            runtime_identity: None,
        })
    }
}

pub fn offline_capability_runtime_activator() -> Arc<dyn CapabilityRuntimeActivator> {
    Arc::new(OfflineCapabilityRuntimeActivator)
}
