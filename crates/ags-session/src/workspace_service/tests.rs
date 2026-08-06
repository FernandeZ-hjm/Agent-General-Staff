use super::*;
use crate::workspace_service::capability_snapshot::WorkspaceState;
use crate::workspace_service::registry_ownership::{
    acquire_workspace_owner, atomic_write_json, current_executable_hash,
    current_process_start_identity, ensure_private_dir, reclaim_stale_lock, workspace_key,
    ServicePaths, WorkspaceOwner, WorkspaceRegistry, REGISTRY_SCHEMA,
};
use crate::workspace_service::transport_handshake::{
    finish_workspace_session, handle_connection, inspect_existing_workspace_service_at,
    read_json_line, write_json_line, Handshake, HandshakeResult, WIRE_SCHEMA,
};
use crate::workspace_service::upgrade_recycle::connect_registered;
use crate::{CapabilityCatalogSource, PreflightBinding};
use ags_platform::canonical_workspace_root;
use std::fs;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[test]
fn instance_identity_is_only_the_canonical_workspace_path() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let child = workspace.join("nested");
    let sibling = root.path().join("other");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    fs::create_dir_all(&child).unwrap();
    fs::create_dir_all(sibling.join(".git")).unwrap();

    let canonical = canonical_workspace_root(&child).unwrap();
    let state = WorkspaceState::new(canonical.clone(), root.path().join("runtime")).unwrap();
    assert_eq!(state.instance_key(), workspace_key(&canonical));
    assert!(state.target_matches(&child));
    assert!(!state.target_matches(&sibling));
}

#[cfg(windows)]
#[test]
fn current_process_start_identity_is_stable_without_a_shell() {
    let first = current_process_start_identity().expect("current process identity");
    let second = current_process_start_identity().expect("current process identity");
    assert!(first.starts_with("filetime:"));
    assert_eq!(first, second);
}

#[test]
fn workspace_daemon_keeps_one_static_snapshot_for_its_lifetime() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = canonical_workspace_root(&root).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let runtime = fixture.path().join("runtime");
    let home = fixture.path().join("home");
    let initial = ags_capability_governance::write_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    let state = WorkspaceState::new(root.clone(), runtime.clone()).unwrap();
    let mut binding = PreflightBinding {
        host: "codex".to_string(),
        target: root.clone(),
        host_home: home.clone(),
        capability: None,
    };
    let accepted = state.capability_reference(&binding);
    assert_eq!(
        accepted
            .ready_binding()
            .map(|binding| binding.snapshot_hash.as_str()),
        Some(initial.snapshot_hash.as_str())
    );
    binding.capability = Some(accepted);

    let added = home.join(".codex/skills/new-after-daemon-start/SKILL.md");
    fs::create_dir_all(added.parent().unwrap()).unwrap();
    fs::write(
        &added,
        "---\nname: new-after-daemon-start\ndescription: refresh-only fixture\n---\n",
    )
    .unwrap();
    let refreshed = ags_capability_governance::write_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    assert_ne!(refreshed.snapshot_hash, initial.snapshot_hash);

    let loaded = state.read_catalog(&binding).unwrap();
    assert_eq!(loaded.snapshot_hash, initial.snapshot_hash);
}

#[test]
fn maintenance_activation_atomically_replaces_the_exact_host_snapshot_set() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = canonical_workspace_root(&root).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let runtime = fixture.path().join("runtime");
    let home = fixture.path().join("home");
    let snapshot = ags_capability_governance::write_capability_snapshot_with_roots(
        &root, "codex", &runtime, &home,
    )
    .unwrap();
    let state = WorkspaceState::new(root, runtime).unwrap();

    assert!(state.loaded_snapshot_hashes().unwrap().is_empty());
    let activated = state
        .activate_host_snapshots(&["codex".to_string()], &[], true)
        .unwrap();
    assert_eq!(activated.get("codex"), Some(&snapshot.snapshot_hash));
    assert_eq!(
        state.loaded_snapshot_hashes().unwrap().get("codex"),
        Some(&snapshot.snapshot_hash)
    );

    let before = state.loaded_snapshot_hashes().unwrap();
    assert!(state
        .activate_host_snapshots(&["codex".to_string(), "claude-code".to_string()], &[], true,)
        .is_err());
    assert_eq!(state.loaded_snapshot_hashes().unwrap(), before);

    state
        .activate_host_snapshots(&[], &["codex".to_string()], false)
        .unwrap();
    assert!(state.loaded_snapshot_hashes().unwrap().is_empty());
}

