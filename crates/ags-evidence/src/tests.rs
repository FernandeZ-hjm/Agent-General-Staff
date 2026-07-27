use super::*;
use std::fs;

fn write_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn sha256_produces_64_char_hex() {
    let hash = sha256_hex(b"hello world");
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn sha256_is_deterministic() {
    let a = sha256_hex(b"test");
    let b = sha256_hex(b"test");
    assert_eq!(a, b);
}

#[test]
fn sha256_different_for_different_input() {
    let a = sha256_hex(b"foo");
    let b = sha256_hex(b"bar");
    assert_ne!(a, b);
}

#[test]
fn hash_file_matches_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_temp_file(&dir, "test.md", "## Task Card\ncontent here\n");
    let hash = hash_file(&path).unwrap();
    let expected = sha256_hex(b"## Task Card\ncontent here\n");
    assert_eq!(hash, expected);
}

#[test]
fn hash_file_error_on_missing() {
    let result = hash_file(Path::new("/no/such/file.txt"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot read"));
}

#[test]
fn generate_receipt_produces_valid_schema() {
    let dir = tempfile::tempdir().unwrap();
    let task_card = write_temp_file(&dir, "task.md", "## 任务卡\n任务：test\n");
    let delivery = write_temp_file(&dir, "delivery.md", "# Delivery Report\n");

    let receipt = generate_receipt(
        &task_card,
        "allow",
        None,
        vec![VerificationResult {
            command: "cargo test".to_string(),
            exit_code: 0,
            output_hash: sha256_hex(b"all tests passed"),
        }],
        Some(&delivery),
    )
    .unwrap();

    assert_eq!(receipt.schema_version, RECEIPT_SCHEMA_VERSION);
    assert_eq!(
        receipt.governance_status,
        Some(ags_governance_decision::GovernanceStatus::DoneWithReceipt)
    );
    assert!(receipt.receipt_id.starts_with("receipt-"));
    assert!(!receipt.task_card_hash.is_empty());
    assert_eq!(receipt.gate_result.decision, "allow");
    assert!(receipt.gate_result.reason.is_none());
    assert_eq!(receipt.verification_results.len(), 1);
    assert!(receipt.delivery_report_hash.is_some());
}

#[test]
fn generate_receipt_with_gate_reason() {
    let dir = tempfile::tempdir().unwrap();
    let task_card = write_temp_file(&dir, "task.md", "## 任务卡\n");

    let receipt = generate_receipt(
        &task_card,
        "stop",
        Some("writable-parallelism-blocked-by-permission"),
        vec![],
        None,
    )
    .unwrap();

    assert_eq!(receipt.gate_result.decision, "stop");
    assert_eq!(
        receipt.gate_result.reason.as_deref(),
        Some("writable-parallelism-blocked-by-permission")
    );
}

#[test]
fn writer_adds_typed_governance_evidence_without_raw_request() {
    let dir = tempfile::tempdir().unwrap();
    let task_card = write_temp_file(&dir, "task.md", "## 任务卡\n任务：test\n");
    let receipt = generate_receipt_with_governance(
        &task_card,
        "allow",
        None,
        vec![],
        None,
        ags_governance_decision::GovernanceStatus::DoneWithReceipt,
        GovernanceEvidence {
            decision_id: Some("decision-1".to_string()),
            lease_id: Some("lease-1".to_string()),
            proposal_hash: Some("sha256:proposal".to_string()),
            solution_state: Some(ags_governance_decision::SolutionState::Confirmed),
            scope_hash: Some("sha256:scope".to_string()),
            snapshot_hash: Some("sha256:snapshot".to_string()),
            policy_hash: Some("sha256:policy".to_string()),
            skill_selection: Some(ReceiptSkillSelection {
                skill_id: "codebase-design".to_string(),
                entrypoint: Some("module-design".to_string()),
            }),
        },
    )
    .unwrap();
    let json = render_receipt_json(&receipt);
    assert_eq!(receipt.schema_version, RECEIPT_SCHEMA_VERSION);
    assert!(!json.contains("raw_request"));
    assert_eq!(
        receipt
            .governance_evidence
            .as_ref()
            .and_then(|evidence| evidence.skill_selection.as_ref())
            .map(|selection| selection.skill_id.as_str()),
        Some("codebase-design")
    );

    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["governance_evidence"]["raw_request"] = serde_json::json!("secret prompt");
    assert!(serde_json::from_value::<Receipt>(value).is_err());
}

#[test]
fn generate_receipt_error_on_missing_task_card() {
    let result = generate_receipt(Path::new("/no/such/task.md"), "allow", None, vec![], None);
    assert!(result.is_err());
}

#[test]
fn verify_valid_receipt_passes_all_checks() {
    let dir = tempfile::tempdir().unwrap();
    let task_content = "## Task Card\nverify test here\n";
    let task_card = write_temp_file(&dir, "task.md", task_content);
    let task_hash = sha256_hex(task_content.as_bytes());

    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: format!("receipt-{}", &task_hash[..12]),
        timestamp: "unix-0".to_string(),
        task_card_hash: task_hash,
        task_card_path: Some(task_card.display().to_string()),
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![VerificationResult {
            command: "cargo test".to_string(),
            exit_code: 0,
            output_hash: sha256_hex(b"ok"),
        }],
        delivery_report_hash: None,
        exit_code: Some(0),
        governance_status: None,
        governance_evidence: None,
    };

    let result = verify_receipt(&receipt);
    assert!(result.valid, "should be valid but got: {:?}", result.checks);

    // Check specific items
    let schema_check = result
        .checks
        .iter()
        .find(|c| c.name == "schema_version")
        .unwrap();
    assert!(schema_check.passed);

    let hash_check = result
        .checks
        .iter()
        .find(|c| c.name == "task_card_hash")
        .unwrap();
    assert!(hash_check.passed);
    assert!(hash_check.detail.contains("hash matches"));
}

#[test]
fn verify_detects_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let task_card = write_temp_file(&dir, "task.md", "original content\n");

    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-abc123".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "00deadbeef000000000000000000000000000000000000000000000000000000"
            .to_string(),
        task_card_path: Some(task_card.display().to_string()),
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![],
        delivery_report_hash: None,
        exit_code: None,
        governance_status: None,
        governance_evidence: None,
    };

    let result = verify_receipt(&receipt);
    assert!(!result.valid);
    let hash_check = result
        .checks
        .iter()
        .find(|c| c.name == "task_card_hash")
        .unwrap();
    assert!(!hash_check.passed);
    assert!(hash_check.detail.contains("hash mismatch"));
}

