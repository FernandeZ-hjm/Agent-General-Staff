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
    run_ags_at(args, &repo_root())
}

fn run_ags_at(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .current_dir(cwd)
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

#[test]
fn skill_adoption_cli_requires_a_reviewed_plan_and_persists_private_state() {
    let fixture = TestDir::new("private-skill-adoption");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    let source_root = fixture.path().join("source");
    let source = source_root.join("skill");
    let metadata = fixture.path().join("routing.yaml");
    std::fs::create_dir_all(source_root.join(".git")).unwrap();
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source_root.join("LICENSE"), "MIT fixture license\n").unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: cli-adopted-team\ndescription: Upstream description.\n---\n# CLI adopted team\n",
    )
    .unwrap();
    std::fs::write(
        &metadata,
        "summary: Delegate bounded software work when parallel exploration is useful.\nintent_tags: [delegation, parallel-software-work]\npositive_examples: [Delegate this investigation in parallel]\nnegative_examples: [Answer this directly]\n",
    )
    .unwrap();

    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ags"))
            .args(args)
            .current_dir(repo_root())
            .env("HOME", &home)
            .env("AGS_RUNTIME_HOME", &runtime)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .expect("run isolated skill adoption command")
    };
    let source_arg = source.to_str().unwrap();
    let metadata_arg = metadata.to_str().unwrap();
    let plan = parse_json(
        &run(&[
            "skill",
            "adopt",
            source_arg,
            "--metadata",
            metadata_arg,
            "--host",
            "codex",
            "--format",
            "json",
        ]),
        "skill adopt plan",
    );
    assert_eq!(plan["operation"], "adopt");
    assert!(
        !runtime.exists(),
        "plan-only CLI must not write runtime state"
    );

    let refused = run(&[
        "skill",
        "adopt",
        source_arg,
        "--metadata",
        metadata_arg,
        "--host",
        "codex",
        "--yes",
        "--format",
        "json",
    ]);
    assert!(!refused.status.success());
    assert!(
        !runtime.exists(),
        "unbound apply must not write runtime state"
    );

    let plan_hash = plan["plan_hash"].as_str().unwrap();
    let receipt = parse_json(
        &run(&[
            "skill",
            "adopt",
            source_arg,
            "--metadata",
            metadata_arg,
            "--host",
            "codex",
            "--yes",
            "--plan-hash",
            plan_hash,
            "--format",
            "json",
        ]),
        "skill adopt apply",
    );
    assert_eq!(receipt["skill_id"], "cli-adopted-team");
    assert_eq!(receipt["requires_repreflight"], true);

    let status = parse_json(
        &run(&["skill", "status", "cli-adopted-team", "--format", "json"]),
        "skill adoption status",
    );
    assert_eq!(status["registered"], true);
    assert_eq!(status["body_hash_matches"], true);
    assert_eq!(status["visible_hosts"], serde_json::json!(["codex"]));
    assert_eq!(status["active_hosts"], serde_json::json!(["codex"]));
}

#[test]
fn retired_sync_and_full_scope_are_absent_from_the_cli_surface() {
    let root_help = run_ags(&["--help"]);
    assert_success(&root_help, "root help");
    let root_help = String::from_utf8_lossy(&root_help.stdout);
    assert!(
        !root_help
            .lines()
            .any(|line| line.trim_start().starts_with("sync ")),
        "retired sync command leaked into root help:\n{root_help}"
    );

    let verify_help = run_ags(&["verify", "--help"]);
    assert_success(&verify_help, "verify help");
    let verify_help = String::from_utf8_lossy(&verify_help.stdout);
    assert!(
        !verify_help.contains("local, full, release"),
        "retired full scope leaked into verify help:\n{verify_help}"
    );

    for args in [&["sync", "check"][..], &["verify", "--scope", "full"][..]] {
        let output = run_ags(args);
        assert!(
            !output.status.success(),
            "retired CLI surface unexpectedly succeeded: {args:?}"
        );
    }
}

