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
fn metadata_inline_yaml_is_a_structured_file_usage_error() {
    let fixture = TestDir::new("metadata-file-usage");
    let source = fixture.path().join("skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("SKILL.md"),
        "---\nname: metadata-fixture\ndescription: Fixture.\n---\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(["skill", "adopt"])
        .arg(&source)
        .args([
            "--metadata",
            "summary: inline is not a file",
            "--host",
            "codex",
            "--format",
            "json",
        ])
        .current_dir(repo_root())
        .env("HOME", fixture.path().join("home"))
        .env("AGS_HOME", fixture.path().join("runtime"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        diagnostic["error"]["code"],
        "metadata_argument_requires_file"
    );
    assert!(diagnostic["error"]["message"]
        .as_str()
        .unwrap()
        .contains("existing YAML file path (<FILE>)"));
}

#[test]
fn manifest_unknown_field_json_keeps_path_field_allowlist_and_location() {
    let fixture = TestDir::new("manifest-diagnostic");
    let authority = fixture.path().join("authority");
    let manifests = authority.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    for name in [
        "onboarding-public.yaml",
        "skills-registry.yaml",
        "mcp-registry.yaml",
    ] {
        std::fs::copy(
            repo_root().join("manifests").join(name),
            manifests.join(name),
        )
        .unwrap();
    }
    std::fs::write(
        manifests.join("third-party-capabilities.yaml"),
        "schema_version: \"1.0\"\ncapabilities:\n  - id: invalid\n    kind: cli\n    profiles: [public]\n    notes: unsupported\n",
    )
    .unwrap();
    let target = fixture.path().join("project");
    std::fs::create_dir_all(&target).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(["onboarding", "plan", "--target"])
        .arg(&target)
        .args(["--host", "codex", "--format", "json"])
        .current_dir(&authority)
        .env("HOME", fixture.path().join("home"))
        .env("AGS_HOME", fixture.path().join("runtime"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let diagnostic: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(diagnostic["error"]["code"], "manifest_unknown_field");
    let message = diagnostic["error"]["message"].as_str().unwrap();
    assert!(message.contains("third-party-capabilities.yaml"));
    assert!(message.contains("yaml_path=capabilities[0].notes"));
    assert!(message.contains("field=notes"));
    assert!(message.contains("allowed_fields=["));
    assert!(message.contains("line=6 column=5"));
}

#[test]
fn setup_json_reports_adapter_decision_without_implicit_skill_state() {
    let fixture = TestDir::new("setup-adapter-decision");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(["setup", "--format", "json"])
        .current_dir(repo_root())
        .env("HOME", &home)
        .env("AGS_HOME", &runtime)
        .env("AGS_SOURCE_ROOT", repo_root())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert_success(&output, "setup JSON adapter decision");
    let plan: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["superpowers_adapter"]["decision_required"], true);
    assert_eq!(
        plan["superpowers_adapter"]["distribution_id"],
        "ags-superpowers-adapter"
    );
    assert_eq!(
        plan["superpowers_adapter"]["compatibility_parent"],
        "superpowers"
    );
    assert_eq!(plan["superpowers_adapter"]["implicit_install"], false);
    assert!(!ags_platform::RuntimeLayout::new(&runtime)
        .installed_skills()
        .exists());
}

#[test]
fn init_local_succeeds_without_creating_git_repository() {
    let fixture = TestDir::new("init-non-git");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    let project = fixture.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ags"))
            .args(args)
            .current_dir(repo_root())
            .env("HOME", &home)
            .env("AGS_HOME", &runtime)
            .env("AGS_SOURCE_ROOT", repo_root())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    };
    let setup = run(&[
        "setup",
        "--yes",
        "--force",
        "--lifecycle-hosts",
        "codex",
        "--format",
        "json",
    ]);
    assert_success(&setup, "isolated setup before non-Git init");
    let init = run(&[
        "init",
        "--target",
        project.to_str().unwrap(),
        "--mode",
        "local",
        "--format",
        "json",
    ]);
    assert_success(&init, "local init in non-Git workspace");
    let output: Value = serde_json::from_slice(&init.stdout).unwrap();
    assert_eq!(output["overlay"]["applicability"], "not-applicable");
    assert!(!project.join(".git").exists());
}

#[test]
fn bundled_adapter_plan_and_receipt_preserve_distribution_identity() {
    let fixture = TestDir::new("adapter-distribution-identity");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ags"));
        command
            .current_dir(repo_root())
            .env("HOME", &home)
            .env("AGS_HOME", &runtime)
            .env("AGS_RUNTIME_HOME", &runtime)
            .env("AGS_SOURCE_ROOT", repo_root())
            .env("PATH", "/usr/bin:/bin");
        command
    };
    let setup = command()
        .args([
            "setup",
            "--yes",
            "--force",
            "--lifecycle-hosts",
            "codex",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_success(&setup, "isolated setup before adapter installation");

    let recommended = command()
        .args(["skill", "recommend", "--format", "json"])
        .output()
        .unwrap();
    assert_success(&recommended, "adapter catalog recommendation");
    let recommendations: Value = serde_json::from_slice(&recommended.stdout).unwrap();
    let adapter_recommendation = recommendations
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "ags-superpowers-adapter")
        .expect("adapter must be present in the public catalog snapshot");

    let planned = command()
        .args([
            "skill",
            "install",
            "ags-superpowers-adapter",
            "--host",
            "codex",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_success(&planned, "adapter install plan");
    let plan: Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(
        plan["metadata"]["catalog_distribution_id"],
        "ags-superpowers-adapter"
    );
    assert_eq!(
        plan["metadata"]["catalog_display_name"],
        "AGS Superpowers Adapter"
    );
    assert_eq!(plan["metadata"]["skill_id"], "superpowers");
    assert_eq!(
        plan["metadata"]["catalog_hash"],
        adapter_recommendation["catalog_hash"]
    );
    assert_eq!(
        plan["metadata"]["catalog_release"],
        adapter_recommendation["catalog_release"]
    );

    let mut apply_args = vec![
        "skill".to_string(),
        "install".to_string(),
        "ags-superpowers-adapter".to_string(),
        "--host".to_string(),
        "codex".to_string(),
        "--plan-hash".to_string(),
        plan["plan_hash"].as_str().unwrap().to_string(),
    ];
    for risk in plan["required_acknowledgements"].as_array().unwrap() {
        apply_args.push("--ack-risk".to_string());
        apply_args.push(risk.as_str().unwrap().to_string());
    }
    apply_args.extend([
        "--yes".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ]);
    let applied = command().args(&apply_args).output().unwrap();
    assert_success(&applied, "adapter install apply");
    let receipt: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(receipt["status"], "verified");
    let distribution = receipt["verification_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["id"] == "catalog-distribution-identity")
        .expect("receipt must retain catalog distribution identity");
    assert_eq!(distribution["passed"], true);
    let evidence: Value = serde_json::from_str(distribution["evidence"].as_str().unwrap()).unwrap();
    assert_eq!(evidence["distribution_id"], "ags-superpowers-adapter");
    assert_eq!(evidence["compatibility_parent"], "superpowers");
}

#[test]
fn absent_host_cli_is_non_blocking_for_doctor_but_strict_verify_fails_closed() {
    let fixture = TestDir::new("absent-host-cli-matrix");
    let home = fixture.path().join("home");
    let runtime = fixture.path().join("runtime");
    std::fs::create_dir_all(&home).unwrap();
    let command = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ags"));
        command
            .current_dir(repo_root())
            .env("HOME", &home)
            .env("AGS_HOME", &runtime)
            .env("AGS_RUNTIME_HOME", &runtime)
            .env("AGS_SOURCE_ROOT", repo_root())
            .env("PATH", "/usr/bin:/bin");
        command
    };
    let setup = command()
        .args([
            "setup",
            "--yes",
            "--force",
            "--lifecycle-hosts",
            "codex",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_success(&setup, "isolated setup before absent-host doctor");

    let doctor = command()
        .args(["doctor", "--target", ".", "--format", "json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let mcp = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["check_name"] == "mcp-registration-current")
        .expect("doctor must report the native MCP applicability decision");
    assert_eq!(mcp["status"], "skip");
    assert!(!report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| {
            finding["check_name"]
                .as_str()
                .is_some_and(|name| name.contains("codex-command-skill-metadata"))
        }));

    let strict = command()
        .args([
            "agents", "verify", "--host", "codex", "--strict", "--format", "json",
        ])
        .output()
        .unwrap();
    assert!(!strict.status.success());
    let strict_report: Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert_eq!(strict_report["strict_ready"], false);
    assert_eq!(
        strict_report["host_native_mcp"]["status"],
        "host-unavailable"
    );
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
    let installed_index = ags_platform::RuntimeLayout::new(&runtime).installed_skills();
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
    assert_eq!(plan["intent"]["operation"], "install");
    assert_eq!(plan["metadata"]["adoption_operation"], "install");
    assert_eq!(plan["metadata"]["skill_id"], "cli-adopted-team");
    assert!(
        runtime.join("maintenance/plans").is_dir(),
        "plan-only CLI must persist the immutable reviewed plan for cross-process apply"
    );
    assert!(
        !installed_index.exists(),
        "planning must not install or register the Skill"
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
        !installed_index.exists(),
        "unbound apply must not install or register the Skill"
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
            "--ack-risk",
            "catalog_unreviewed",
            "--format",
            "json",
        ]),
        "skill adopt apply",
    );
    assert_eq!(receipt["plan_hash"], plan["plan_hash"]);
    assert_eq!(receipt["status"], "verified");
    assert_eq!(plan["activation"][0]["requires_repreflight"], true);

    let status = parse_json(
        &run(&["skill", "status", "cli-adopted-team", "--format", "json"]),
        "skill adoption status",
    );
    assert_eq!(status["schema_version"], "0.4.13-skill-status-projection");
    assert_eq!(status["catalog"]["state"], "unlisted");
    assert_eq!(status["installation"]["state"], "installed");
    assert_eq!(status["activation"]["state"], "route-verified");
    assert_eq!(status["update"]["state"], "rebind-required");
    assert_eq!(
        status["activation"]["routes"]["installation"]["visible_hosts"],
        serde_json::json!(["codex"])
    );
    assert_eq!(
        status["activation"]["routes"]["installation"]["active_hosts"],
        serde_json::json!(["codex"])
    );
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
fn integrity_cli_contract() {
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
fn direct_rust_update_plan_fails_closed_without_verified_launcher() {
    let output = run_ags(&["update", "plan", "--format", "json"]);
    assert!(
        !output.status.success(),
        "the unsigned Rust entrypoint must not plan a core update"
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .expect("direct Rust update plan must emit one JSON error document");
    assert_eq!(report["status"], "launcher_required");
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
fn capability_snapshot_write_json_stdout_is_one_strict_json_document() {
    let fixture = TestDir::new("snapshot-write-json");
    let runtime = fixture.path().join("runtime");
    let home = fixture.path().join("home");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args([
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            ".",
            "--write",
            "--format",
            "json",
        ])
        .current_dir(repo_root())
        .env("HOME", &home)
        .env("AGS_HOME", &runtime)
        .env("AGS_SOURCE_ROOT", repo_root())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert_success(&output, "capability snapshot --write --format json");
    let snapshot: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["host"], "codex");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Static capability snapshot"));
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
