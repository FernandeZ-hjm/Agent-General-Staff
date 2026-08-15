use super::*;
use std::fs;

fn write_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path
}

fn valid_receipt(task_card_path: Option<String>) -> Receipt {
    let task_card_hash = if task_card_path.is_some() {
        sha256_hex(b"canonical task")
    } else {
        sha256_hex(b"task")
    };
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
fn receipt_identity_binds_task_card_and_launch_plan() {
    let task_hash = sha256_hex(b"task");
    assert_ne!(
        receipt_id(&task_hash, &sha256_hex(b"plan-a")),
        receipt_id(&task_hash, &sha256_hex(b"plan-b"))
    );
}

#[test]
fn verify_receipt_is_non_authoritative_structural_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let task = write_temp_file(&dir, "task.md", "canonical task");
    let receipt = valid_receipt(Some(task.display().to_string()));
    let result = verify_receipt(&receipt);
    assert!(result.valid, "{:#?}", result.checks);

    fs::write(task, "tampered task").unwrap();
    assert!(
        verify_receipt(&receipt).valid,
        "portable structural inspection must not reopen receipt-controlled paths"
    );
}

#[cfg(unix)]
#[test]
fn portable_verify_receipt_never_opens_receipt_controlled_fifo_paths() {
    const CHILD_MARKER: &str = "AGS_EVIDENCE_FIFO_VERIFY_CHILD";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("receipt-controlled.fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        let mut receipt = valid_receipt(None);
        receipt.task_card_path = Some(fifo.display().to_string());
        receipt.launch_plan_path = fifo.display().to_string();
        receipt.delivery_report_path = fifo.display().to_string();
        let _ = verify_receipt(&receipt);
        return;
    }

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("portable_verify_receipt_never_opens_receipt_controlled_fifo_paths")
        .arg("--nocapture")
        .env(CHILD_MARKER, "1")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "child verifier failed: {status}");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("verify_receipt blocked while opening a receipt-controlled FIFO");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn evidence_memory_api_is_pure_and_contains_no_direct_filesystem_bypass() {
    let memory_source = include_str!("memory.rs");
    let receipt_source = include_str!("receipt.rs");
    for forbidden in [
        "std::fs::",
        "ags_platform::atomic_write(",
        "pub fn init(",
        "pub fn archive(",
        "pub fn status(",
    ] {
        assert!(
            !memory_source.contains(forbidden),
            "evidence memory exposes filesystem bypass `{forbidden}`"
        );
    }
    assert!(
        !receipt_source.contains("pub fn verify_receipt_artifacts("),
        "receipt-controlled path artifact verification must not be exported"
    );
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
        operation: Some("govern.task.close".to_string()),
        action_ref_hash: Some("sha256:action".to_string()),
        binding_hash: Some("sha256:binding".to_string()),
        plan_hash: Some("sha256:plan".to_string()),
        payload_hash: Some("sha256:payload".to_string()),
        capability_snapshot_hash: Some("sha256:snapshot".to_string()),
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
fn receipt_json_uses_contract_v2_schema() {
    let receipt = valid_receipt(None);
    let parsed: serde_json::Value = serde_json::from_str(&render_receipt_json(&receipt)).unwrap();
    assert_eq!(
        parsed["schema_version"],
        "ags://schema/contract/v2/task-receipt"
    );
    assert_eq!(parsed["launch_plan_hash"], receipt.launch_plan_hash);
}

#[test]
fn pure_receipt_artifact_verifier_uses_caller_supplied_bytes_only() {
    #[cfg(unix)]
    let fifo = {
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("must-not-open.fifo");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        (directory, fifo)
    };
    let task = b"canonical task";
    let report = b"canonical delivery report";
    let mut plan = serde_json::json!({
        "schema_version": ags_task_contract::runner::SCHEMA_VERSION,
        "task_card_hash": sha256_hex(task),
        "launch_plan_hash": ""
    });
    let plan_hash = ags_task_contract::runner::canonical_launch_plan_hash(&plan).unwrap();
    plan["launch_plan_hash"] = serde_json::Value::String(plan_hash.clone());
    let plan_bytes = serde_json::to_vec_pretty(&plan).unwrap();
    let mut receipt = valid_receipt(None);
    receipt.task_card_hash = sha256_hex(task);
    receipt.launch_plan_hash = plan_hash;
    receipt.delivery_report_hash = sha256_hex(report);
    receipt.receipt_id = receipt_id(&receipt.task_card_hash, &receipt.launch_plan_hash);
    #[cfg(unix)]
    {
        receipt.task_card_path = Some(fifo.1.display().to_string());
        receipt.launch_plan_path = fifo.1.display().to_string();
        receipt.delivery_report_path = fifo.1.display().to_string();
    }
    #[cfg(not(unix))]
    {
        receipt.task_card_path = Some("/must/not-be-opened/task.md".to_string());
        receipt.launch_plan_path = "/must/not-be-opened/launch-plan.json".to_string();
        receipt.delivery_report_path = "/must/not-be-opened/delivery-report.md".to_string();
    }

    let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
    let verified =
        VerifiedClosure::from_bounded_bytes(&receipt_bytes, task, &plan_bytes, report).unwrap();
    assert_eq!(verified.receipt().receipt_id, receipt.receipt_id);
    verify_receipt_artifact_bytes(&receipt, task, &plan_bytes, report).unwrap();
    assert!(verify_receipt_artifact_bytes(&receipt, b"tampered", &plan_bytes, report).is_err());
    assert!(verify_receipt_artifact_bytes(&receipt, task, b"{}", report).is_err());
    assert!(verify_receipt_artifact_bytes(&receipt, task, &plan_bytes, b"tampered").is_err());
}
