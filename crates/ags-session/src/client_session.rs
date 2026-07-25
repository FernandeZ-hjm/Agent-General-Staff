use std::path::PathBuf;
use std::sync::Arc;

use crate::{PreflightBinding, SessionActionStore, WorkspaceState};

/// Governance state for one daemon client session.
///
/// The workspace service is shared, but the accepted preflight binding and
/// one-shot actions are isolated here. Protocol adapters may hold this value;
/// they do not own or reimplement its state transitions.
#[derive(Debug)]
pub struct WorkspaceClientSession<T> {
    preflight_completed: bool,
    bootstrap_required: bool,
    preflight_agent: Option<String>,
    preflight_target: Option<String>,
    actions: SessionActionStore<T>,
    workspace: Arc<WorkspaceState>,
    session_id: String,
    host_home: PathBuf,
}

impl<T> WorkspaceClientSession<T> {
    pub fn new(workspace: Arc<WorkspaceState>, session_id: String, host_home: PathBuf) -> Self {
        Self {
            preflight_completed: false,
            bootstrap_required: false,
            preflight_agent: None,
            preflight_target: None,
            actions: SessionActionStore::for_session(&session_id),
            workspace,
            session_id,
            host_home,
        }
    }

    #[doc(hidden)]
    pub fn standalone(host_home: PathBuf) -> Self {
        Self {
            preflight_completed: false,
            bootstrap_required: false,
            preflight_agent: None,
            preflight_target: None,
            actions: SessionActionStore::default(),
            workspace: WorkspaceState::standalone(),
            session_id: format!("standalone-{}", std::process::id()),
            host_home,
        }
    }

    pub fn reset(&mut self) {
        self.preflight_completed = false;
        self.bootstrap_required = false;
        self.preflight_agent = None;
        self.preflight_target = None;
        self.actions.invalidate();
    }

    pub fn invalidate_actions(&mut self) {
        self.actions.invalidate();
    }

    pub fn mark_completed(&mut self, agent: Option<String>, target: Option<String>) {
        self.preflight_completed = true;
        self.bootstrap_required = false;
        self.preflight_agent = agent;
        self.preflight_target = target;
    }

    pub fn mark_bootstrap_required(&mut self, agent: Option<String>, target: Option<String>) {
        self.preflight_completed = false;
        self.bootstrap_required = agent.is_some() && target.is_some();
        self.preflight_agent = agent;
        self.preflight_target = target;
    }

    pub fn binding(&self) -> Option<PreflightBinding> {
        if !self.preflight_completed && !self.bootstrap_required {
            return None;
        }
        Some(PreflightBinding {
            host: self.preflight_agent.clone()?,
            target: self.preflight_target.as_deref()?.into(),
            host_home: self.host_home.clone(),
        })
    }

    pub fn is_preflight_completed(&self) -> bool {
        self.preflight_completed
    }

    pub fn is_bootstrap_required(&self) -> bool {
        self.bootstrap_required
    }

    pub fn preflight_agent(&self) -> Option<&str> {
        self.preflight_agent.as_deref()
    }

    pub fn preflight_target(&self) -> Option<&str> {
        self.preflight_target.as_deref()
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

    #[test]
    fn reset_clears_binding_and_invalidates_only_this_session() {
        let home = PathBuf::from("/tmp/ags-session-test-home");
        let mut first = WorkspaceClientSession::<()>::standalone(home.clone());
        let second = WorkspaceClientSession::<()>::standalone(home);
        first.mark_completed(Some("codex".to_string()), Some(".".to_string()));
        let first_generation = first.action_store().generation;
        let second_generation = second.action_store().generation;

        first.reset();

        assert!(first.binding().is_none());
        assert_eq!(first.action_store().generation, first_generation + 1);
        assert_eq!(second.action_store().generation, second_generation);
    }

    #[test]
    fn bootstrap_binding_is_explicit_and_session_local() {
        let mut session =
            WorkspaceClientSession::<()>::standalone(PathBuf::from("/tmp/ags-session-test-home"));
        session.mark_bootstrap_required(
            Some("claude-code".to_string()),
            Some("/tmp/project".to_string()),
        );

        assert!(session.is_bootstrap_required());
        assert!(!session.is_preflight_completed());
        assert_eq!(session.binding().unwrap().host, "claude-code");
    }
}
