use super::*;
use std::path::Path;

fn report(scope: Scope, errors: usize, warnings: usize, skipped: usize) -> VerificationReport {
    VerificationReport {
        schema_version: "ags://schema/contract/v2/check-report".to_string(),
        scope,
        repo_root: "/tmp".to_string(),
        project_tests_run: false,
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
        ("governance", Scope::Governance),
        ("changes", Scope::Changes),
        ("evidence", Scope::Evidence),
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
        (Scope::Governance, 0, 0, 0, true),
        (Scope::Governance, 0, 1, 0, true),
        (Scope::Governance, 1, 0, 0, false),
        (Scope::Governance, 0, 0, 1, true),
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
    let mut report = report(Scope::Governance, 1, 0, 1);
    report.items = vec![
        CheckItem::pass("pass", "local", "ok"),
        CheckItem::fail("fail", "local", "broken", "repair"),
        CheckItem::skip("skip", "local", "unavailable"),
    ];
    report.summary.total = 3;
    report.summary.passed = 1;

    let json: serde_json::Value = serde_json::from_str(&render_json(&report)).unwrap();
    assert_eq!(json["scope"], "governance");
    assert_eq!(json["project_tests_run"], false);
    assert_eq!(json["summary"]["errors"], 1);

    let text = render_text(&report);
    for marker in ["PASS", "FAIL", "SKIP", "Summary:", "Verdict: FAIL"] {
        assert!(text.contains(marker), "missing {marker}");
    }
}

#[test]
fn successful_human_check_is_explicit_and_within_budget() {
    let report = report(Scope::Governance, 0, 0, 0);
    let text = render_text(&report);
    assert!(text.contains("Project tests run: false"));
    assert!(crate::check_human_output_budget(&text).is_ok(), "{text}");
    assert!(crate::check_json_output_budget(render_json(&report).as_bytes()).is_ok());
}

#[test]
fn evidence_scope_runs_typed_receipt_checks_without_project_tests() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let report = run_verify(Scope::Evidence, repo);
    assert!(!report.project_tests_run);
    assert_eq!(report.items.len(), 3);
    assert!(report.items.iter().all(|item| item.scope == "evidence"));
    assert!(report.passed(), "{:#?}", report.items);
}

#[test]
fn changes_scope_classifies_the_real_git_write_set_without_project_tests() {
    let repo = tempfile::tempdir().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    std::fs::write(repo.path().join("README.md"), "changed\n").unwrap();
    let report = run_verify(Scope::Changes, repo.path());
    assert!(!report.project_tests_run);
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].scope, "changes");
    assert!(report.passed(), "{:#?}", report.items);
}