#[test]
fn abandoned_start_lock_is_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let lock = root.path().join("workspace.lock");
    fs::write(&lock, u32::MAX.to_string()).unwrap();
    reclaim_stale_lock(&lock);
    assert!(!lock.exists());
}

#[cfg(unix)]
#[test]
fn workspace_state_directory_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    fs::create_dir_all(&target).unwrap();
    symlink(&target, &link).unwrap();
    assert!(ensure_private_dir(&link)
        .unwrap_err()
        .contains("must not be a symlink"));
}

#[test]
fn daemon_owner_is_exclusive_for_the_workspace_lifetime() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    ensure_private_dir(&paths.dir).unwrap();

    let first = acquire_workspace_owner(&paths, &canonical).unwrap();
    assert_eq!(
        acquire_workspace_owner(&paths, &canonical).unwrap_err(),
        "workspace daemon already active"
    );
    drop(first);
    assert!(acquire_workspace_owner(&paths, &canonical).is_ok());
}

#[test]
fn incomplete_stale_owner_records_are_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    ensure_private_dir(&paths.dir).unwrap();

    for incomplete in [b"".as_slice(), br#"{"pid":"#] {
        fs::write(&paths.owner, incomplete).unwrap();
        let owner = acquire_workspace_owner(&paths, &canonical)
            .expect("incomplete owner record should be safely reclaimed");
        let published: WorkspaceOwner =
            serde_json::from_slice(&fs::read(&paths.owner).unwrap()).unwrap();
        assert_eq!(published.workspace, canonical);
        assert_eq!(published.pid, std::process::id());
        drop(owner);
        assert!(!paths.owner.exists());
    }
}

#[test]
fn reused_live_pid_with_different_start_identity_does_not_own_workspace() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    ensure_private_dir(&paths.dir).unwrap();
    let stale = WorkspaceOwner {
        pid: std::process::id(),
        token: "reused-pid-owner".to_string(),
        workspace: canonical.clone(),
        executable_hash: current_executable_hash().unwrap(),
        process_start_identity: "different-process-start".to_string(),
        daemon_nonce: "dead-daemon-nonce".to_string(),
    };
    fs::write(&paths.owner, serde_json::to_vec(&stale).unwrap()).unwrap();

    let owner = acquire_workspace_owner(&paths, &canonical)
        .expect("PID reuse must not permanently block workspace ownership");
    assert_ne!(owner.owner.daemon_nonce, stale.daemon_nonce);
}

#[test]
fn stale_registry_with_reused_live_pid_and_closed_endpoint_is_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    ensure_private_dir(&paths.dir).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    drop(listener);
    let stale = WorkspaceRegistry {
        schema_version: "retired-workspace-registry-schema".to_string(),
        workspace: canonical.clone(),
        instance_key: workspace_key(&canonical),
        endpoint,
        token: "stale-reused-pid".to_string(),
        pid: std::process::id(),
        executable_hash: "sha256:stale".to_string(),
        process_start_identity: String::new(),
        daemon_nonce: String::new(),
    };
    atomic_write_json(&paths.registry, &stale).unwrap();

    assert!(
        connect_registered(&paths, &canonical).unwrap().is_none(),
        "a dead daemon from a retired registry schema must not block workspace recovery"
    );
    assert!(!paths.registry.exists());
}

