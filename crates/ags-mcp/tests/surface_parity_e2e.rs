use ags_cli::{execute, Cli, Invocation, ParsedInvocation};
use ags_control_plane::{Decision, OperationReceipt, OperationRequest, OperationState};
use ags_mcp::contract_v2::{Connection, DecideArguments, WorkspaceRpcPort};
use serde_json::json;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CHILD_MODE: &str = "AGS_SURFACE_PARITY_CHILD_MODE";
const CHILD_ARGV: &str = "AGS_SURFACE_PARITY_CHILD_ARGV";
const CHILD_RESULT: &str = "AGS_SURFACE_PARITY_RESULT:";

struct CompanionExecutable {
    path: PathBuf,
}

impl CompanionExecutable {
    fn install() -> Self {
        let source = PathBuf::from(env!("CARGO_BIN_EXE_ags-mcp"));
        let path = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("ags-mcp{}", std::env::consts::EXE_SUFFIX));
        match std::fs::hard_link(&source, &path) {
            Ok(()) => Self { path },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                assert_eq!(
                    ags_platform::sha256(std::fs::read(&source).unwrap()),
                    ags_platform::sha256(std::fs::read(&path).unwrap()),
                    "existing test companion differs from the Cargo-built ags-mcp"
                );
                Self { path }
            }
            Err(error) => panic!("cannot install test companion executable: {error}"),
        }
    }
}

fn workspace(parent: &Path) -> PathBuf {
    let path = parent.join("workspace");
    std::fs::create_dir_all(path.join("config")).unwrap();
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&path)
        .status()
        .unwrap()
        .success());
    std::fs::write(path.join("AGENTS.md"), "# governed\n").unwrap();
    std::fs::write(
        path.join("config/agent-project-profile.yaml"),
        "schema_version: ags://schema/contract/v2/project-profile\n",
    )
    .unwrap();
    path.canonicalize().unwrap()
}

