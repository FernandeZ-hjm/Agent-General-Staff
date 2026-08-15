//! Request-scoped workspace resolution and per-workspace authenticated routing.
//!
//! The resolver's inputs are explicit. It never reads daemon cwd, HOME,
//! managed-projects, recently used projects, or fuzzy path matches.

use ags_platform::canonical_workspace_root;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

pub const MAX_WORKSPACE_SESSIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    #[serde(default)]
    pub mcp_roots: Vec<PathBuf>,
    pub adapter_cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResolutionError {
    pub code: &'static str,
    pub detail: String,
    pub candidates: Vec<PathBuf>,
}

impl fmt::Display for WorkspaceResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for WorkspaceResolutionError {}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceResolver;

impl WorkspaceResolver {
    pub fn resolve(&self, context: &WorkspaceContext) -> Result<PathBuf, WorkspaceResolutionError> {
        if let Some(explicit) = &context.workspace {
            let target = if explicit.is_absolute() {
                explicit.clone()
            } else {
                context.adapter_cwd.join(explicit)
            };
            return require_ags_workspace(&target, "explicit context.workspace");
        }

        let mut roots = canonical_mcp_roots(&context.mcp_roots)?;
        match roots.len() {
            1 => return Ok(roots.remove(0)),
            count if count > 1 => {
                roots.sort();
                return Err(WorkspaceResolutionError {
                    code: "workspace_ambiguous",
                    detail: "multiple canonical AGS workspaces were declared as MCP roots"
                        .to_string(),
                    candidates: roots,
                });
            }
            _ => {}
        }

        if let Some(canonical) = find_ags_workspace(&context.adapter_cwd)? {
            return Ok(canonical);
        }

        Err(WorkspaceResolutionError {
            code: "workspace_required",
            detail: "provide context.workspace, one unique MCP root, or an adapter cwd inside one AGS workspace".to_string(),
            candidates: Vec::new(),
        })
    }
}