#[test]
fn successful_check_with_warning_keeps_evidence_and_remediation() {
    let mut report = report(Scope::Governance, 0, 1, 0);
    report.items = vec![CheckItem::warn(
        "snapshot-stale",
        "local",
        "capability snapshot is stale",
        "refresh the snapshot",
    )];
    let text = render_text(&report);
    for expected in [
        "snapshot-stale",
        "capability snapshot is stale",
        "refresh the snapshot",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(crate::check_human_output_budget(&text).is_ok(), "{text}");
}

#[test]
fn oversized_json_without_details_store_is_blocked_without_dangling_uri() {
    let mut report = report(Scope::Governance, 0, 0, 0);
    report.items = vec![CheckItem::pass("large", "local", &"x".repeat(20_000))];
    report.summary.total = 1;
    report.summary.passed = 1;
    let rendered = render_json(&report);
    assert!(crate::check_json_output_budget(rendered.as_bytes()).is_ok());
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["status"], "blocked");
    assert_eq!(value["error_code"], "details_storage_required");
    assert!(value.get("details_uri").is_none());
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
fn npm_product_metadata_rejects_typed_version_and_license_tampering_for_every_package() {
    for (relative, field, wrong) in [
        ("packages/ags-cli/package.json", "version", "9.9.9"),
        ("packages/ags-cli/package.json", "license", "MIT"),
        ("packages/ags-launcher/package.json", "version", "9.9.9"),
        ("packages/ags-launcher/package.json", "license", "MIT"),
        ("packages/ags-mcp/package.json", "version", "9.9.9"),
        ("packages/ags-mcp/package.json", "license", "MIT"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join(relative);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let mut package = serde_json::json!({
            "name": "fixture",
            "version": env!("CARGO_PKG_VERSION"),
            "license": "GPL-3.0-only"
        });
        package[field] = serde_json::Value::String(wrong.to_string());
        std::fs::write(&destination, serde_json::to_vec(&package).unwrap()).unwrap();
        let errors = super::version::check_npm_product_metadata(
            root.path(),
            env!("CARGO_PKG_VERSION"),
            "GPL-3.0-only",
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains(relative) && error.contains(field)),
            "{relative} {field}: {errors:?}"
        );
    }
}

#[test]
fn public_ci_release_gate_requires_current_source_and_verification_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let workflow = dir.path().join(".github/workflows/ci.yml");
    std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    std::fs::write(
        &workflow,
        r#"run: |
  cargo run -q --locked -p ags-cli -- check release \
    --workspace . --format json
  cargo run -q --locked -p ags-cli -- check bundle create \
    --source public-full
  cargo run -q --locked -p ags-cli -- check bundle validate \
    --source public-full
"#,
    )
    .unwrap();

    let errors = super::version::check_public_ci_release_invocation(dir.path());
    assert!(errors.is_empty(), "{errors:?}");

    std::fs::write(
        &workflow,
        "run: ./target/release/ags check release --workspace . --format json\n",
    )
    .unwrap();
    let errors = super::version::check_public_ci_release_invocation(dir.path());
    assert!(errors
        .iter()
        .any(|error| error.contains("exact-input public gate marker")));
    assert!(errors
        .iter()
        .any(|error| error.contains("must not execute a cached target/release/ags")));
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
fn subagent_playbook_mirrors_are_byte_identical_and_task_card_precedes_defaults() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let global =
        root.join("global-skills/superpowers/playbooks/subagent-driven-development/PLAYBOOK.md");
    let adapter = root.join(
        "skill-packs/optional/ags-superpowers-adapter/playbooks/subagent-driven-development/PLAYBOOK.md",
    );
    let global_bytes = std::fs::read(&global).unwrap();
    let adapter_bytes = std::fs::read(&adapter).unwrap();
    assert_eq!(global_bytes, adapter_bytes, "playbook mirrors drifted");
    let text = String::from_utf8(global_bytes).unwrap();

    let card = text.find("1. A validated, confirmed task card").unwrap();
    let constraints = text.find("2. Explicit task constraints").unwrap();
    let defaults = text.find("3. This playbook's defaults").unwrap();
    assert!(card < constraints && constraints < defaults);
    for marker in [
        "task card's explicit `Execution mode`, `Execution",
        "topology`, and `Delegation planning` tuple",
        "An explicit no-commit constraint binds every implementer, fixer, integrator,",
        "commit SHA range, or `commits: none (no-commit)`",
        "tracked diff hash and exact untracked-file inventory",
        "The main executor is the sole integrator",
    ] {
        assert!(text.contains(marker), "missing playbook contract: {marker}");
    }
}

#[test]
fn implementer_prompt_mirrors_bind_tuple_ownership_and_no_commit_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let global = root.join(
        "global-skills/superpowers/playbooks/subagent-driven-development/implementer-prompt.md",
    );
    let adapter = root.join(
        "skill-packs/optional/ags-superpowers-adapter/playbooks/subagent-driven-development/implementer-prompt.md",
    );
    let global_bytes = std::fs::read(&global).unwrap();
    let adapter_bytes = std::fs::read(&adapter).unwrap();
    assert_eq!(
        global_bytes, adapter_bytes,
        "implementer prompt mirrors drifted"
    );
    let text = String::from_utf8(global_bytes).unwrap();
    for marker in [
        "- Execution mode: [MODE]",
        "- Execution topology: [TOPOLOGY]",
        "- Delegation planning: [DELEGATION]",
        "- Write ownership: [OWNED_PATHS]",
        "If commit policy is NO_COMMIT, do not commit, stage, push,",
        "Review-package hash, tracked diff hash, and exact untracked-file",
        "or `none (no-commit)`",
    ] {
        assert!(text.contains(marker), "missing prompt contract: {marker}");
    }
}

#[test]
fn mcp_roots_are_discovery_only_and_task_card_authority_order_is_preserved() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mcp = std::fs::read_to_string(root.join("protocol/mcp-server.md")).unwrap();
    for marker in [
        "MCP roots are discovery hints, never an authorization boundary",
        "Private `initialize.params.roots` input is ignored",
        "An explicit workspace is resolved independently of declared roots",
        "`ags_apply` accepts the sealed `action_ref`",
        "does not accept a workspace",
    ] {
        assert!(mcp.contains(marker), "missing MCP binding rule: {marker}");
    }
    assert!(!mcp.contains("an explicit workspace must be one of those canonical roots"));

    let task = std::fs::read_to_string(root.join("protocol/agent-task-protocol.md")).unwrap();
    let validation = task
        .find("after schema validation and an explicit handoff")
        .unwrap();
    let tuple = task
        .find("`Execution mode`, `Execution topology`, and `Delegation planning` tuple")
        .unwrap();
    let risk = task
        .find("adds independent review and cannot alter the tuple")
        .unwrap();
    assert!(validation < tuple && tuple < risk);
}