#[test]
fn reachable_reused_endpoint_is_reclaimed_after_handshake_mismatch() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    ensure_private_dir(&paths.dir).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: canonical.clone(),
        instance_key: workspace_key(&canonical),
        endpoint: listener.local_addr().unwrap().to_string(),
        token: "stale-reachable-token".to_string(),
        pid: std::process::id(),
        executable_hash: current_executable_hash().unwrap(),
        process_start_identity: "different-process-start".to_string(),
        daemon_nonce: "stale-daemon-nonce".to_string(),
    };
    atomic_write_json(&paths.registry, &registry).unwrap();

    let fake_registry = registry.clone();
    let fake_workspace = canonical.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let _: Handshake = read_json_line(&mut reader).unwrap();
        write_json_line(
            &mut stream,
            &HandshakeResult {
                status: "ready".to_string(),
                workspace: fake_workspace,
                instance_key: fake_registry.instance_key,
                executable_hash: fake_registry.executable_hash,
                process_start_identity: fake_registry.process_start_identity,
                daemon_nonce: "different-daemon-nonce".to_string(),
            },
        )
        .unwrap();
    });

    let (stream, connected_registry) = connect_registered(&paths, &canonical)
        .unwrap()
        .expect("the stale endpoint is reachable before authentication");
    let error =
        finish_workspace_session(stream, &connected_registry, &canonical, &paths).unwrap_err();
    assert_eq!(error, "workspace daemon handshake mismatch");
    server.join().unwrap();
    assert!(
        !paths.registry.exists(),
        "a reachable endpoint with a reused PID must not pin stale registry state"
    );
}

#[derive(Default)]
struct CapturingHandler {
    session_ids: Mutex<Vec<String>>,
}

impl WorkspaceSessionHandler for CapturingHandler {
    fn run(
        &self,
        _reader: BufReader<TcpStream>,
        _writer: TcpStream,
        _workspace: Arc<WorkspaceState>,
        session_id: String,
        _startup_executable_hash: String,
    ) {
        self.session_ids.lock().unwrap().push(session_id);
    }

    fn run_workspace_command(
        &self,
        kind: &str,
        _payload: serde_json::Value,
        workspace: Arc<WorkspaceState>,
    ) -> Result<serde_json::Value, String> {
        if kind != "status" {
            return Err(format!("unsupported workspace command `{kind}`"));
        }
        serde_json::to_value(WorkspaceServiceInspection {
            schema_version: WORKSPACE_DAEMON_STATUS_SCHEMA_VERSION.to_string(),
            canonical_workspace: workspace.root().to_string_lossy().to_string(),
            workspace_identity: workspace.instance_key().to_string(),
            loaded_snapshot_hashes: workspace.loaded_snapshot_hashes()?,
        })
        .map_err(|error| error.to_string())
    }
}

#[test]
fn existing_daemon_inspection_is_authenticated_and_never_mutates_registry() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let workspace = canonical_workspace_root(&workspace).unwrap();
    let state = Arc::new(WorkspaceState::new(workspace.clone(), runtime.clone()).unwrap());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: workspace.clone(),
        instance_key: workspace_key(&workspace),
        endpoint: listener.local_addr().unwrap().to_string(),
        token: "inspection-token".to_string(),
        pid: std::process::id(),
        executable_hash: current_executable_hash().unwrap(),
        process_start_identity: current_process_start_identity().unwrap(),
        daemon_nonce: "inspection-daemon-nonce".to_string(),
    };
    let paths = ServicePaths::new(&runtime, &workspace);
    ensure_private_dir(&paths.dir).unwrap();
    atomic_write_json(&paths.registry, &registry).unwrap();
    let registry_before = fs::read(&paths.registry).unwrap();

    let server_registry = registry.clone();
    let server_state = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_connection(
            stream,
            server_registry,
            server_state,
            Arc::new(AtomicBool::new(false)),
            Arc::new(CapturingHandler::default()),
        )
        .unwrap();
    });

    let inspection = inspect_existing_workspace_service_at(&runtime, &workspace)
        .unwrap()
        .expect("running daemon inspection");
    server.join().unwrap();
    assert_eq!(inspection.canonical_workspace, workspace.to_string_lossy());
    assert_eq!(inspection.workspace_identity, workspace_key(&workspace));
    assert!(inspection.loaded_snapshot_hashes.is_empty());
    assert_eq!(fs::read(&paths.registry).unwrap(), registry_before);
    assert!(inspect_existing_workspace_service_at(&runtime, &workspace).is_err());
    assert_eq!(fs::read(&paths.registry).unwrap(), registry_before);

    fs::remove_file(&paths.registry).unwrap();
    assert!(inspect_existing_workspace_service_at(&runtime, &workspace)
        .unwrap()
        .is_none());
    assert!(!paths.registry.exists());
}