#[test]
fn local_verify_routes_integrated_projects_away_from_suite_checks() {
    let root = repo_root();
    let fixture = TestDir::new("verify-integrated-project");
    let project = fixture.path().join("business-project");
    let home = fixture.path().join("home");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join("config")).unwrap();
    copy_tree(&root.join("protocol"), &project.join("protocol"));
    for entry in [
        "AGENTS.md",
        "CLAUDE.md",
        "WORKSPACE.md",
        "AGENT_SUITE_PROTOCOL.md",
    ] {
        std::fs::copy(root.join(entry), project.join(entry)).unwrap();
    }
    std::fs::write(
        project.join("config/agent-project-profile.yaml"),
        "schema_version: 1\nproject:\n  name: Business Project\n  slug: business-project\n",
    )
    .unwrap();
    let memory = home.join(".agents/memory/projects/business-project");
    std::fs::create_dir_all(&memory).unwrap();
    std::fs::write(memory.join("context-capsule.md"), "# Context Capsule\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args([
            "verify",
            "--scope",
            "local",
            "--target",
            project.to_str().unwrap(),
            "--format",
            "json",
        ])
        .current_dir(fixture.path())
        .env("HOME", &home)
        .env("AGS_HOME", home.join(".ags/private-runtime"))
        .output()
        .expect("verify integrated business project");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "verify did not emit JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let ids = report["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect::<Vec<_>>();

    assert!(
        output.status.success(),
        "integrated project verification failed: {}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    assert!(ids.contains(&"session-preflight"));
    for suite_only in [
        "cargo-fmt",
        "cargo-test",
        "cargo-build-release",
        "fixture-valid-full",
        "fixture-invalid-compact-rejected",
        "runtime-profile-templates",
    ] {
        assert!(
            !ids.contains(&suite_only),
            "integrated project ran suite-only check {suite_only}: {ids:?}"
        );
    }
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
    assert_eq!(policy["effective_execution_mode"], "single-writer");

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
    std::fs::create_dir_all(temp.path().join(".git")).unwrap();
    let contract_path = temp.path().join("handoff.json");
    let task_card_path = temp.path().join("task-card.md");
    let launch_plan_path = temp.path().join("launch-plan.json");
    let delivery_report_path = temp.path().join("delivery-report.md");
    let receipt_path = temp.path().join("receipt.json");
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
    let launch_plan_output = run_ags(&[
        "run",
        task_card_path.to_str().unwrap(),
        "--current-task-approval",
        "--format",
        "json",
    ]);
    assert_success(&launch_plan_output, "launch plan");
    std::fs::write(&launch_plan_path, &launch_plan_output.stdout).unwrap();
    let launch_plan: Value = serde_json::from_slice(&launch_plan_output.stdout).unwrap();
    let launch_plan_hash = launch_plan["launch_plan_hash"].as_str().unwrap();
    let report = format!(
        "# 任务交付报告\n\
\n\
Closure schema: 1.1\n\
Contract ID: {contract_id}\n\
task-card-hash: {task_card_hash}\n\
launch-plan-hash: {launch_plan_hash}\n\
execution-mode-used: single-writer\n\
execution-topology-used: single\n\
delegation-used: none\n\
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
- none\n"
    );
    std::fs::write(&delivery_report_path, report).unwrap();

    let closed = parse_json(
        &run_ags_at(
            &[
                "task",
                "close",
                task_card_path.to_str().unwrap(),
                launch_plan_path.to_str().unwrap(),
                delivery_report_path.to_str().unwrap(),
                "--receipt-out",
                receipt_path.to_str().unwrap(),
                "--format",
                "json",
            ],
            temp.path(),
        ),
        "task close",
    );
    assert_eq!(closed["valid"], true);
    assert_eq!(closed["contract_id"], contract_id);
    assert_eq!(closed["task_card_hash"], task_card_hash);
    assert_eq!(closed["launch_plan_hash"], launch_plan_hash);
    assert!(receipt_path.is_file());
}

