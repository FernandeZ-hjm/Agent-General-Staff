use super::*;
use std::path::Path;

#[test]
fn test_scope_from_str() {
    assert_eq!(Scope::from_str("local").unwrap(), Scope::Local);
    assert_eq!(Scope::from_str("full").unwrap(), Scope::Full);
    assert_eq!(Scope::from_str("release").unwrap(), Scope::Release);
    assert_eq!(Scope::from_str("promotion").unwrap(), Scope::Promotion);
    assert!(Scope::from_str("invalid").is_err());
}

#[test]
fn test_scope_display() {
    assert_eq!(Scope::Local.to_string(), "local");
    assert_eq!(Scope::Full.to_string(), "full");
    assert_eq!(Scope::Release.to_string(), "release");
    assert_eq!(Scope::Promotion.to_string(), "promotion");
}

#[test]
fn test_check_item_pass() {
    let item = CheckItem::pass("test-check", "local", "all good");
    assert_eq!(item.status, CheckStatus::Pass);
    assert_eq!(item.severity, Severity::Info);
    assert_eq!(item.exit_code, Some(0));
}

#[test]
fn test_check_item_fail() {
    let item = CheckItem::fail("test-check", "local", "broken", "fix it");
    assert_eq!(item.status, CheckStatus::Fail);
    assert_eq!(item.severity, Severity::Error);
    assert_eq!(item.remediation, Some("fix it".to_string()));
}

#[test]
fn test_check_item_skip() {
    let item = CheckItem::skip("test-check", "local", "not available");
    assert_eq!(item.status, CheckStatus::Skip);
    assert_eq!(item.exit_code, None);
}

#[test]
fn test_check_item_warn() {
    let item = CheckItem::warn("test-check", "local", "advisory", "review");
    assert_eq!(item.status, CheckStatus::Fail);
    assert_eq!(item.severity, Severity::Warn);
}

#[test]
fn test_check_item_builder() {
    let item = CheckItem::pass("test", "local", "ok")
        .with_command("echo hi")
        .with_exit_code(0);
    assert_eq!(item.command, Some("echo hi".to_string()));
    assert_eq!(item.exit_code, Some(0));
}

#[test]
fn test_empty_report_passes() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Local,
        repo_root: "/tmp".to_string(),
        items: vec![],
        summary: VerificationSummary {
            total: 0,
            passed: 0,
            failed: 0,
            skipped: 0,
            errors: 0,
            warnings: 0,
        },
    };
    assert!(report.passed());
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn release_report_with_required_skip_fails_closed() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Release,
        repo_root: "/tmp".to_string(),
        items: vec![CheckItem::skip(
            "release-public-root",
            "release",
            "required release input unavailable",
        )],
        summary: VerificationSummary {
            total: 1,
            passed: 0,
            failed: 0,
            skipped: 1,
            errors: 0,
            warnings: 0,
        },
    };

    assert!(!report.passed());
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn promotion_requires_explicit_public_root() {
    let dir = tempfile::tempdir().unwrap();
    let items = check_promotion_boundary(dir.path(), None);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "promotion-public-root-required");
    assert_eq!(items[0].status, CheckStatus::Fail);
    assert_eq!(items[0].severity, Severity::Error);
}