#[test]
fn verify_handles_missing_schema_version() {
    let receipt = Receipt {
        schema_version: "0.1-old".to_string(),
        receipt_id: "receipt-abc".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "abc123".to_string(),
        task_card_path: None,
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![],
        delivery_report_hash: None,
        exit_code: None,
        governance_status: None,
        governance_evidence: None,
    };

    let result = verify_receipt(&receipt);
    assert!(!result.valid);
    let schema_check = result
        .checks
        .iter()
        .find(|c| c.name == "schema_version")
        .unwrap();
    assert!(!schema_check.passed);
    assert!(schema_check.detail.contains("0.1-old"));
}

#[test]
fn compliance_check_allows_valid_receipt() {
    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-abc123".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "abc123".to_string(),
        task_card_path: None,
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![
            VerificationResult {
                command: "cargo test".to_string(),
                exit_code: 0,
                output_hash: sha256_hex(b"ok"),
            },
            VerificationResult {
                command: "cargo build".to_string(),
                exit_code: 0,
                output_hash: sha256_hex(b"ok"),
            },
        ],
        delivery_report_hash: None,
        exit_code: Some(0),
        governance_status: None,
        governance_evidence: None,
    };

    let result = check_compliance(&receipt);
    assert!(result.compliant);

    let gate_check = result
        .checks
        .iter()
        .find(|c| c.name == "gate_decision")
        .unwrap();
    assert!(gate_check.passed);

    let verify_check = result
        .checks
        .iter()
        .find(|c| c.name == "verification_results")
        .unwrap();
    assert!(verify_check.passed);
}

#[test]
fn compliance_check_rejects_stop_decision() {
    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-stop1".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "abc123".to_string(),
        task_card_path: None,
        gate_result: GateResult {
            decision: "stop".to_string(),
            reason: Some("writable-parallelism-blocked-by-permission".to_string()),
        },
        verification_results: vec![],
        delivery_report_hash: None,
        exit_code: None,
        governance_status: None,
        governance_evidence: None,
    };

    let result = check_compliance(&receipt);
    assert!(!result.compliant);

    let gate_check = result
        .checks
        .iter()
        .find(|c| c.name == "gate_decision")
        .unwrap();
    assert!(!gate_check.passed);
    assert!(gate_check.detail.contains("stop"));
    assert!(gate_check
        .detail
        .contains("writable-parallelism-blocked-by-permission"));
}

#[test]
fn compliance_check_rejects_failed_verification() {
    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-fail1".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "abc123".to_string(),
        task_card_path: None,
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![VerificationResult {
            command: "cargo test".to_string(),
            exit_code: 1,
            output_hash: sha256_hex(b"2 tests failed"),
        }],
        delivery_report_hash: None,
        exit_code: Some(1),
        governance_status: None,
        governance_evidence: None,
    };

    let result = check_compliance(&receipt);
    assert!(!result.compliant);

    let verify_check = result
        .checks
        .iter()
        .find(|c| c.name == "verification_results")
        .unwrap();
    assert!(!verify_check.passed);
    assert!(verify_check.detail.contains("failed"));
    assert!(verify_check.detail.contains("cargo test"));
    assert!(verify_check.detail.contains("exit 1"));
}