fn wait_for_registry(runtime: &Path, daemon: &mut Child) {
    // External-volume macOS builds can spend tens of seconds in dyld/XProtect
    // before the Cargo-built companion reaches AGS startup. This is a harness
    // budget only; production workspace-service deadlines remain unchanged.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if std::fs::read_dir(runtime.join("workspace-services"))
            .ok()
            .is_some_and(|entries| {
                entries.flatten().any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
            })
        {
            return;
        }
        if let Some(status) = daemon.try_wait().unwrap() {
            let mut stdout = String::new();
            let mut stderr = String::new();
            daemon
                .stdout
                .take()
                .unwrap()
                .read_to_string(&mut stdout)
                .unwrap();
            daemon
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("workspace daemon exited {status}:\nstdout={stdout}\nstderr={stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("workspace daemon registry was not published");
}

fn parsed_operation(parsed: &ParsedInvocation) -> &OperationRequest {
    let Invocation::Decide(operation) = &parsed.invocation else {
        panic!("fixture must route through decide")
    };
    operation
}

fn request_hash(operation: &OperationRequest) -> String {
    ags_platform::sha256(serde_json::to_vec(operation).unwrap())
}

fn receipt_hash(receipt: &OperationReceipt) -> String {
    // Binding identity deliberately differs between CLI and MCP. This hash is
    // the transport-independent receipt contract whose identity must be equal.
    ags_platform::sha256(
        serde_json::to_vec(&json!({
            "schema_version": receipt.schema_version,
            "receipt_id": receipt.receipt_id,
            "operation": receipt.operation,
            "status": receipt.status,
            "plan_hash": receipt.plan_hash,
            "payload_hash": receipt.payload_hash,
            "output_digest": receipt.output_digest,
            "observed_write_set": receipt.observed_write_set,
            "recovered": receipt.recovered,
            "evidence": receipt.evidence,
        }))
        .unwrap(),
    )
}

#[test]
fn adapter_child_process() {
    let Ok(mode) = std::env::var(CHILD_MODE) else {
        return;
    };
    match mode.as_str() {
        "cli" => {
            let argv: Vec<String> =
                serde_json::from_str(&std::env::var(CHILD_ARGV).unwrap()).unwrap();
            let parsed = Cli::try_parse_from(argv).unwrap().into_invocation();
            let cwd = std::env::current_dir().unwrap();
            let result = execute(parsed, cwd).unwrap();
            println!("{CHILD_RESULT}{}", serde_json::to_string(&result).unwrap());
        }
        mode => panic!("unknown child mode {mode}"),
    }
}

fn child_command(mode: &str, runtime: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "adapter_child_process",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_MODE, mode)
        .env("AGS_RUNTIME_HOME", runtime)
        .env("AGS_WORKSPACE_IDLE_MS", "1500");
    command
}

fn start_daemon(executable: &Path, workspace: &Path, runtime: &Path) -> Child {
    Command::new(executable)
        .args(["daemon", "--workspace"])
        .arg(workspace)
        .env("AGS_RUNTIME_HOME", runtime)
        .env("AGS_WORKSPACE_IDLE_MS", "1500")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn execute_cli(argv: Vec<String>, cwd: &Path, runtime: &Path) -> String {
    let output = child_command("cli", runtime)
        .env(CHILD_ARGV, serde_json::to_string(&argv).unwrap())
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "CLI child failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let encoded = stdout
        .lines()
        .find_map(|line| {
            line.find(CHILD_RESULT)
                .map(|index| &line[index + CHILD_RESULT.len()..])
        })
        .unwrap_or_else(|| panic!("CLI child emitted no result: {stdout}"));
    let (rendered, succeeded): (String, bool) = serde_json::from_str(encoded).unwrap();
    assert!(succeeded, "CLI surface reported failure: {rendered}");
    rendered
}

#[test]
fn human_machine_and_mcp_share_request_plan_policy_payload_and_receipt_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("runtime");
    let workspace = workspace(temp.path());

    std::env::set_var("AGS_RUNTIME_HOME", &runtime);
    std::env::set_var("AGS_WORKSPACE_IDLE_MS", "1500");

    let companion = CompanionExecutable::install();
    let mut daemon = start_daemon(&companion.path, &workspace, &runtime);
    wait_for_registry(&runtime, &mut daemon);

    let transaction_argv = |format: Option<&str>| {
        let mut argv = vec![
            "ags".to_string(),
            "agent".to_string(),
            "register".to_string(),
            "--host".to_string(),
            "Hermes".to_string(),
            "--surface".to_string(),
            "mcp".to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
        ];
        if let Some(format) = format {
            argv.extend(["--format".to_string(), format.to_string()]);
        }
        argv
    };

    let human_transaction = Cli::try_parse_from(transaction_argv(None))
        .unwrap()
        .into_invocation();
    let machine_transaction = Cli::try_parse_from(transaction_argv(Some("json")))
        .unwrap()
        .into_invocation();
    let mcp_transaction: DecideArguments = serde_json::from_value(json!({
        "operation": parsed_operation(&machine_transaction),
    }))
    .unwrap();
    assert_eq!(
        parsed_operation(&human_transaction),
        parsed_operation(&machine_transaction)
    );
    assert_eq!(
        parsed_operation(&machine_transaction),
        &mcp_transaction.operation
    );
    let normalized_request_hash = request_hash(parsed_operation(&human_transaction));
    assert_eq!(
        normalized_request_hash,
        request_hash(&mcp_transaction.operation)
    );

    let human_transaction_output = execute_cli(transaction_argv(None), &workspace, &runtime);
    assert!(human_transaction_output.starts_with("state: planned\n"));
    let machine_transaction_output =
        execute_cli(transaction_argv(Some("json")), &workspace, &runtime);
    let machine_transaction_decision: Decision =
        serde_json::from_str(&machine_transaction_output).unwrap();
    assert_eq!(machine_transaction_decision.state, OperationState::Planned);

    let mut mcp = Connection::new(workspace.clone(), WorkspaceRpcPort);
    mcp.initialize("Hermes").unwrap();
    let mcp_transaction_decision: Decision =
        serde_json::from_value(mcp.decide(mcp_transaction).unwrap()).unwrap();
    assert_eq!(mcp_transaction_decision.state, OperationState::Planned);

    let machine_plan = machine_transaction_decision.plan.as_ref().unwrap();
    let mcp_plan = mcp_transaction_decision.plan.as_ref().unwrap();
    assert_eq!(machine_plan.operation, mcp_plan.operation);
    assert_eq!(machine_plan.policy_hash, mcp_plan.policy_hash);
    assert_eq!(machine_plan.payload_hash, mcp_plan.payload_hash);
    assert_eq!(machine_plan.action_digest, mcp_plan.action_digest);
    assert_eq!(machine_plan.steps, mcp_plan.steps);
    assert_eq!(
        machine_plan.expected_write_paths,
        mcp_plan.expected_write_paths
    );
    assert_eq!(machine_plan.verification, mcp_plan.verification);
    assert_eq!(machine_plan.recoverability, mcp_plan.recoverability);
    assert_eq!(machine_plan.execution, mcp_plan.execution);
    assert_eq!(machine_plan.plan_hash, mcp_plan.plan_hash);
    assert_ne!(machine_plan.binding_hash, mcp_plan.binding_hash);
    assert_ne!(
        machine_transaction_decision.action_ref,
        mcp_transaction_decision.action_ref
    );

    let schema_argv = |format: Option<&str>| {
        let mut argv = vec![
            "ags".to_string(),
            "schema".to_string(),
            "agent.register".to_string(),
            "--workspace".to_string(),
            workspace.display().to_string(),
        ];
        if let Some(format) = format {
            argv.extend(["--format".to_string(), format.to_string()]);
        }
        argv
    };
    let human_schema = Cli::try_parse_from(schema_argv(None))
        .unwrap()
        .into_invocation();
    let machine_schema = Cli::try_parse_from(schema_argv(Some("json")))
        .unwrap()
        .into_invocation();
    let mcp_schema: DecideArguments = serde_json::from_value(json!({
        "operation": parsed_operation(&machine_schema),
    }))
    .unwrap();
    assert_eq!(
        parsed_operation(&human_schema),
        parsed_operation(&machine_schema)
    );
    assert_eq!(parsed_operation(&machine_schema), &mcp_schema.operation);
    assert_eq!(
        request_hash(parsed_operation(&human_schema)),
        request_hash(&mcp_schema.operation)
    );

    let human_schema_output = execute_cli(schema_argv(None), &workspace, &runtime);
    assert!(human_schema_output.starts_with("state: no-change"));
    let machine_schema_output = execute_cli(schema_argv(Some("json")), &workspace, &runtime);
    let machine_schema_decision: Decision = serde_json::from_str(&machine_schema_output).unwrap();
    let mcp_schema_decision: Decision =
        serde_json::from_value(mcp.decide(mcp_schema).unwrap()).unwrap();
    let machine_receipt = machine_schema_decision.receipt.as_ref().unwrap();
    let mcp_receipt = mcp_schema_decision.receipt.as_ref().unwrap();
    assert_eq!(machine_receipt.receipt_id, mcp_receipt.receipt_id);
    assert_eq!(machine_receipt.plan_hash, mcp_receipt.plan_hash);
    assert_eq!(machine_receipt.payload_hash, mcp_receipt.payload_hash);
    assert_eq!(receipt_hash(machine_receipt), receipt_hash(mcp_receipt));
    assert_ne!(machine_receipt.binding_hash, mcp_receipt.binding_hash);

    drop(mcp);
    assert!(daemon.wait().unwrap().success());
}
