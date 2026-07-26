use super::*;
use crate::workspace_service::capability_bundle::WorkspaceState;
use crate::workspace_service::registry_ownership::{
    acquire_workspace_owner, atomic_write_json, current_executable_hash,
    current_process_start_identity, ensure_private_dir, reclaim_stale_lock, workspace_key,
    ServicePaths, WorkspaceOwner, WorkspaceRegistry, REGISTRY_SCHEMA,
};
use crate::workspace_service::transport_handshake::{
    handle_connection, read_json_line, write_json_line, Handshake, HandshakeResult, WIRE_SCHEMA,
};
use crate::workspace_service::upgrade_recycle::connect_registered;
use ags_platform::canonical_workspace_root;
use std::fs;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
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

#[test]
fn abandoned_start_lock_is_reclaimed() {
    let root = tempfile::tempdir().unwrap();
    let lock = root.path().join("workspace.lock");
    fs::write(&lock, u32::MAX.to_string()).unwrap();
    reclaim_stale_lock(&lock);
    assert!(!lock.exists());
}

#[test]
fn corrupt_capability_bundle_fails_before_daemon_readiness() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    fs::create_dir_all(&paths.dir).unwrap();
    fs::write(&paths.capabilities, b"{not-json").unwrap();

    let error = WorkspaceState::new(canonical, runtime).unwrap_err();
    assert!(error.contains("workspace capability bundle corrupt"));
    assert!(!paths.registry.exists());
}

#[test]
fn capability_bundle_binding_mismatch_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let runtime = root.path().join("runtime");
    fs::create_dir_all(workspace.join(".git")).unwrap();
    let canonical = canonical_workspace_root(&workspace).unwrap();
    let paths = ServicePaths::new(&runtime, &canonical);
    fs::create_dir_all(&paths.dir).unwrap();
    let bundle = serde_json::json!({
        "schema_version": "0.3.0-workspace-capabilities",
        "workspace": root.path().join("other"),
        "workspace_identity": "wrong",
        "epoch": 1,
        "host_epochs": {},
        "snapshots": {}
    });
    fs::write(&paths.capabilities, serde_json::to_vec(&bundle).unwrap()).unwrap();

    assert!(WorkspaceState::new(canonical, runtime)
        .unwrap_err()
        .contains("workspace capability bundle binding invalid"));
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
fn incomplete_legacy_owner_records_are_reclaimed() {
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
fn legacy_registry_with_reused_live_pid_and_closed_endpoint_is_reclaimed() {
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
    let legacy = WorkspaceRegistry {
        schema_version: REGISTRY_SCHEMA.to_string(),
        workspace: canonical.clone(),
        instance_key: workspace_key(&canonical),
        endpoint,
        token: "legacy-reused-pid".to_string(),
        pid: std::process::id(),
        executable_hash: "sha256:legacy".to_string(),
        process_start_identity: String::new(),
        daemon_nonce: String::new(),
        version: "0.3.1".to_string(),
    };
    atomic_write_json(&paths.registry, &legacy).unwrap();

    assert!(
        connect_registered(&paths, &canonical).unwrap().is_none(),
        "an unauthenticated legacy endpoint must not block workspace recovery"
    );
    assert!(!paths.registry.exists());
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
    ) {
        self.session_ids.lock().unwrap().push(session_id);
    }
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
        version: env!("CARGO_PKG_VERSION").to_string(),
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

    let mut issued = Vec::new();
    for _ in 0..2 {
        let mut stream = TcpStream::connect(&endpoint).unwrap();
        write_json_line(
            &mut stream,
            &Handshake {
                protocol: WIRE_SCHEMA.to_string(),
                token: registry.token.clone(),
                kind: "session".to_string(),
                session_id: Some("client-chosen-collision".to_string()),
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
        issued.push(ready.session_id.unwrap());
    }
    server.join().unwrap();
    issued.sort();
    issued.dedup();
    assert_eq!(issued.len(), 2);
    let mut captured = handler.session_ids.lock().unwrap().clone();
    captured.sort();
    captured.dedup();
    assert_eq!(captured, issued);
}