#[test]
fn compliance_check_includes_verify_checks() {
    // Even when verify passes, compliance should include all verify checks
    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-combo1".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: "abc123".to_string(),
        task_card_path: None,
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![VerificationResult {
            command: "echo ok".to_string(),
            exit_code: 0,
            output_hash: sha256_hex(b"ok"),
        }],
        delivery_report_hash: None,
        exit_code: None,
        governance_status: None,
        governance_evidence: None,
    };

    let result = check_compliance(&receipt);
    assert!(result.compliant);

    // Verify checks should be included with verify: prefix
    let schema_verify = result
        .checks
        .iter()
        .find(|c| c.name == "verify:schema_version")
        .unwrap();
    assert!(schema_verify.passed);
}

#[test]
fn render_receipt_json_is_valid() {
    let receipt = Receipt {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-test1".to_string(),
        timestamp: "unix-0".to_string(),
        task_card_hash: sha256_hex(b"test"),
        task_card_path: Some("/tmp/task.md".to_string()),
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![],
        delivery_report_hash: None,
        exit_code: Some(0),
        governance_status: None,
        governance_evidence: None,
    };

    let json = render_receipt_json(&receipt);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["schema_version"], "0.3.4-task-receipt");
    assert_eq!(parsed["receipt_id"], "receipt-test1");
}

#[test]
fn verify_result_json_includes_failed_checks() {
    let result = VerifyResult {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-bad".to_string(),
        valid: false,
        checks: vec![
            CheckItem {
                name: "schema_version".to_string(),
                passed: false,
                detail: "expected 0.3.4-task-receipt, got 1.0".to_string(),
            },
            CheckItem {
                name: "task_card_hash".to_string(),
                passed: false,
                detail: "hash mismatch".to_string(),
            },
        ],
    };

    let json = render_verify_json(&result);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["valid"], false);
    let checks = parsed["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2);
    // Each failed check should include detail
    for check in checks {
        assert!(!check["detail"].as_str().unwrap().is_empty());
    }
}

#[test]
fn compliance_result_json_includes_specific_failures() {
    let result = ComplianceResult {
        schema_version: "0.3.4-task-receipt".to_string(),
        receipt_id: "receipt-fail".to_string(),
        compliant: false,
        checks: vec![CheckItem {
            name: "gate_decision".to_string(),
            passed: false,
            detail: "gate decision is 'stop' — task should not have been executed".to_string(),
        }],
    };

    let json = render_compliance_json(&result);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["compliant"], false);
    assert!(!parsed["checks"][0]["detail"].as_str().unwrap().is_empty());
}

// ── Action receipt tests ──────────────────────────────────────────────

fn sample_write() -> ReceiptWrite {
    ReceiptWrite {
        op: "create".to_string(),
        path: "/tmp/ags/x".to_string(),
        from: None,
        detail: "created".to_string(),
    }
}

#[test]
fn emit_action_receipt_writes_file_and_returns_path() {
    let dir = tempfile::tempdir().unwrap();
    let r = build_action_receipt(
        "setup-apply",
        Some("/tmp/ags"),
        GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        vec![],
        vec![sample_write()],
        vec![],
        vec![],
        "applied",
        true,
    );
    let path = emit_action_receipt(dir.path(), &r).unwrap();
    assert!(path.exists());
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ar-setup-apply-"));
    let back: ActionReceipt =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back.action, "setup-apply");
    assert_eq!(back.apply_status, "applied");
    assert_eq!(back.schema_version, "0.3.4-action-receipt");
}

#[test]
fn build_action_receipt_derives_stable_prefix() {
    let r = build_action_receipt(
        "skill-apply",
        None,
        GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        vec![],
        vec![],
        vec![],
        vec![],
        "advised-only",
        false,
    );
    assert!(r.receipt_id.starts_with("ar-skill-apply-"));
    assert_eq!(r.schema_version, "0.3.4-action-receipt");
    assert!(!r.applied);
}

#[test]
fn emit_refuses_secret_in_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = sample_write();
    w.detail = "Bearer abcdefghijklmnopqrstuvwxyz0123".to_string();
    let r = build_action_receipt(
        "agents-govern",
        None,
        GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        vec![],
        vec![w],
        vec![],
        vec![],
        "applied",
        true,
    );
    assert!(emit_action_receipt(dir.path(), &r).is_err());
}

#[test]
fn emit_action_receipt_never_overwrites_on_id_collision() {
    let dir = tempfile::tempdir().unwrap();
    // Force two receipts to share an id; the second must NOT overwrite the
    // first (mutation evidence must never be lost).
    let mut r = build_action_receipt(
        "agents-govern",
        None,
        GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        vec![],
        vec![],
        vec![],
        vec![],
        "advised-only",
        false,
    );
    r.receipt_id = "ar-fixed-collision-id".to_string();
    let p1 = emit_action_receipt(dir.path(), &r).unwrap();
    let p2 = emit_action_receipt(dir.path(), &r).unwrap();
    assert_ne!(p1, p2, "second receipt must not overwrite the first");
    assert!(p1.exists() && p2.exists());
}
