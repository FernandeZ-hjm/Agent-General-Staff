use super::*;
use std::path::Path;

fn report(scope: Scope, errors: usize, warnings: usize, skipped: usize) -> VerificationReport {
    VerificationReport {
        schema_version: "0.3.4-verification-report".to_string(),
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
        ("full", Scope::Full),
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
fn promotion_boundary_requires_explicit_target_and_structured_redactions() {
    let dir = tempfile::tempdir().unwrap();
    let items = check_promotion_boundary(dir.path(), None);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "promotion-public-root-required");
    assert_eq!(items[0].severity, Severity::Error);

    let legal = r#"{"projects":[{"drifts":[{"code":"DRIFT_LEGAL_REDACTION","kind":"legal_redaction","severity":"info"}]}]}"#;
    let blocking = r#"{"projects":[{"drifts":[{"code":"INVARIANT_MISSING","kind":"invariant","severity":"error"}]}]}"#;
    assert_eq!(allowlisted_promotion_redaction_count(legal), Some(1));
    assert_eq!(allowlisted_promotion_redaction_count(blocking), None);
    assert_eq!(allowlisted_promotion_redaction_count("not-json"), None);
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