#[test]
fn daemon_mints_sessions_and_handshake_proves_process_identity() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let workspace = canonical_workspace_root(&workspace).unwrap();
    let state = Arc::new(WorkspaceState::new(workspace.clone(), runtime).unwrap());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: workspace.clone(),
        instance_key: workspace_key(&workspace),
        endpoint: endpoint.clone(),
        token: "review-token".to_string(),
        pid: std::process::id(),
        executable_hash: current_executable_hash().unwrap(),
        process_start_identity: current_process_start_identity().unwrap(),
        daemon_nonce: "review-daemon-nonce".to_string(),
    };
    let handler = Arc::new(CapturingHandler::default());
    let server_handler = Arc::clone(&handler);
    let server_state = Arc::clone(&state);
    let server_registry = registry.clone();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(
                stream,
                server_registry.clone(),
                Arc::clone(&server_state),
                Arc::new(AtomicBool::new(false)),
                server_handler.clone(),
            )
            .unwrap();
        }
    });

    for _ in 0..2 {
        let mut stream = TcpStream::connect(&endpoint).unwrap();
        write_json_line(
            &mut stream,
            &Handshake {
                protocol: WIRE_SCHEMA.to_string(),
                token: registry.token.clone(),
                kind: "session".to_string(),
                command: None,
                workspace: workspace.clone(),
            },
        )
        .unwrap();
        let mut reader = BufReader::new(stream);
        let ready: HandshakeResult = read_json_line(&mut reader).unwrap();
        assert_eq!(ready.executable_hash, registry.executable_hash);
        assert_eq!(
            ready.process_start_identity,
            registry.process_start_identity
        );
        assert_eq!(ready.daemon_nonce, registry.daemon_nonce);
    }
    server.join().unwrap();
    let mut captured = handler.session_ids.lock().unwrap().clone();
    captured.sort();
    captured.dedup();
    assert_eq!(captured.len(), 2);
}

#[test]
fn workspace_daemon_rejects_a_wrong_handshake_token() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let workspace = canonical_workspace_root(&workspace).unwrap();
    let state = Arc::new(WorkspaceState::new(workspace.clone(), runtime).unwrap());
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let registry = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: workspace.clone(),
        instance_key: workspace_key(&workspace),
        endpoint: endpoint.clone(),
        token: "expected-token".to_string(),
        pid: std::process::id(),
        executable_hash: current_executable_hash().unwrap(),
        process_start_identity: current_process_start_identity().unwrap(),
        daemon_nonce: "expected-daemon-nonce".to_string(),
    };
    let server_registry = registry.clone();
    let server_workspace = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_connection(
            stream,
            server_registry,
            server_workspace,
            Arc::new(AtomicBool::new(false)),
            Arc::new(CapturingHandler::default()),
        )
        .unwrap_err()
    });

    let mut stream = TcpStream::connect(endpoint).unwrap();
    write_json_line(
        &mut stream,
        &Handshake {
            protocol: WIRE_SCHEMA.to_string(),
            token: "wrong-token".to_string(),
            kind: "session".to_string(),
            command: None,
            workspace,
        },
    )
    .unwrap();
    drop(stream);

    assert_eq!(
        server.join().unwrap(),
        "workspace daemon authentication failed"
    );
}