fn canonical_mcp_roots(
    declared_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, WorkspaceResolutionError> {
    let mut roots = Vec::new();
    let mut identities = HashSet::new();
    for root in declared_roots {
        if let Some(canonical) = find_ags_workspace(root)? {
            if identities.insert(canonical.clone()) {
                roots.push(canonical);
            }
        }
    }
    Ok(roots)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBinding {
    connection_id: String,
    host_id: String,
    canonical_workspace: PathBuf,
    workspace_identity: String,
    project_facts_hash: String,
    registry_key: String,
    authenticated_session: String,
}

impl WorkspaceBinding {
    pub fn connection_id(&self) -> &str {
        &self.connection_id
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
    pub fn authenticated_session(&self) -> &str {
        &self.authenticated_session
    }
}

#[derive(Debug)]
pub struct AuthenticatedWorkspaceSession<S> {
    canonical_workspace: PathBuf,
    workspace_identity: String,
    project_facts_hash: String,
    registry_key: String,
    session_id: String,
    session: S,
}

impl<S> AuthenticatedWorkspaceSession<S> {
    pub fn new(
        canonical_workspace: impl Into<PathBuf>,
        workspace_identity: impl Into<String>,
        project_facts_hash: impl Into<String>,
        registry_key: impl Into<String>,
        session_id: impl Into<String>,
        session: S,
    ) -> Result<Self, WorkspaceResolutionError> {
        let canonical_workspace = canonical_workspace.into();
        let workspace_identity = workspace_identity.into();
        let project_facts_hash = project_facts_hash.into();
        let registry_key = registry_key.into();
        let session_id = session_id.into();
        if workspace_identity.trim().is_empty()
            || !ags_platform::is_sha256(&project_facts_hash)
            || registry_key.trim().is_empty()
            || session_id.trim().is_empty()
        {
            return Err(WorkspaceResolutionError {
                code: "workspace_session_unauthenticated",
                detail: "authenticated handshake must provide workspace identity, project facts hash, registry key, and session id".to_string(),
                candidates: Vec::new(),
            });
        }
        Ok(Self {
            canonical_workspace,
            workspace_identity,
            project_facts_hash,
            registry_key,
            session_id,
            session,
        })
    }
}

#[derive(Debug)]
struct RoutedSession<S> {
    binding: WorkspaceBinding,
    session: S,
}

/// One transport connection with isolated sessions keyed by canonical
/// workspace. There is intentionally no mutable `current_workspace` field.
#[derive(Debug)]
pub struct WorkspaceRouter<S> {
    resolver: WorkspaceResolver,
    connection_id: String,
    host_id: String,
    sessions: HashMap<PathBuf, RoutedSession<S>>,
    access_order: VecDeque<PathBuf>,
    handshake_count: usize,
    open_count: usize,
    warm_hit_count: usize,
    stale_reopen_count: usize,
    eviction_count: usize,
}

impl<S> WorkspaceRouter<S> {
    pub fn new(
        connection_id: impl Into<String>,
        host_id: impl Into<String>,
    ) -> Result<Self, WorkspaceResolutionError> {
        let connection_id = connection_id.into();
        let host_id = host_id.into();
        if connection_id.trim().is_empty() || host_id.trim().is_empty() {
            return Err(WorkspaceResolutionError {
                code: "workspace_router_binding_invalid",
                detail: "connection_id and host_id must not be empty".to_string(),
                candidates: Vec::new(),
            });
        }
        Ok(Self {
            resolver: WorkspaceResolver,
            connection_id,
            host_id,
            sessions: HashMap::new(),
            access_order: VecDeque::new(),
            handshake_count: 0,
            open_count: 0,
            warm_hit_count: 0,
            stale_reopen_count: 0,
            eviction_count: 0,
        })
    }

    pub fn resolve(&self, context: &WorkspaceContext) -> Result<PathBuf, WorkspaceResolutionError> {
        self.resolver.resolve(context)
    }

    pub fn open_workspace<F>(
        &mut self,
        context: &WorkspaceContext,
        open: F,
    ) -> Result<WorkspaceBinding, WorkspaceResolutionError>
    where
        F: FnOnce(&Path) -> Result<AuthenticatedWorkspaceSession<S>, WorkspaceResolutionError>,
    {
        let canonical_workspace = self.resolve(context)?;
        let warm_binding = self
            .sessions
            .get(&canonical_workspace)
            .filter(|existing| {
                existing.binding.project_facts_hash
                    == crate::workspace_service::project_facts_hash_at(&canonical_workspace)
            })
            .map(|existing| existing.binding.clone());
        if let Some(binding) = warm_binding {
            self.warm_hit_count = self.warm_hit_count.saturating_add(1);
            self.touch(&canonical_workspace);
            return Ok(binding);
        }
        if self.sessions.remove(&canonical_workspace).is_some() {
            self.access_order
                .retain(|candidate| candidate != &canonical_workspace);
            self.stale_reopen_count = self.stale_reopen_count.saturating_add(1);
        }

        let opened = open(&canonical_workspace)?;
        let handshake_workspace = opened.canonical_workspace.canonicalize().map_err(|error| {
            WorkspaceResolutionError {
                code: "workspace_handshake_invalid",
                detail: error.to_string(),
                candidates: vec![canonical_workspace.clone()],
            }
        })?;
        if handshake_workspace != canonical_workspace {
            return Err(WorkspaceResolutionError {
                code: "workspace_handshake_mismatch",
                detail: "authenticated daemon response names a different canonical workspace"
                    .to_string(),
                candidates: vec![canonical_workspace, handshake_workspace],
            });
        }
        if self.sessions.values().any(|session| {
            session.binding.authenticated_session == opened.session_id
                && session.binding.canonical_workspace != canonical_workspace
        }) {
            return Err(WorkspaceResolutionError {
                code: "workspace_session_cross_binding",
                detail: "one authenticated session id cannot be reused across canonical workspaces"
                    .to_string(),
                candidates: vec![canonical_workspace],
            });
        }
        let binding = WorkspaceBinding {
            connection_id: self.connection_id.clone(),
            host_id: self.host_id.clone(),
            canonical_workspace: canonical_workspace.clone(),
            workspace_identity: opened.workspace_identity,
            project_facts_hash: opened.project_facts_hash,
            registry_key: opened.registry_key,
            authenticated_session: opened.session_id,
        };
        if self.sessions.len() == MAX_WORKSPACE_SESSIONS {
            let oldest = self
                .access_order
                .pop_front()
                .expect("a full session cache has an oldest workspace");
            self.sessions.remove(&oldest);
            self.eviction_count = self.eviction_count.saturating_add(1);
        }
        self.sessions.insert(
            canonical_workspace.clone(),
            RoutedSession {
                binding: binding.clone(),
                session: opened.session,
            },
        );
        self.access_order.push_back(canonical_workspace);
        self.handshake_count = self.handshake_count.saturating_add(1);
        self.open_count = self.open_count.saturating_add(1);
        Ok(binding)
    }

    pub fn session(&self, binding: &WorkspaceBinding) -> Result<&S, WorkspaceResolutionError> {
        self.validate_binding(binding)?;
        Ok(&self
            .sessions
            .get(&binding.canonical_workspace)
            .expect("validated routed session")
            .session)
    }

    pub fn session_mut(
        &mut self,
        binding: &WorkspaceBinding,
    ) -> Result<&mut S, WorkspaceResolutionError> {
        self.validate_binding(binding)?;
        Ok(&mut self
            .sessions
            .get_mut(&binding.canonical_workspace)
            .expect("validated routed session")
            .session)
    }

    pub fn workspace_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn open_count(&self) -> usize {
        self.open_count
    }

    pub fn handshake_count(&self) -> usize {
        self.handshake_count
    }

    pub fn warm_hit_count(&self) -> usize {
        self.warm_hit_count
    }

    pub fn stale_reopen_count(&self) -> usize {
        self.stale_reopen_count
    }

    pub fn eviction_count(&self) -> usize {
        self.eviction_count
    }

    fn touch(&mut self, workspace: &Path) {
        if let Some(position) = self
            .access_order
            .iter()
            .position(|candidate| candidate == workspace)
        {
            self.access_order.remove(position);
        }
        self.access_order.push_back(workspace.to_path_buf());
    }

    fn validate_binding(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceResolutionError> {
        let Some(routed) = self.sessions.get(&binding.canonical_workspace) else {
            return Err(cross_binding_error(binding));
        };
        if routed.binding != *binding
            || binding.connection_id != self.connection_id
            || binding.host_id != self.host_id
        {
            return Err(cross_binding_error(binding));
        }
        Ok(())
    }
}

fn cross_binding_error(binding: &WorkspaceBinding) -> WorkspaceResolutionError {
    WorkspaceResolutionError {
        code: "workspace_binding_rejected",
        detail: "binding belongs to a different connection, host, canonical workspace, or authenticated session".to_string(),
        candidates: vec![binding.canonical_workspace.clone()],
    }
}

fn require_ags_workspace(path: &Path, source: &str) -> Result<PathBuf, WorkspaceResolutionError> {
    find_ags_workspace(path)?.ok_or_else(|| WorkspaceResolutionError {
        code: "workspace_required",
        detail: format!("{source} does not identify an AGS workspace"),
        candidates: Vec::new(),
    })
}

fn find_ags_workspace(path: &Path) -> Result<Option<PathBuf>, WorkspaceResolutionError> {
    let canonical = canonical_workspace_root(path).map_err(|error| WorkspaceResolutionError {
        code: "workspace_required",
        detail: error,
        candidates: Vec::new(),
    })?;
    Ok(is_ags_workspace(&canonical).then_some(canonical))
}

fn is_ags_workspace(root: &Path) -> bool {
    let repository = root.join(".git").exists();
    let governed_entry = root.join("AGENTS.md").is_file();
    let suite = root.join("AGENT_SUITE_PROTOCOL.md").is_file()
        && root.join("manifests/suite.yaml").is_file();
    let integrated = root.join("protocol/agent-task-protocol.md").is_file()
        || root.join("config/agent-project-profile.yaml").is_file();
    repository && governed_entry && (suite || integrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workspace(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.join(".git")).unwrap();
        fs::create_dir_all(path.join("protocol")).unwrap();
        fs::write(path.join("AGENTS.md"), "# governed\n").unwrap();
        fs::write(path.join("protocol/agent-task-protocol.md"), "# protocol\n").unwrap();
        path.canonicalize().unwrap()
    }

    fn context(workspace: Option<PathBuf>, roots: Vec<PathBuf>, cwd: PathBuf) -> WorkspaceContext {
        WorkspaceContext {
            workspace,
            mcp_roots: roots,
            adapter_cwd: cwd,
        }
    }

    #[test]
    fn resolution_order_is_explicit_then_unique_root_then_adapter_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let resolver = WorkspaceResolver;
        assert_eq!(
            resolver
                .resolve(&context(
                    Some(a.clone()),
                    vec![a.clone(), b.clone()],
                    b.clone()
                ))
                .unwrap(),
            a
        );
        assert_eq!(
            resolver
                .resolve(&context(None, vec![b.clone()], temp.path().to_path_buf()))
                .unwrap(),
            b
        );
        assert_eq!(
            resolver
                .resolve(&context(None, vec![], a.join("nested")))
                .unwrap_err()
                .code,
            "workspace_required"
        );
        fs::create_dir_all(a.join("nested")).unwrap();
        assert_eq!(
            resolver
                .resolve(&context(None, vec![], a.join("nested")))
                .unwrap(),
            a
        );
    }

    #[test]
    fn explicit_workspace_is_resolved_independently_of_discovery_roots() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let resolved = WorkspaceResolver
            .resolve(&context(
                Some(b.clone()),
                vec![a],
                temp.path().to_path_buf(),
            ))
            .unwrap();
        assert_eq!(resolved, b);
    }

    #[test]
    fn explicit_relative_workspace_uses_adapter_cwd_even_with_a_different_root_hint() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let resolved = WorkspaceResolver
            .resolve(&context(Some(PathBuf::from(".")), vec![a], b.clone()))
            .unwrap();
        assert_eq!(resolved, b);
    }

    #[test]
    fn explicit_workspace_does_not_depend_on_root_hint_validity() {
        let temp = tempfile::tempdir().unwrap();
        let b = workspace(temp.path(), "b");
        let missing_root = temp.path().join("root-that-no-longer-exists");
        let resolved = WorkspaceResolver
            .resolve(&context(
                Some(b.clone()),
                vec![missing_root],
                temp.path().to_path_buf(),
            ))
            .unwrap();
        assert_eq!(resolved, b);
    }

    #[test]
    fn ambiguous_roots_and_home_like_cwd_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let resolver = WorkspaceResolver;
        assert_eq!(
            resolver
                .resolve(&context(None, vec![a, b], temp.path().to_path_buf()))
                .unwrap_err()
                .code,
            "workspace_ambiguous"
        );
        assert_eq!(
            resolver
                .resolve(&context(None, vec![], temp.path().to_path_buf()))
                .unwrap_err()
                .code,
            "workspace_required"
        );
    }

    #[test]
    fn one_connection_reuses_independent_a_b_a_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let mut router = WorkspaceRouter::new("connection-1", "hermes").unwrap();
        let a_binding = router
            .open_workspace(
                &context(Some(a.clone()), vec![], temp.path().to_path_buf()),
                |_| {
                    AuthenticatedWorkspaceSession::new(
                        a.clone(),
                        "identity-a",
                        crate::workspace_service::project_facts_hash_at(&a),
                        "registry-a",
                        "session-a",
                        "transport-a",
                    )
                },
            )
            .unwrap();
        let b_binding = router
            .open_workspace(
                &context(Some(b.clone()), vec![], temp.path().to_path_buf()),
                |_| {
                    AuthenticatedWorkspaceSession::new(
                        b.clone(),
                        "identity-b",
                        crate::workspace_service::project_facts_hash_at(&b),
                        "registry-b",
                        "session-b",
                        "transport-b",
                    )
                },
            )
            .unwrap();
        let a_again = router
            .open_workspace(&context(Some(a), vec![], temp.path().to_path_buf()), |_| {
                panic!("A session must be reused")
            })
            .unwrap();
        assert_eq!(a_binding, a_again);
        assert_ne!(
            a_binding.authenticated_session(),
            b_binding.authenticated_session()
        );
        assert_eq!(*router.session(&a_binding).unwrap(), "transport-a");
        assert_eq!(*router.session(&b_binding).unwrap(), "transport-b");
        assert_eq!(router.workspace_count(), 2);
        assert_eq!(router.handshake_count(), 2);
        assert_eq!(router.open_count(), 2);
        assert_eq!(router.warm_hit_count(), 1);
    }

    #[test]
    fn one_root_discovers_a_but_explicit_b_authenticates_and_routes_independently() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let b = workspace(temp.path(), "b");
        let mut router = WorkspaceRouter::new("connection-1", "hermes").unwrap();

        let a_binding = router
            .open_workspace(
                &context(None, vec![a.clone()], temp.path().to_path_buf()),
                |canonical| {
                    assert_eq!(canonical, a);
                    AuthenticatedWorkspaceSession::new(
                        canonical,
                        "identity-a",
                        crate::workspace_service::project_facts_hash_at(canonical),
                        "registry-a",
                        "session-a",
                        "transport-a",
                    )
                },
            )
            .unwrap();
        let b_binding = router
            .open_workspace(
                &context(Some(b.clone()), vec![a.clone()], temp.path().to_path_buf()),
                |canonical| {
                    assert_eq!(canonical, b);
                    AuthenticatedWorkspaceSession::new(
                        canonical,
                        "identity-b",
                        crate::workspace_service::project_facts_hash_at(canonical),
                        "registry-b",
                        "session-b",
                        "transport-b",
                    )
                },
            )
            .unwrap();
        let a_again = router
            .open_workspace(&context(None, vec![a], temp.path().to_path_buf()), |_| {
                panic!("the root-discovered A session must be reused")
            })
            .unwrap();

        assert_eq!(a_binding, a_again);
        assert_eq!(*router.session(&a_binding).unwrap(), "transport-a");
        assert_eq!(*router.session(&b_binding).unwrap(), "transport-b");
        assert_eq!(router.workspace_count(), 2);
        assert_eq!(router.open_count(), 2);
        assert_eq!(router.warm_hit_count(), 1);
    }

    #[test]
    fn cross_connection_and_cross_host_bindings_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let ctx = context(Some(a), vec![], temp.path().to_path_buf());
        let mut first = WorkspaceRouter::new("connection-1", "hermes").unwrap();
        let binding = first
            .open_workspace(&ctx, |workspace| {
                AuthenticatedWorkspaceSession::new(
                    workspace,
                    "identity-a",
                    ags_platform::sha256("facts-a"),
                    "registry-a",
                    "session-a",
                    (),
                )
            })
            .unwrap();
        let second = WorkspaceRouter::<()>::new("connection-2", "hermes").unwrap();
        assert_eq!(
            second.session(&binding).unwrap_err().code,
            "workspace_binding_rejected"
        );
        let other_host = WorkspaceRouter::<()>::new("connection-1", "codex").unwrap();
        assert_eq!(
            other_host.session(&binding).unwrap_err().code,
            "workspace_binding_rejected"
        );
    }

    #[test]
    fn unchanged_facts_reuse_the_immutable_warm_session_without_git_processes() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let ctx = context(Some(a.clone()), vec![], temp.path().to_path_buf());
        let mut router = WorkspaceRouter::new("connection-1", "hermes").unwrap();
        let first = router
            .open_workspace(&ctx, |workspace| {
                AuthenticatedWorkspaceSession::new(
                    workspace,
                    "identity-a",
                    crate::workspace_service::project_facts_hash_at(workspace),
                    "registry-a",
                    "session-a",
                    "transport-a",
                )
            })
            .unwrap();
        let second = router
            .open_workspace(&ctx, |_| {
                panic!("unchanged facts must reuse the warm session")
            })
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(*router.session(&second).unwrap(), "transport-a");
        assert_eq!(router.open_count(), 1);
        assert_eq!(router.warm_hit_count(), 1);

        let facts_source = include_str!("workspace_service/capability_snapshot.rs");
        let forbidden = ["Command", "::new(\"git\")"].concat();
        assert!(!facts_source.contains(&forbidden));
    }

    #[test]
    fn changed_daemon_facts_reopen_only_that_workspace_session_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let a = workspace(temp.path(), "a");
        let ctx = context(Some(a.clone()), vec![], temp.path().to_path_buf());
        let mut router = WorkspaceRouter::new("connection-1", "hermes").unwrap();
        let first = router
            .open_workspace(&ctx, |workspace| {
                AuthenticatedWorkspaceSession::new(
                    workspace,
                    "identity-a",
                    crate::workspace_service::project_facts_hash_at(workspace),
                    "registry-a",
                    "session-a",
                    "transport-a",
                )
            })
            .unwrap();

        fs::write(a.join("AGENTS.md"), "# governed v2\n").unwrap();
        let second = router
            .open_workspace(&ctx, |workspace| {
                AuthenticatedWorkspaceSession::new(
                    workspace,
                    "identity-a",
                    crate::workspace_service::project_facts_hash_at(workspace),
                    "registry-a",
                    "session-b",
                    "transport-b",
                )
            })
            .unwrap();

        assert_ne!(first.project_facts_hash(), second.project_facts_hash());
        assert_ne!(
            first.authenticated_session(),
            second.authenticated_session()
        );
        assert_eq!(*router.session(&second).unwrap(), "transport-b");
        assert_eq!(
            router.session(&first).unwrap_err().code,
            "workspace_binding_rejected"
        );
        assert_eq!(router.handshake_count(), 2);
        assert_eq!(router.open_count(), 2);
        assert_eq!(router.stale_reopen_count(), 1);
    }

    #[test]
    fn workspace_session_cache_is_bounded_and_evicts_lru() {
        let temp = tempfile::tempdir().unwrap();
        let mut router = WorkspaceRouter::new("connection-1", "hermes").unwrap();
        let mut first_binding = None;
        for index in 0..=MAX_WORKSPACE_SESSIONS {
            let root = workspace(temp.path(), &format!("workspace-{index}"));
            let binding = router
                .open_workspace(
                    &context(Some(root.clone()), vec![], temp.path().to_path_buf()),
                    |canonical| {
                        AuthenticatedWorkspaceSession::new(
                            canonical,
                            format!("identity-{index}"),
                            ags_platform::sha256(format!("facts-{index}")),
                            format!("registry-{index}"),
                            format!("session-{index}"),
                            index,
                        )
                    },
                )
                .unwrap();
            first_binding.get_or_insert(binding);
        }

        assert_eq!(router.workspace_count(), MAX_WORKSPACE_SESSIONS);
        assert_eq!(router.open_count(), MAX_WORKSPACE_SESSIONS + 1);
        assert_eq!(router.eviction_count(), 1);
        assert_eq!(
            router
                .session(first_binding.as_ref().unwrap())
                .unwrap_err()
                .code,
            "workspace_binding_rejected"
        );
    }
}