#[test]
fn release_scope_uses_explicit_public_source_when_supplied() {
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
fn promotion_accepts_only_structured_legal_redactions() {
    let safe = r#"{"projects":[{"drifts":[{"code":"DRIFT_LEGAL_REDACTION","kind":"legal_redaction","severity":"info"}]}]}"#;
    assert_eq!(allowlisted_promotion_redaction_count(safe), Some(1));

    let blocking = r#"{"projects":[{"drifts":[{"code":"INVARIANT_MISSING","kind":"invariant","severity":"error"}]}]}"#;
    assert_eq!(allowlisted_promotion_redaction_count(blocking), None);
    assert_eq!(allowlisted_promotion_redaction_count("not-json"), None);
}

fn write_release_version_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("packages/ags-mcp")).unwrap();
    std::fs::create_dir_all(root.join("manifests")).unwrap();
    std::fs::create_dir_all(root.join("crates/ags-governance-decision/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/ags-task-contract/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/ags-mcp/src")).unwrap();
    for package in [
        "ags-platform",
        "ags-workspace-facts",
        "ags-host-integration",
        "ags-capability-governance",
        "ags-task-contract",
        "ags-governance-decision",
        "ags-session",
        "ags-evidence",
        "ags-verification",
        "ags-lifecycle",
        "ags-cli",
        "ags-mcp",
    ] {
        let path = root.join("crates").join(package).join("Cargo.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(
                "[package]\nname = \"{package}\"\nversion.workspace = true\nlicense.workspace = true\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        root.join("packages/ags-mcp/package.json"),
        format!(
            r#"{{"version":"{}","license":"GPL-3.0-only"}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    for (relative, content) in [
        (
            "AGENT_SUITE_PROTOCOL.md",
            format!(
                "Current product version: **{}**.",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (
            "RELEASE_NOTES.md",
            format!(
                "## Release {}\n## Release 0.3.1\n## Release 0.3.0",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (
            "README.md",
            format!("latest release 是 **v{}**", env!("CARGO_PKG_VERSION")),
        ),
        (
            "README_EN.md",
            format!("are **v{}**.", env!("CARGO_PKG_VERSION")),
        ),
        (
            "packages/ags-mcp/README.md",
            format!("`v{}` GitHub", env!("CARGO_PKG_VERSION")),
        ),
        (
            "Cargo.toml",
            format!(
                "[workspace.package]\nversion = \"{}\"\nlicense = \"GPL-3.0-only\"",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        ("SECURITY.md", "| 0.3.x | Yes |".to_string()),
        ("LICENSE", "GPL-3.0-only".to_string()),
        ("packages/ags-mcp/LICENSE", "GPL-3.0-only".to_string()),
        (
            "manifests/suite.yaml",
            format!("suite:\n  version: \"{}\"", env!("CARGO_PKG_VERSION")),
        ),
        (
            "manifests/mcp-registry.yaml",
            format!(
                "suite_interfaces:\n  - name: ags\n    package:\n      version: \"{}\"\n      license: \"GPL-3.0-only\"",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (
            "protocol/mcp-server.md",
            format!("AGS {} MCP", env!("CARGO_PKG_VERSION")),
        ),
        (
            "global-skills/ags-agents/SKILL.md",
            format!("AGS 产品版本：{}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "global-skills/ags-capability/SKILL.md",
            format!("AGS 产品版本：{}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "global-skills/ags-doctor/SKILL.md",
            format!("AGS 产品版本：{}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "global-skills/ags-init/SKILL.md",
            format!("AGS 产品版本：{}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "global-skills/ags-setup/SKILL.md",
            format!(
                "ags setup --yes --force\nAGS 产品版本：{}",
                env!("CARGO_PKG_VERSION")
            ),
        ),
        (
            "global-skills/ags-skill/SKILL.md",
            format!("AGS 产品版本：{}", env!("CARGO_PKG_VERSION")),
        ),
        (
            "crates/ags-governance-decision/src/lib.rs",
            "0.3.0-host-route-proposal\n0.3.0-route-resolution".to_string(),
        ),
        (
            "crates/ags-task-contract/src/intent.rs",
            "0.3.0-handoff-contract".to_string(),
        ),
        (
            "crates/ags-task-contract/src/runner.rs",
            "0.3.0-launch-plan".to_string(),
        ),
        (
            "crates/ags-lifecycle/src/onboarding/mod.rs",
            "0.3.0-onboarding-plan".to_string(),
        ),
        (
            "crates/ags-capability-governance/src/authority.rs",
            concat!(
                "0.3.0-host-capability-snapshot\n",
                "0.3.0-user-skill-overlay\n",
                "0.3.0-user-skill-sources\n",
                "0.3.0-overlay-mutation-receipt\n",
                "0.3.0-skill-usage-event"
            )
            .to_string(),
        ),
        (
            "crates/ags-session/src/workspace_service/registry_ownership.rs",
            "0.3.0-workspace-service".to_string(),
        ),
        (
            "crates/ags-capability-governance/src/skill_body/console/model.rs",
            "0.3.0-skill-console".to_string(),
        ),
        (
            "crates/ags-capability-governance/src/adoption/model.rs",
            "0.3.0-skill-adoption-plan".to_string(),
        ),
        (
            "crates/ags-mcp/src/protocol.rs",
            "pub const SERVER_VERSION: &str = env!(\"CARGO_PKG_VERSION\");".to_string(),
        ),
        (
            "crates/ags-mcp/src/server.rs",
            "version: SERVER_VERSION.to_string()".to_string(),
        ),
    ] {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
}

#[test]
fn release_version_surfaces_reject_one_drifting_workspace_package() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());
    std::fs::write(
        dir.path().join("crates/ags-session/Cargo.toml"),
        "[package]\nname = \"ags-session\"\nversion = \"9.9.9\"\nlicense.workspace = true\n",
    )
    .unwrap();

    let item = check_release_version_surfaces(dir.path());
    assert_eq!(item.status, CheckStatus::Fail);
    assert!(item
        .evidence
        .contains("crates/ags-session/Cargo.toml package.version must be 0.3.2, found 9.9.9"));
}

#[test]
fn release_version_surfaces_reject_mcp_server_info_version_decoupling() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());
    std::fs::write(
        dir.path().join("crates/ags-mcp/src/protocol.rs"),
        "pub const SERVER_VERSION: &str = \"0.3.0\";",
    )
    .unwrap();

    let item = check_release_version_surfaces(dir.path());
    assert_eq!(item.status, CheckStatus::Fail);
    assert!(item
        .evidence
        .contains("MCP serverInfo.version from the product package version"));
}

#[test]
fn release_version_surfaces_reject_retired_setup_flag_in_command_skill() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());
    std::fs::write(
        dir.path().join("global-skills/ags-setup/SKILL.md"),
        format!(
            "ags setup --with-evomap --yes\nAGS 产品版本：{}",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let item = check_release_version_surfaces(dir.path());
    assert_eq!(item.status, CheckStatus::Fail);
    assert!(item
        .evidence
        .contains("still references retired flag --with-evomap"));
    assert!(item
        .evidence
        .contains("is missing current command: ags setup --yes --force"));
}

#[test]
fn release_version_surfaces_keep_product_schema_and_history_separate() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());

    std::fs::write(
        dir.path().join("crates/ags-governance-decision/src/lib.rs"),
        "0.3.2-host-route-proposal",
    )
    .unwrap();
    let schema_item = check_release_version_surfaces(dir.path());
    assert_eq!(schema_item.status, CheckStatus::Fail);
    assert!(schema_item
        .evidence
        .contains("must retain compatibility schema marker 0.3.0-host-route-proposal"));

    write_release_version_fixture(dir.path());
    std::fs::write(
        dir.path().join("RELEASE_NOTES.md"),
        format!("## Release {}", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    let history_item = check_release_version_surfaces(dir.path());
    assert_eq!(history_item.status, CheckStatus::Fail);
    assert!(history_item
        .evidence
        .contains("must retain the v0.3.0 and v0.3.1 history sections"));
}

#[test]
fn release_version_surfaces_accept_aligned_product_metadata() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());

    let item = check_release_version_surfaces(dir.path());
    assert_eq!(item.status, CheckStatus::Pass, "{}", item.evidence);

    // Public release trees intentionally omit command skill bodies and the
    // private source-adoption implementation. The release version gate must
    // still pass while the public manifest keeps both surfaces forbidden.
    std::fs::remove_dir_all(dir.path().join("global-skills")).unwrap();
    std::fs::remove_dir_all(
        dir.path()
            .join("crates/ags-capability-governance/src/adoption"),
    )
    .unwrap();
    let public_item = check_release_version_surfaces(dir.path());
    assert_eq!(
        public_item.status,
        CheckStatus::Pass,
        "{}",
        public_item.evidence
    );
}

#[test]
fn release_version_surfaces_reject_decoys_outside_authoritative_fields() {
    let dir = tempfile::tempdir().unwrap();
    write_release_version_fixture(dir.path());

    std::fs::write(
        dir.path().join("Cargo.toml"),
        format!(
            "[package]\nversion = \"{}\"\nlicense = \"GPL-3.0-only\"\n\
             [workspace.package]\nversion = \"9.9.9\"\nlicense = \"MIT\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("manifests/suite.yaml"),
        format!(
            "decoy:\n  version: \"{}\"\nsuite:\n  version: \"9.9.9\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("manifests/mcp-registry.yaml"),
        format!(
            "decoy:\n  version: \"{}\"\n  license: GPL-3.0-only\n\
             suite_interfaces:\n  - name: ags\n    package:\n      version: \"9.9.9\"\n      license: MIT\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();

    let item = check_release_version_surfaces(dir.path());
    assert_eq!(item.status, CheckStatus::Fail, "{}", item.evidence);
    assert!(item.evidence.contains("workspace.package.version"));
    assert!(item.evidence.contains("workspace.package.license"));
    assert!(item.evidence.contains("suite.version"));
    assert!(item
        .evidence
        .contains("suite_interfaces[name=ags].package.version"));
    assert!(item
        .evidence
        .contains("suite_interfaces[name=ags].package.license"));
}

#[test]
fn release_version_surfaces_accept_the_real_workspace_tree() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let item = check_release_version_surfaces(&repo_root);
    assert_eq!(item.status, CheckStatus::Pass, "{}", item.evidence);
}

#[test]
fn test_report_with_failures() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Local,
        repo_root: "/tmp".to_string(),
        items: vec![
            CheckItem::pass("a", "local", "ok"),
            CheckItem::fail("b", "local", "broken", "fix"),
        ],
        summary: VerificationSummary {
            total: 2,
            passed: 1,
            failed: 1,
            skipped: 0,
            errors: 1,
            warnings: 0,
        },
    };
    assert!(!report.passed());
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn test_report_with_only_warnings_passes() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Full,
        repo_root: "/tmp".to_string(),
        items: vec![
            CheckItem::pass("a", "local", "ok"),
            CheckItem::warn("b", "full", "advisory", "review"),
        ],
        summary: VerificationSummary {
            total: 2,
            passed: 1,
            failed: 1,
            skipped: 0,
            errors: 0,
            warnings: 1,
        },
    };
    assert!(report.passed());
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn test_render_json_produces_valid_json() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Local,
        repo_root: "/tmp/test".to_string(),
        items: vec![CheckItem::pass("t1", "local", "ok")],
        summary: VerificationSummary {
            total: 1,
            passed: 1,
            failed: 0,
            skipped: 0,
            errors: 0,
            warnings: 0,
        },
    };
    let json = render_json(&report);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["schema_version"], "2.0-verify");
    assert_eq!(parsed["scope"], "local");
    assert_eq!(parsed["summary"]["total"], 1);
    assert_eq!(parsed["summary"]["passed"], 1);
}

#[test]
fn test_render_text_contains_summary() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Local,
        repo_root: "/tmp/test".to_string(),
        items: vec![
            CheckItem::pass("t1", "local", "check passed"),
            CheckItem::fail("t2", "local", "check failed", "run fix"),
            CheckItem::skip("t3", "local", "not available"),
        ],
        summary: VerificationSummary {
            total: 3,
            passed: 1,
            failed: 1,
            skipped: 1,
            errors: 1,
            warnings: 0,
        },
    };
    let text = render_text(&report);
    assert!(text.contains("PASS"));
    assert!(text.contains("FAIL"));
    assert!(text.contains("SKIP"));
    assert!(text.contains("Summary:"));
    assert!(text.contains("Verdict: FAIL"));
}

#[test]
fn test_governance_yaml_parse_valid() {
    // Test with inline valid YAML
    let valid_yaml = "key: value\nitems:\n  - a\n  - b\n";
    let result = serde_yaml::from_str::<serde_yaml::Value>(valid_yaml);
    assert!(result.is_ok());
}

#[test]
fn test_governance_yaml_parse_invalid() {
    let invalid_yaml = "key: value\n\t- tab indent\n";
    let result = serde_yaml::from_str::<serde_yaml::Value>(invalid_yaml);
    assert!(result.is_err());
}

#[test]
fn test_truncate() {
    assert_eq!(truncate("hello", 10), "hello");
    assert_eq!(truncate("hello world", 5), "hello...");
    assert_eq!(truncate("", 5), "");
}

#[test]
fn test_check_status_display() {
    assert_eq!(CheckStatus::Pass.to_string(), "pass");
    assert_eq!(CheckStatus::Fail.to_string(), "fail");
    assert_eq!(CheckStatus::Skip.to_string(), "skip");
}

#[test]
fn test_severity_ordering() {
    assert!(Severity::Info < Severity::Warn);
    assert!(Severity::Warn < Severity::Error);
}

#[test]
fn test_json_roundtrip_checkitem() {
    let item = CheckItem {
        id: "test".to_string(),
        scope: "local".to_string(),
        status: CheckStatus::Pass,
        severity: Severity::Info,
        evidence: "ok".to_string(),
        remediation: None,
        command: Some("cmd".to_string()),
        exit_code: Some(0),
    };
    let json = serde_json::to_string(&item).unwrap();
    let parsed: CheckItem = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.id, "test");
    assert_eq!(parsed.status, CheckStatus::Pass);
}

#[test]
fn test_json_roundtrip_report() {
    let report = VerificationReport {
        schema_version: "2.0-verify".to_string(),
        scope: Scope::Full,
        repo_root: "/test".to_string(),
        items: vec![
            CheckItem::pass("a", "local", "ok"),
            CheckItem::fail("b", "full", "bad", "fix"),
            CheckItem::warn("c", "full", "advisory", "review"),
        ],
        summary: VerificationSummary {
            total: 3,
            passed: 1,
            failed: 2,
            skipped: 0,
            errors: 1,
            warnings: 1,
        },
    };
    let json = serde_json::to_string(&report).unwrap();
    let parsed: VerificationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.schema_version, "2.0-verify");
    assert_eq!(parsed.scope, Scope::Full);
    assert_eq!(parsed.items.len(), 3);
    assert!(!parsed.passed());
}

#[test]
fn test_run_command_executes_in_repo_root() {
    let root = std::env::temp_dir().join(format!("ags-verify-cwd-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let manifest = root.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"ags-cwd-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let expected = manifest.canonicalize().unwrap_or_else(|_| manifest.clone());

    let (code, stdout, stderr) = run_command(
        &root,
        env!("CARGO"),
        &["locate-project", "--message-format", "plain"],
        &[],
    );
    let actual = Path::new(stdout.trim())
        .canonicalize()
        .unwrap_or_else(|_| Path::new(stdout.trim()).to_path_buf());
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(actual, expected);
}

#[test]
fn test_session_preflight_failure_records_explicit_target() {
    let root = std::env::temp_dir().join(format!(
        "ags-verify-preflight-target-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();

    let item = check_session_preflight(&root);
    let _ = std::fs::remove_dir_all(&root);

    let command = item.command.unwrap_or_default();
    assert!(
        command.contains("--target"),
        "preflight command must carry explicit --target: {command}"
    );
    assert!(
        command.contains(&root.to_string_lossy().to_string()),
        "preflight command must use the repo root target: {command}"
    );
    assert!(
        item.remediation.unwrap_or_default().contains("--target"),
        "preflight remediation must preserve target authority"
    );
}

// ── Template leak detection tests ──────────────────────────────────

#[test]
fn test_longest_hex_run() {
    assert_eq!(longest_hex_run(""), 0);
    assert_eq!(longest_hex_run("xyzzy"), 0);
    assert_eq!(longest_hex_run("abc123"), 6);
    assert_eq!(longest_hex_run("abc xyz 123"), 3);
    assert_eq!(longest_hex_run(&"a".repeat(65)), 65);
}

#[test]
fn template_leak_detection_flags_real_user_path() {
    let content = "proxy_url: \"/home/tester/.evolver/settings.json\"";
    let leaks = detect_template_leaks(content, "test.yaml");
    assert!(!leaks.is_empty(), "should detect /home/tester path leak");
}

#[test]
fn template_leak_detection_ignores_grep_command() {
    let content = "grep -E '/Users/' templates/ -r  # check for leaks";
    let leaks = detect_template_leaks(content, "test.md");
    assert!(leaks.is_empty(), "should ignore grep commands with /Users/");
}

#[test]
fn template_leak_detection_flags_node_command_with_real_user_path() {
    let content = "node /home/tester/.evolver/run-hook.js";
    let leaks = detect_template_leaks(content, "test.md");
    assert!(
        !leaks.is_empty(),
        "should detect /home/ path in node command"
    );
}

#[test]
fn template_leak_detection_flags_python_command_with_real_user_path() {
    let content = "python3 /home/tester/scripts/evolver.py";
    let leaks = detect_template_leaks(content, "test.md");
    assert!(
        !leaks.is_empty(),
        "should detect /home/ path in python command"
    );
}

#[test]
fn template_leak_detection_flags_comments_with_paths() {
    // Real /home/<name> paths in comments are detected.
    let content = "# /home/tester/.evolver/settings.json";
    let leaks = detect_template_leaks(content, "test.yaml");
    assert!(
        !leaks.is_empty(),
        "should detect /home/ path even in comments"
    );
}

#[test]
fn template_leak_detection_accepts_safe_comment() {
    // Comments without real paths or tokens are fine.
    let content = "# This template uses a token file for authentication.";
    let leaks = detect_template_leaks(content, "test.yaml");
    assert!(leaks.is_empty(), "safe comments should pass");
}

#[test]
fn template_leak_detection_ignores_replace_slots() {
    let content = "\"REPLACE: path/to/evolver-recall-script\"";
    let leaks = detect_template_leaks(content, "test.json");
    assert!(leaks.is_empty(), "should ignore REPLACE slot lines");
}

#[test]
fn template_leak_detection_flags_long_hex_token() {
    let hex64 = "a".repeat(64);
    let content = format!("token: \"{hex64}\"");
    let leaks = detect_template_leaks(&content, "test.yaml");
    assert!(!leaks.is_empty(), "should detect 64-char hex token");
}

#[test]
fn template_leak_detection_ignores_short_hex() {
    let content = "hash: \"abc123def456\""; // short hex, not a token
    let leaks = detect_template_leaks(content, "test.yaml");
    assert!(leaks.is_empty(), "should not flag short hex strings");
}

#[test]
fn check_runtime_profile_templates_pass_in_ags_repo() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let item = check_runtime_profile_templates(repo_root);
    assert_eq!(
        item.status,
        CheckStatus::Pass,
        "templates should pass: {}",
        item.evidence
    );
}
