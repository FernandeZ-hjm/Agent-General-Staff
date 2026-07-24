use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ags-public-cli-contract-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create public CLI contract test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn run_ags(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("run compiled public ags binary")
}

fn run_ags_isolated(args: &[&str]) -> Output {
    let home = TestDir::new("isolated-home");
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .current_dir(repo_root())
        .env("HOME", home.path())
        .env("AGS_HOME", home.path().join(".ags/runtime"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run public ags binary in isolated host environment")
}

fn assert_success(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed (status {:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(output: &Output, label: &str) -> Value {
    assert_success(output, label);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label} did not emit JSON: {error}\nstdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn task_card_pipeline_cli_contract() {
    let root = repo_root();
    let card = root.join("tests/fixtures/valid-full.md");
    let card = card.to_str().expect("UTF-8 fixture path");

    assert_success(&run_ags(&["task", "validate", card]), "task validate");

    let policy = parse_json(
        &run_ags(&["policy", "resolve", card, "--format", "json"]),
        "policy resolve",
    );
    assert_eq!(policy["effective_permission_mode"], "execute-and-verify");

    let gate = parse_json(
        &run_ags(&["gate", "check", card, "--format", "json"]),
        "gate check",
    );
    assert_eq!(gate["decision"], "allow");
    assert!(gate.get("resolved_policy").is_some());

    let runner = parse_json(
        &run_ags(&["run", card, "--check-only", "--format", "json"]),
        "run --check-only",
    );
    assert_eq!(runner["gate_decision"], "allow");
    assert_eq!(runner["validation_passed"], true);
}

#[test]
fn integrity_and_bootstrap_cli_contract() {
    let root = repo_root();
    let receipt = root.join("tests/fixtures/receipt-valid.json");
    let receipt = receipt.to_str().expect("UTF-8 receipt path");

    let verified = parse_json(
        &run_ags(&["receipt", "verify", receipt, "--format", "json"]),
        "receipt verify",
    );
    assert_eq!(verified["valid"], true);

    let root = root.to_str().expect("UTF-8 workspace path");
    assert_success(
        &run_ags(&[
            "bootstrap",
            "--dry-run",
            "--target",
            root,
            "--format",
            "json",
        ]),
        "bootstrap --dry-run",
    );
}

#[test]
fn session_preflight_host_matrix_cli_contract() {
    let root = repo_root();
    let root = root.to_str().expect("UTF-8 workspace path");
    let cases = [
        ("codex", "codex"),
        ("claude-code", "claude-code"),
        ("omp", "omp"),
        ("cursor", "cursor"),
        ("CodeBuddy-Code", "codebuddy-code"),
    ];

    for (input, canonical) in cases {
        let report = parse_json(
            &run_ags(&[
                "session",
                "preflight",
                "--for",
                input,
                "--target",
                root,
                "--format",
                "json",
            ]),
            &format!("session preflight {input}"),
        );
        assert_eq!(report["for_agent"], canonical);
        assert_eq!(report["exit_code"], 0);
        assert_ne!(report["overall_status"], "stop");
    }
}

#[test]
fn setup_init_and_update_read_only_cli_contract() {
    let setup_home = TestDir::new("setup");
    let init_target = TestDir::new("init");
    let setup_home = setup_home.path().to_str().expect("UTF-8 setup path");
    let init_target = init_target.path().to_str().expect("UTF-8 init path");

    let commands: &[(&str, &[&str])] = &[
        (
            "setup --dry-run",
            &[
                "setup",
                "--target",
                setup_home,
                "--dry-run",
                "--format",
                "json",
            ],
        ),
        (
            "init --dry-run",
            &[
                "init",
                "--target",
                init_target,
                "--dry-run",
                "--format",
                "json",
            ],
        ),
        ("update check", &["update", "check", "--format", "json"]),
    ];

    for (label, args) in commands {
        let output = run_ags(args);
        assert_success(&output, label);
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{label} did not emit JSON: {error}\nstdout:\n{}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    }
}

#[test]
fn agents_scan_cli_contract() {
    let output = run_ags_isolated(&["agents", "scan", "--format", "json"]);
    assert_success(&output, "agents scan");
    serde_json::from_slice::<Value>(&output.stdout).expect("agents scan JSON");
}

#[test]
fn capability_inventory_cli_contract() {
    let output = run_ags_isolated(&["capability", "inventory", "--format", "json"]);
    assert_success(&output, "capability inventory");
    serde_json::from_slice::<Value>(&output.stdout).expect("capability inventory JSON");
}

#[test]
fn skill_inventory_cli_contract() {
    let output = run_ags_isolated(&["skill", "--format", "json"]);
    assert_success(&output, "skill inventory");
    serde_json::from_slice::<Value>(&output.stdout).expect("skill inventory JSON");
}

#[test]
fn high_risk_cli_rejections_remain_fail_closed() {
    let root = repo_root();
    let invalid_card = root.join("tests/fixtures/invalid-ultracode-authority-abuse.md");
    let invalid_receipt = root.join("tests/fixtures/receipt-invalid-hash.json");

    let policy = run_ags(&[
        "policy",
        "resolve",
        invalid_card.to_str().expect("UTF-8 invalid card path"),
        "--format",
        "json",
    ]);
    assert!(!policy.status.success());
    assert!(
        String::from_utf8_lossy(&policy.stdout).contains("ULTRACODE_AUTHORITY_ABUSE")
            || String::from_utf8_lossy(&policy.stderr).contains("ULTRACODE_AUTHORITY_ABUSE")
    );

    let receipt = run_ags(&[
        "receipt",
        "verify",
        invalid_receipt
            .to_str()
            .expect("UTF-8 invalid receipt path"),
        "--format",
        "json",
    ]);
    assert!(!receipt.status.success());

    // Exercise the compatibility parser without recursively launching
    // workspace verification from inside `cargo test`.
    let verify_alias = run_ags(&["verify", "run", "--scope", "invalid"]);
    assert_eq!(verify_alias.status.code(), Some(2));
}
