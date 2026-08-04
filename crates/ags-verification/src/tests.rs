use super::*;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn report(scope: Scope, errors: usize, warnings: usize, skipped: usize) -> VerificationReport {
    VerificationReport {
        schema_version: "0.3.6-verification-report".to_string(),
        scope,
        repo_root: "/tmp".to_string(),
        items: Vec::new(),
        summary: VerificationSummary {
            total: errors + warnings + skipped,
            passed: 0,
            failed: errors + warnings,
            skipped,
            errors,
            warnings,
        },
    }
}

#[test]
fn scope_and_check_item_contract_matrix() {
    for (text, scope) in [
        ("local", Scope::Local),
        ("release", Scope::Release),
        ("promotion", Scope::Promotion),
    ] {
        assert_eq!(Scope::from_str(text).unwrap(), scope);
        assert_eq!(scope.to_string(), text);
    }
    assert!(Scope::from_str("invalid").is_err());

    let cases = [
        (
            CheckItem::pass("pass", "local", "ok"),
            CheckStatus::Pass,
            Severity::Info,
            Some(0),
        ),
        (
            CheckItem::fail("fail", "local", "broken", "repair"),
            CheckStatus::Fail,
            Severity::Error,
            Some(1),
        ),
        (
            CheckItem::warn("warn", "local", "review", "inspect"),
            CheckStatus::Fail,
            Severity::Warn,
            Some(0),
        ),
        (
            CheckItem::skip("skip", "local", "unavailable"),
            CheckStatus::Skip,
            Severity::Info,
            None,
        ),
    ];
    for (item, status, severity, exit_code) in cases {
        assert_eq!(item.status, status);
        assert_eq!(item.severity, severity);
        assert_eq!(item.exit_code, exit_code);
    }
}

#[test]
fn report_verdict_matrix_is_fail_closed_for_release_inputs() {
    for (scope, errors, warnings, skipped, expected) in [
        (Scope::Local, 0, 0, 0, true),
        (Scope::Local, 0, 1, 0, true),
        (Scope::Local, 1, 0, 0, false),
        (Scope::Local, 0, 0, 1, true),
        (Scope::Release, 0, 0, 1, false),
        (Scope::Promotion, 0, 0, 1, false),
    ] {
        let report = report(scope, errors, warnings, skipped);
        assert_eq!(report.passed(), expected);
        assert_eq!(report.exit_code(), if expected { 0 } else { 1 });
    }
}

#[test]
fn renderers_preserve_machine_and_human_contracts() {
    let mut report = report(Scope::Local, 1, 0, 1);
    report.items = vec![
        CheckItem::pass("pass", "local", "ok"),
        CheckItem::fail("fail", "local", "broken", "repair"),
        CheckItem::skip("skip", "local", "unavailable"),
    ];
    report.summary.total = 3;
    report.summary.passed = 1;

    let json: serde_json::Value = serde_json::from_str(&render_json(&report)).unwrap();
    assert_eq!(json["scope"], "local");
    assert_eq!(json["summary"]["errors"], 1);

    let text = render_text(&report);
    for marker in ["PASS", "FAIL", "SKIP", "Summary:", "Verdict: FAIL"] {
        assert!(text.contains(marker), "missing {marker}");
    }
}

#[test]
fn promotion_boundary_requires_explicit_target() {
    let dir = tempfile::tempdir().unwrap();
    let items = check_promotion_boundary(dir.path(), None);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "promotion-public-root-required");
    assert_eq!(items[0].severity, Severity::Error);
}

#[test]
fn release_scope_uses_only_an_explicit_public_source() {
    let private_root = Path::new("/tmp/ags-private-authority");
    let public_root = Path::new("/tmp/ags-public-release-source");
    let options = VerificationOptions {
        public_root: Some(public_root.to_path_buf()),
    };
    assert_eq!(
        super::orchestrator::release_target_root(private_root, &options),
        public_root
    );
    assert_eq!(
        super::orchestrator::release_target_root(private_root, &VerificationOptions::default()),
        private_root
    );
}

