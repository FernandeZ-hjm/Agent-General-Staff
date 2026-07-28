use super::*;
use std::fs;

fn write_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

fn valid_receipt(task_card_path: Option<String>) -> Receipt {
    let task_card_hash = task_card_path
        .as_ref()
        .map(|path| hash_file(Path::new(path)).unwrap())
        .unwrap_or_else(|| sha256_hex(b"task"));
    let launch_plan_hash = sha256_hex(b"launch-plan");
    Receipt {
        schema_version: RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: receipt_id(&task_card_hash, &launch_plan_hash),
        timestamp: "unix-0".to_string(),
        task_card_hash,
        launch_plan_hash,
        task_card_path,
        launch_plan_path: "launch-plan.json".to_string(),
        delivery_report_path: "delivery-report.md".to_string(),
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results: vec![VerificationResult {
            command: "cargo test".to_string(),
            exit_code: 0,
            output_hash: sha256_hex(b"ok"),
        }],
        delivery_report_hash: sha256_hex(b"delivery-report"),
        execution_footprint: ExecutionFootprint {
            execution_mode_used: "single-writer".to_string(),
            execution_topology_used: "single".to_string(),
            delegation_used: "none".to_string(),
        },
        closure_status: "completed".to_string(),
        exit_code: Some(0),
        governance_status: Some(ags_governance_decision::GovernanceStatus::DoneWithReceipt),
        governance_evidence: None,
    }
}

#[test]
fn hashes_are_deterministic_and_content_sensitive() {
    assert_eq!(sha256_hex(b"same"), sha256_hex(b"same"));
    assert_ne!(sha256_hex(b"same"), sha256_hex(b"different"));
    assert_eq!(sha256_hex(b"same").len(), 64);
}

#[test]
fn hash_file_reports_missing_input() {
    assert!(hash_file(Path::new("/no/such/file.txt"))
        .unwrap_err()
        .contains("cannot read"));
}

#[test]
fn receipt_identity_binds_task_card_and_launch_plan() {
    let task_hash = sha256_hex(b"task");
    assert_ne!(
        receipt_id(&task_hash, &sha256_hex(b"plan-a")),
        receipt_id(&task_hash, &sha256_hex(b"plan-b"))
    );
}

#[test]
fn verify_receipt_checks_all_three_artifact_hashes_and_source_card() {
    let dir = tempfile::tempdir().unwrap();
    let task = write_temp_file(&dir, "task.md", "canonical task");
    let receipt = valid_receipt(Some(task.display().to_string()));
    let result = verify_receipt(&receipt);
    assert!(result.valid, "{:#?}", result.checks);

    fs::write(task, "tampered task").unwrap();
    assert!(!verify_receipt(&receipt).valid);
}

#[test]
fn verify_receipt_rejects_identity_or_hash_tampering() {
    let mut receipt = valid_receipt(None);
    receipt.receipt_id = "receipt-forged".to_string();
    assert!(!verify_receipt(&receipt).valid);

    let mut receipt = valid_receipt(None);
    receipt.launch_plan_hash = "not-a-sha256".to_string();
    assert!(!verify_receipt(&receipt).valid);
}

#[test]
fn compliance_requires_allow_and_successful_verification() {
    let mut receipt = valid_receipt(None);
    assert!(check_compliance(&receipt).compliant);

    receipt.gate_result.decision = "stop".to_string();
    assert!(!check_compliance(&receipt).compliant);

    receipt.gate_result.decision = "allow".to_string();
    receipt.verification_results[0].exit_code = 1;
    assert!(!check_compliance(&receipt).compliant);
}

#[test]
fn governance_evidence_rejects_raw_request() {
    let mut receipt = valid_receipt(None);
    receipt.governance_evidence = Some(GovernanceEvidence {
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
    });
    let mut value: serde_json::Value =
        serde_json::from_str(&render_receipt_json(&receipt)).unwrap();
    value["governance_evidence"]["raw_request"] = serde_json::json!("secret prompt");
    assert!(serde_json::from_value::<Receipt>(value).is_err());
}

#[test]
fn receipt_json_uses_v036_schema() {
    let receipt = valid_receipt(None);
    let parsed: serde_json::Value = serde_json::from_str(&render_receipt_json(&receipt)).unwrap();
    assert_eq!(parsed["schema_version"], "0.3.6-task-receipt");
    assert_eq!(parsed["launch_plan_hash"], receipt.launch_plan_hash);
}
