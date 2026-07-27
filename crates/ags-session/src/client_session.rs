use std::path::PathBuf;
use std::sync::Arc;

use crate::{CapabilityReference, PreflightBinding, SessionActionStore, WorkspaceState};

#[derive(Debug)]
enum AdmissionState {
    Unbound,
    Ready(PreflightBinding),
    BootstrapRequired(PreflightBinding),
}

/// Governance state for one daemon client session.
///
/// The workspace service is shared, but the accepted preflight binding and
/// one-shot actions are isolated here. Protocol adapters may hold this value;
/// they do not own or reimplement its state transitions.
#[derive(Debug)]
pub struct WorkspaceClientSession<T> {
    admission: AdmissionState,
    actions: SessionActionStore<T>,
    workspace: Arc<WorkspaceState>,
    session_id: String,
    host_home: PathBuf,
}

impl<T> WorkspaceClientSession<T> {
    pub fn new(workspace: Arc<WorkspaceState>, session_id: String, host_home: PathBuf) -> Self {
        Self {
            admission: AdmissionState::Unbound,
            actions: SessionActionStore::for_session(&session_id),
            workspace,
            session_id,
            host_home,
        }
    }

    pub fn reset(&mut self) {
        self.admission = AdmissionState::Unbound;
        self.actions.invalidate();
    }

    pub fn invalidate_actions(&mut self) {
        self.actions.invalidate();
    }

    pub fn bind_ready(
        &mut self,
        agent: String,
        target: String,
        capability: Option<CapabilityReference>,
    ) {
        self.admission = AdmissionState::Ready(PreflightBinding {
            host: agent,
            target: target.into(),
            host_home: self.host_home.clone(),
            capability,
        });
    }

    pub fn bind_bootstrap_required(&mut self, agent: String, target: String) {
        self.admission = AdmissionState::BootstrapRequired(PreflightBinding {
            host: agent,
            target: target.into(),
            host_home: self.host_home.clone(),
            capability: None,
        });
    }

    pub fn binding(&self) -> Option<PreflightBinding> {
        match &self.admission {
            AdmissionState::Unbound => None,
            AdmissionState::Ready(binding) | AdmissionState::BootstrapRequired(binding) => {
                Some(binding.clone())
            }
        }
    }

    pub fn capability_reference(&self) -> Option<&CapabilityReference> {
        match &self.admission {
            AdmissionState::Ready(binding) => binding.capability.as_ref(),
            AdmissionState::Unbound | AdmissionState::BootstrapRequired(_) => None,
        }
    }

    pub fn is_preflight_completed(&self) -> bool {
        matches!(self.admission, AdmissionState::Ready(_))
    }

    pub fn is_bootstrap_required(&self) -> bool {
        matches!(self.admission, AdmissionState::BootstrapRequired(_))
    }

    pub fn preflight_agent(&self) -> Option<&str> {
        match &self.admission {
            AdmissionState::Ready(binding) | AdmissionState::BootstrapRequired(binding) => {
                Some(&binding.host)
            }
            AdmissionState::Unbound => None,
        }
    }

    pub fn preflight_target(&self) -> Option<&str> {
        match &self.admission {
            AdmissionState::Ready(binding) | AdmissionState::BootstrapRequired(binding) => {
                binding.target.to_str()
            }
            AdmissionState::Unbound => None,
        }
    }

    pub fn action_store(&self) -> &SessionActionStore<T> {
        &self.actions
    }

    pub fn action_store_mut(&mut self) -> &mut SessionActionStore<T> {
        &mut self.actions
    }

    pub fn workspace(&self) -> &Arc<WorkspaceState> {
        &self.workspace
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session<T>(host_home: PathBuf, session_id: &str) -> WorkspaceClientSession<T> {
        let root = ags_platform::canonical_workspace_root(
            &std::env::current_dir().expect("current test directory"),
        )
        .expect("canonical test workspace");
        let workspace = Arc::new(
            WorkspaceState::new(root, ags_capability_governance::locate_runtime_home())
                .expect("test workspace state"),
        );
        WorkspaceClientSession::new(workspace, session_id.to_string(), host_home)
    }

    #[test]
    fn reset_clears_binding_and_expires_only_this_sessions_actions() {
        let home = PathBuf::from("/tmp/ags-session-test-home");
        let mut first = session::<()>(home.clone(), "first-session");
        let mut second = session::<()>(home, "second-session");
        first.bind_ready("codex".to_string(), ".".to_string(), None);
        first
            .action_store_mut()
            .insert("first-action".to_string(), ());
        second
            .action_store_mut()
            .insert("second-action".to_string(), ());

        first.reset();

        assert!(first.binding().is_none());
        assert!(first.action_store().get("first-action").is_none());
        assert!(second.action_store().get("second-action").is_some());
    }

    #[test]
    fn bootstrap_binding_is_explicit_and_session_local() {
        let mut session = session::<()>(
            PathBuf::from("/tmp/ags-session-test-home"),
            "bootstrap-session",
        );
        session.bind_bootstrap_required("claude-code".to_string(), "/tmp/project".to_string());

        assert!(session.is_bootstrap_required());
        assert!(!session.is_preflight_completed());
        assert_eq!(session.binding().unwrap().host, "claude-code");
    }
}