#[test]
fn release_and_promotion_scopes_do_not_replay_local_checks() {
    let root = tempfile::tempdir().unwrap();
    for (scope, options) in [
        (Scope::Release, VerificationOptions::default()),
        (Scope::Promotion, VerificationOptions::default()),
    ] {
        let report = run_verify_with_options(scope, root.path(), &options);
        assert!(
            report
                .items
                .iter()
                .all(|item| item.scope == scope.to_string()),
            "{scope} replayed checks from another scope: {:?}",
            report
                .items
                .iter()
                .map(|item| (&item.id, &item.scope))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn local_scope_does_not_replay_the_independent_source_gate() {
    assert_eq!(
        super::orchestrator::local_check_plan(true),
        vec![
            super::orchestrator::LocalCheckGroup::TaskCardFixtures,
            super::orchestrator::LocalCheckGroup::GovernanceYaml,
            super::orchestrator::LocalCheckGroup::SessionPreflight,
            super::orchestrator::LocalCheckGroup::RuntimeProfileTemplates,
        ]
    );
}

#[test]
fn release_version_surfaces_accept_the_real_workspace_tree() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let item = check_release_version_surfaces(&repo_root);
    assert_eq!(item.status, CheckStatus::Pass, "{}", item.evidence);
}

#[test]
fn command_runner_uses_the_requested_repository_root() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let (code, stdout, stderr) = run_command(dir.path(), "cmd", &["/C", "cd"], &[]);
    #[cfg(not(windows))]
    let (code, stdout, stderr) = run_command(dir.path(), "pwd", &[], &[]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        Path::new(stdout.trim()).canonicalize().unwrap(),
        dir.path().canonicalize().unwrap()
    );
}

#[test]
fn runtime_templates_reject_real_leaks_and_accept_placeholders() {
    let cases = [
        ("home=/Users/alice/private", true),
        (
            "token=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            true,
        ),
        ("home={{HOME}} token={{TOKEN}}", false),
        ("# replace /Users/example with your own path", false),
        ("short=0123456789abcdef", false),
    ];
    for (content, should_leak) in cases {
        assert_eq!(
            !detect_template_leaks(content, "fixture").is_empty(),
            should_leak,
            "{content}"
        );
    }
}

#[test]
fn evolver_stop_template_still_records_a_sanitized_method_event() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let script = workspace.join("manifests/templates/hooks/claude-code-executor-stop.template.js");
    let temp = tempfile::tempdir().unwrap();
    let transcript = temp.path().join("transcript.md");
    let method_log = temp.path().join("method-events.jsonl");
    std::fs::write(
        &transcript,
        "# 任务交付报告\n\n## 任务状态\n完成\n\n验证结果已通过。\n",
    )
    .unwrap();

    let mut child = Command::new("node")
        .arg(&script)
        .env("EVOLVER_METHOD_LOG", &method_log)
        .current_dir(temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("node is required to verify the retained EvoMap hook");
    let input = serde_json::json!({
        "cwd": temp.path(),
        "task_id": "task-12345678",
        "transcript_path": transcript,
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&input).unwrap().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "EvoMap hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let hook_output: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(hook_output["systemMessage"]
        .as_str()
        .is_some_and(|message| message.contains("Method capture recorded")));
    let event: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&method_log).unwrap().trim()).unwrap();
    assert_eq!(event["schema_version"], "ags-evolution-memory/1");
    assert_eq!(event["source"], "hook:evolver-session-end");
    assert_eq!(event["reference_id"], "task-12345678");
    assert_eq!(event["evidence_path"], "");
    assert_eq!(event["outcome"]["status"], "completed");
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains(transcript.to_string_lossy().as_ref()));
}

#[test]
fn planner_recall_template_remains_advisory_and_authority_safe() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let content = std::fs::read_to_string(
        workspace.join("manifests/templates/hooks/codex-planner-recall.template.json"),
    )
    .unwrap();
    let template: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(template["evolver_recall"], "advisory_only");
    assert_eq!(template["enabled_by_default"], false);
    assert_eq!(template["authority_boundary"]["ags_authority_wins"], true);
    for hook in ["SessionStart", "UserPromptSubmit", "PostToolUse"] {
        assert_eq!(template["hooks"][hook]["advisory_only"], true);
    }
    let boundary = template["authority_boundary"]["must_not_decide"]
        .as_array()
        .unwrap();
    assert!(boundary.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|text| text.contains("execution mode"))
    }));
}