#[test]
fn integrity_and_bootstrap_cli_contract() {
    let root = repo_root();
    let temp = TestDir::new("receipt-verify");
    let task = temp.path().join("task.md");
    let plan = temp.path().join("launch-plan.json");
    let report = temp.path().join("report.md");
    let receipt = temp.path().join("receipt.json");
    std::fs::write(&task, "task").unwrap();
    let mut plan_value = serde_json::json!({
        "schema_version": "0.3.6-launch-plan",
        "task_card_hash": ags_evidence::sha256_hex(b"task"),
        "launch_plan_hash": "",
        "effective_execution_mode": "single-writer",
        "effective_execution_topology": "single",
        "delegation_planning": false
    });
    let plan_hash = ags_task_contract::runner::canonical_launch_plan_hash(&plan_value).unwrap();
    plan_value["launch_plan_hash"] = Value::String(plan_hash.clone());
    std::fs::write(&plan, serde_json::to_vec_pretty(&plan_value).unwrap()).unwrap();
    std::fs::write(&report, "report").unwrap();
    let task_hash = ags_evidence::sha256_hex(b"task");
    let receipt_value = serde_json::json!({
        "schema_version": "0.3.6-task-receipt",
        "receipt_id": ags_evidence::receipt_id(&task_hash, &plan_hash),
        "timestamp": "unix-0",
        "task_card_hash": task_hash,
        "launch_plan_hash": plan_hash,
        "task_card_path": task,
        "launch_plan_path": plan,
        "delivery_report_path": report,
        "gate_result": {"decision": "allow"},
        "verification_results": [],
        "delivery_report_hash": ags_evidence::sha256_hex(b"report"),
        "execution_footprint": {
            "execution_mode_used": "single-writer",
            "execution_topology_used": "single",
            "delegation_used": "none"
        },
        "closure_status": "completed",
        "exit_code": 0
    });
    std::fs::write(&receipt, serde_json::to_vec_pretty(&receipt_value).unwrap()).unwrap();
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

    let manifests = root.join("manifests");
    assert!(
        manifests.is_dir(),
        "the capability authority fixture requires manifests"
    );
    copy_tree(&manifests, &authority.join("manifests"));
    for relative in ["global-skills", "skill-packs"] {
        let source = root.join(relative);
        if source.is_dir() {
            copy_tree(&source, &authority.join(relative));
        }
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
    assert_eq!(
        report["lifecycle_workspace"],
        root.canonicalize().unwrap().display().to_string(),
        "update verify must observe the working workspace, not the installed source authority"
    );
}

#[test]
fn agents_scan_cli_contract() {
    let output = run_ags_isolated(&["agents", "scan", "--format", "json"]);
    assert_success(&output, "agents scan");
    serde_json::from_slice::<Value>(&output.stdout).expect("agents scan JSON");
}

#[cfg(unix)]
#[test]
fn capability_snapshot_target_controls_workspace_scoped_host_probe() {
    use std::os::unix::fs::PermissionsExt;

    let root = repo_root();
    let fixture = TestDir::new("capability-snapshot-target");
    let workspace = fixture.path().join("workspace");
    let caller = fixture.path().join("caller");
    let runtime = fixture.path().join("runtime");
    let host_home = fixture.path().join("host-home");
    let bin = fixture.path().join("bin");
    for path in [&workspace, &caller, &runtime, &host_home, &bin] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::create_dir_all(workspace.join(".git")).unwrap();

    let codex = bin.join("codex");
    std::fs::write(
        &codex,
        format!(
            "#!/bin/sh\nprintf 'Name Command Args Status\\n'\nif [ \"$PWD\" = '{}' ]; then\n  printf 'ags ags mcp serve enabled\\n'\nfi\n",
            workspace.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&codex).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&codex, permissions).unwrap();

    let run_snapshot = |cwd: &Path| {
        Command::new(env!("CARGO_BIN_EXE_ags"))
            .args([
                "capability",
                "snapshot",
                "--host",
                "codex",
                "--target",
                workspace.to_str().unwrap(),
                "--format",
                "json",
            ])
            .current_dir(cwd)
            .env("HOME", &host_home)
            .env("AGS_HOME", &runtime)
            .env_remove("AGS_RUNTIME_HOME")
            .env("AGS_SOURCE_ROOT", &root)
            .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output()
            .expect("build workspace-scoped capability snapshot")
    };

    let from_caller = parse_json(
        &run_snapshot(&caller),
        "capability snapshot from unrelated caller directory",
    );
    let from_workspace = parse_json(
        &run_snapshot(&workspace),
        "capability snapshot from target workspace",
    );
    assert_eq!(
        from_caller["snapshot_hash"], from_workspace["snapshot_hash"],
        "--target must make the host probe independent of the caller's current directory"
    );
}

#[test]
fn agents_govern_previews_workspace_owned_codebuddy_migration() {
    let workspace = TestDir::new("agents-govern-target");
    std::fs::create_dir_all(workspace.path().join(".git")).unwrap();
    let workspace_arg = workspace.path().to_str().unwrap();
    let output = run_ags_isolated(&[
        "agents",
        "govern",
        "--agent",
        "codebuddy-code",
        "--target",
        workspace_arg,
        "--format",
        "json",
    ]);
    let report = parse_json(&output, "agents govern CodeBuddy migration preview");
    let preview = &report["hosts"][0]["lifecycle_migration_preview"];
    assert_eq!(preview["host"], "codebuddy-code");
    assert_eq!(preview["removal_ready_after_apply"], true);
    assert_eq!(preview["managed_workspaces"][0]["adapter_ready_now"], false);
    assert_eq!(
        preview["managed_workspaces"][0]["adapter_ready_after_apply"],
        true
    );
    assert!(Path::new(preview["workspace_adapter"].as_str().unwrap())
        .ends_with(Path::new(".codebuddy").join("settings.local.json")));
    assert_eq!(
        Path::new(preview["current_workspace"].as_str().unwrap()),
        workspace.path().canonicalize().unwrap()
    );
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
    let invalid_receipt = root.join("tests/fixtures/receipt-invalid-hash.json");

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
}

#[test]
fn doctor_rejects_macbook_legacy_lifecycle_before_host_start() {
    let fixture = TestDir::new("doctor-legacy-lifecycle");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    let project = fixture.path().join("managed-workspace");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join(".claude")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(
        runtime.join("install-manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "lifecycle": {
                "approved_hosts": ["claude-code"],
                "selection_source": "setup"
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let retired_interpreter = concat!("python", "3");
    let retired_command =
        format!("{retired_interpreter} \"$HOME/.agents/scripts/context-memory-start.py\"");
    std::fs::write(
        project.join(".claude/settings.local.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": retired_command
                    }]
                }],
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "ags host lifecycle --event session-end --host claude-code --target ."
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(["doctor", "--target"])
        .arg(&project)
        .args(["--format", "json"])
        .current_dir(repo_root())
        .env("HOME", &home)
        .env("AGS_HOME", &runtime)
        .env_remove("AGS_RUNTIME_HOME")
        .env("AGS_SOURCE_ROOT", repo_root())
        .env("AGS_REMOTE_LATEST_OFFLINE", "1")
        .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run doctor against legacy lifecycle fixture");
    assert!(
        !output.status.success(),
        "Doctor must fail before Claude Code starts when legacy lifecycle wiring is effective"
    );
    assert!(
        !runtime.join("workspace-services").exists(),
        "read-only Doctor must not start or register a workspace daemon"
    );

    let report: Value =
        serde_json::from_slice(&output.stdout).expect("Doctor failure still emits JSON");
    let finding = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["check_name"] == "lifecycle-legacy-commands-absent")
        .expect("legacy lifecycle conformance finding");
    assert_eq!(finding["status"], "fail");
    assert!(finding["observed"]
        .as_str()
        .unwrap()
        .contains("context-memory-start.py"));
    assert!(finding["observed"].as_str().unwrap().contains("--target ."));
    assert!(finding["remediation"]
        .as_str()
        .unwrap()
        .contains("Migrate managed workspaces"));
    let approval = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["check_name"] == "lifecycle-host-approval-current")
        .expect("approved host conformance finding");
    assert_eq!(approval["status"], "fail");
    assert!(approval["observed"]
        .as_str()
        .unwrap()
        .contains("claude-code"));
}
