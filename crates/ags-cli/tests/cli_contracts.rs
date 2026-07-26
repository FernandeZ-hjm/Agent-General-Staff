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
            "ags-cli-contract-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create CLI contract test directory");
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
        .expect("run compiled ags binary")
}

fn run_ags_isolated(args: &[&str]) -> Output {
    let home = TestDir::new("isolated-home");
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .current_dir(repo_root())
        .env("HOME", home.path())
        .env("AGS_HOME", home.path().join(".ags/private-runtime"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run compiled ags binary in isolated host environment")
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in std::fs::read_dir(source).expect("read copied fixture directory") {
        let entry = entry.expect("read copied fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::metadata(&source_path).expect("read copied fixture metadata");
        if metadata.is_dir() {
            copy_tree(&source_path, &destination_path);
        } else if metadata.is_file() {
            std::fs::copy(&source_path, &destination_path).expect("copy fixture file");
        }
    }
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

    let validate = run_ags(&["task", "validate", card]);
    assert_success(&validate, "task validate");

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
fn host_plan_card_closes_against_its_exact_delivery_report() {
    let temp = TestDir::new("host-plan-closure");
    let contract_path = temp.path().join("handoff.json");
    let task_card_path = temp.path().join("task-card.md");
    let delivery_report_path = temp.path().join("delivery-report.md");
    let contract = serde_json::json!({
        "schema_version": ags_task_contract::HANDOFF_CONTRACT_SCHEMA_VERSION,
        "task_level": "Medium",
        "task": "执行已封闭的宿主 Plan 模式方案",
        "fields": {
            "目标：": "- G-01: 完成 Plan 模式任务卡闭环",
            "验收标准：": "- AC-01 -> G-01: 任务卡通过校验并与交付报告精确回绑",
            "Verification gate:": "- commands:\n  - V-01 -> AC-01: ags task validate task-card.md\n- expected evidence:\n  - EV-01 -> AC-01: validator and closure checks pass\n- stop condition:\n  - 任一校验失败即停止"
        }
    });
    std::fs::write(
        &contract_path,
        serde_json::to_string_pretty(&contract).unwrap(),
    )
    .unwrap();

    let contract_arg = contract_path.to_str().unwrap();
    let compiled = run_ags(&[
        "task",
        "compile",
        contract_arg,
        "--output",
        "card",
        "--host-plan-mode-final",
        "--confirmed-handoff-contract",
    ]);
    assert_success(&compiled, "host Plan-mode task compile");
    let card = String::from_utf8(compiled.stdout).unwrap();
    assert!(card.starts_with("## 任务卡"));
    assert!(card.contains("Handoff source: host-plan-mode"));
    let contract_id = card
        .lines()
        .find_map(|line| line.strip_prefix("Contract ID: "))
        .expect("compiled task card Contract ID");
    std::fs::write(&task_card_path, &card).unwrap();

    let task_card_hash = ags_evidence::sha256_hex(card.as_bytes());
    let report = format!(
        "# 任务交付报告\n\
\n\
Closure schema: 1.0\n\
Contract ID: {contract_id}\n\
task-card-hash: {task_card_hash}\n\
receipt-id: receipt-{}\n\
状态: completed\n\
review-gate: n/a\n\
\n\
## 目标闭环\n\
- G-01: done — Plan 模式任务卡已生成并验证\n\
\n\
## 验收闭环\n\
- AC-01: pass — evidence: exact task-card hash matched\n\
\n\
## 验证闭环\n\
- V-01: pass — ags task validate; exit 0\n\
\n\
## 未闭环项\n\
- none\n",
        &task_card_hash[..12]
    );
    std::fs::write(&delivery_report_path, report).unwrap();

    let closed = parse_json(
        &run_ags(&[
            "task",
            "close",
            task_card_path.to_str().unwrap(),
            delivery_report_path.to_str().unwrap(),
            "--format",
            "json",
        ]),
        "task close",
    );
    assert_eq!(closed["valid"], true);
    assert_eq!(closed["contract_id"], contract_id);
    assert_eq!(closed["task_card_hash"], task_card_hash);
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
    let bootstrap = run_ags(&[
        "bootstrap",
        "--dry-run",
        "--target",
        root,
        "--format",
        "json",
    ]);
    assert_success(&bootstrap, "bootstrap --dry-run");
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
fn update_verify_validates_snapshot_against_installed_capability_authority() {
    let root = repo_root();
    let fixture = TestDir::new("update-verify-authority");
    let authority = fixture.path().join("stable-authority");
    let runtime = fixture.path().join("runtime");
    let host_home = fixture.path().join("host-home");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&host_home).unwrap();

    for relative in ["manifests", "global-skills", "skill-packs"] {
        copy_tree(&root.join(relative), &authority.join(relative));
    }
    let registry_path = authority.join("manifests/skills-registry.yaml");
    let mut registry = std::fs::read_to_string(&registry_path).unwrap();
    registry.push_str("\n# authority fixture hash differs from current checkout\n");
    std::fs::write(&registry_path, registry).unwrap();
    std::fs::write(
        runtime.join("install-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source_root": authority.display().to_string(),
        }))
        .unwrap(),
    )
    .unwrap();

    let snapshot = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args([
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            authority.to_str().unwrap(),
            "--write",
            "--format",
            "json",
        ])
        .current_dir(&root)
        .env("HOME", &host_home)
        .env("AGS_HOME", &runtime)
        .env_remove("AGS_RUNTIME_HOME")
        .env("AGS_SOURCE_ROOT", &authority)
        .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("write authority-bound capability snapshot");
    assert_success(&snapshot, "capability snapshot --write");
    assert_eq!(
        ags_capability_governance::resolve_capability_authority_root(&root, &runtime, None)
            .unwrap(),
        authority.canonicalize().unwrap()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args([
            "update",
            "verify",
            "--target",
            runtime.to_str().unwrap(),
            "--format",
            "json",
        ])
        .current_dir(&root)
        .env("HOME", &host_home)
        .env("AGS_HOME", &runtime)
        .env_remove("AGS_RUNTIME_HOME")
        .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run update verify against installed authority");
    let report = parse_json(&output, "update verify installed authority");
    assert_eq!(
        report["skill_resolver"]["snapshot_current"], true,
        "update verify must validate the snapshot against the installed capability authority, \
         not the current A checkout"
    );
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

    // Exercise the compatibility `verify run` parser without recursively
    // launching workspace verification from inside `cargo test`.
    let verify_alias = run_ags(&["verify", "run", "--scope", "invalid"]);
    assert_eq!(verify_alias.status.code(), Some(2));
}
