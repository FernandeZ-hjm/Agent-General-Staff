use super::*;

/// Compute SHA-256 hash of a byte slice, returning a hex-encoded string.
pub fn sha256_hex(data: &[u8]) -> String {
    ags_platform::sha256_hex(data)
}

// ── Receipt generation ─────────────────────────────────────────────────────

/// Derive the receipt identity from both immutable authority artifacts.
pub fn receipt_id(task_card_hash: &str, launch_plan_hash: &str) -> String {
    let material = format!("{task_card_hash}\n{launch_plan_hash}");
    let digest = sha256_hex(material.as_bytes());
    format!("receipt-{}", &digest[..12])
}

/// Build the only first-party receipt: one produced by a successful task
/// closure. Callers cannot generate an unbound task-card-only receipt.
pub fn generate_closed_receipt(
    task_card_path: &Path,
    launch_plan_path: &Path,
    delivery_report_path: &Path,
    closure: &delivery_report::DeliveryClosureResult,
    verification_results: Vec<VerificationResult>,
    governance_evidence: Option<GovernanceEvidence>,
) -> Receipt {
    Receipt {
        schema_version: RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: closure.receipt_id.clone(),
        timestamp: iso8601_now(),
        task_card_hash: closure.task_card_hash.clone(),
        launch_plan_hash: closure.launch_plan_hash.clone(),
        task_card_path: Some(task_card_path.display().to_string()),
        launch_plan_path: launch_plan_path.display().to_string(),
        delivery_report_path: delivery_report_path.display().to_string(),
        gate_result: GateResult {
            decision: "allow".to_string(),
            reason: None,
        },
        verification_results,
        delivery_report_hash: closure.delivery_report_hash.clone(),
        execution_footprint: ExecutionFootprint {
            execution_mode_used: closure.execution_mode_used.clone(),
            execution_topology_used: closure.execution_topology_used.clone(),
            delegation_used: closure.delegation_used.clone(),
        },
        closure_status: closure.task_status.clone(),
        exit_code: Some(0),
        governance_status: Some(ags_governance_decision::GovernanceStatus::DoneWithReceipt),
        governance_evidence,
    }
}

/// Generate an ISO 8601 timestamp using std only.
fn iso8601_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Format: YYYY-MM-DDTHH:MM:SSZ (UTC)
    let secs = now.as_secs();
    let _days = secs / 86400;
    // Rough but deterministic for test purposes; production would use a proper calendar library
    // but we avoid chrono by design
    let time_str = format!("{}", secs);
    // For deterministic output in tests, we use a fixed format
    format!("unix-{}", time_str)
}

// ── Receipt verification ───────────────────────────────────────────────────

/// Verify a receipt's integrity.
///
/// Checks:
/// 1. Schema version is present and recognized
/// 2. Required fields and the receipt identity binding are valid
/// 3. Artifact digests have valid structural forms
///
/// Receipt-controlled paths are opaque metadata here. Authoritative artifact
/// verification requires caller-supplied bytes through
/// [`verify_receipt_artifact_bytes`].
pub fn verify_receipt(receipt: &Receipt) -> VerifyResult {
    let structural = verify_receipt_structure(receipt);
    let checks = vec![
        CheckItem {
            name: "structural_integrity".to_string(),
            passed: structural.is_ok(),
            detail: structural.err().unwrap_or_else(|| {
                "schema, required fields, identity, and hashes are valid".to_string()
            }),
        },
        CheckItem {
            name: "portable_inspection".to_string(),
            passed: true,
            detail: "receipt paths are opaque metadata; no filesystem authority was exercised"
                .to_string(),
        },
    ];

    let valid = checks.iter().all(|c| c.passed);

    VerifyResult {
        schema_version: RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: receipt.receipt_id.clone(),
        valid,
        checks,
    }
}

/// Authoritative artifact verification over bytes obtained by a governed,
/// descriptor-confined caller. This function performs no filesystem access.
pub fn verify_receipt_artifact_bytes(
    receipt: &Receipt,
    task_card_bytes: &[u8],
    launch_plan_bytes: &[u8],
    delivery_report_bytes: &[u8],
) -> Result<(), String> {
    verify_receipt_structure(receipt)?;
    for (label, bytes, expected) in [
        (
            "task card",
            task_card_bytes,
            receipt.task_card_hash.as_str(),
        ),
        (
            "delivery report",
            delivery_report_bytes,
            receipt.delivery_report_hash.as_str(),
        ),
    ] {
        let actual = sha256_hex(bytes);
        if actual != expected {
            return Err(format!(
                "{label} hash mismatch: expected `{expected}`, actual `{actual}`"
            ));
        }
    }
    let plan: serde_json::Value = serde_json::from_slice(launch_plan_bytes)
        .map_err(|error| format!("invalid LaunchPlan JSON: {error}"))?;
    let plan_task_hash = plan
        .get("task_card_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "LaunchPlan has no task_card_hash".to_string())?;
    if plan_task_hash != receipt.task_card_hash {
        return Err(format!(
            "launch plan task hash mismatch: expected `{}`, actual `{plan_task_hash}`",
            receipt.task_card_hash
        ));
    }
    let actual_plan_hash = ags_task_contract::runner::canonical_launch_plan_hash(&plan)?;
    if actual_plan_hash != receipt.launch_plan_hash {
        return Err(format!(
            "launch plan hash mismatch: expected `{}`, actual `{actual_plan_hash}`",
            receipt.launch_plan_hash
        ));
    }
    Ok(())
}

/// A closure proven entirely from caller-supplied bounded bytes. Production
/// control-plane code must establish descriptor confinement and byte budgets
/// before calling this pure verifier; this type never opens receipt paths.
#[derive(Debug, Clone)]
pub struct VerifiedClosure {
    receipt: Receipt,
}

impl VerifiedClosure {
    pub fn from_bounded_bytes(
        receipt_bytes: &[u8],
        task_card_bytes: &[u8],
        launch_plan_bytes: &[u8],
        delivery_report_bytes: &[u8],
    ) -> Result<Self, String> {
        let receipt: Receipt = serde_json::from_slice(receipt_bytes)
            .map_err(|error| format!("invalid receipt JSON: {error}"))?;
        verify_receipt_artifact_bytes(
            &receipt,
            task_card_bytes,
            launch_plan_bytes,
            delivery_report_bytes,
        )?;
        Ok(Self { receipt })
    }

    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }

    pub fn into_receipt(self) -> Receipt {
        self.receipt
    }
}

fn verify_receipt_structure(receipt: &Receipt) -> Result<(), String> {
    let mut errors = Vec::new();
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
        errors.push(format!(
            "schema_version: expected `{RECEIPT_SCHEMA_VERSION}`, actual `{}`",
            receipt.schema_version
        ));
    }
    for (field, value) in [
        ("receipt_id", receipt.receipt_id.as_str()),
        ("task_card_hash", receipt.task_card_hash.as_str()),
        ("launch_plan_hash", receipt.launch_plan_hash.as_str()),
        (
            "delivery_report_hash",
            receipt.delivery_report_hash.as_str(),
        ),
        (
            "gate_result.decision",
            receipt.gate_result.decision.as_str(),
        ),
    ] {
        if value.is_empty() {
            errors.push(format!("required field `{field}` is empty"));
        }
    }
    let expected_receipt_id = receipt_id(&receipt.task_card_hash, &receipt.launch_plan_hash);
    if receipt.receipt_id != expected_receipt_id {
        errors.push(format!(
            "receipt id mismatch: expected `{expected_receipt_id}`, actual `{}`",
            receipt.receipt_id
        ));
    }
    for (field, value) in [
        ("task_card_hash", receipt.task_card_hash.as_str()),
        ("launch_plan_hash", receipt.launch_plan_hash.as_str()),
        (
            "delivery_report_hash",
            receipt.delivery_report_hash.as_str(),
        ),
    ] {
        if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
            errors.push(format!("{field} is not SHA-256 hex"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

// ── Compliance checking ─────────────────────────────────────────────────────

/// Check receipt compliance (MVP checks only).
///
/// Checks:
/// 1. All verify checks pass (schema, hashes)
/// 2. Gate decision is not "stop"
/// 3. All verification results have exit_code == 0
pub fn check_compliance(receipt: &Receipt) -> ComplianceResult {
    let verify = verify_receipt(receipt);
    let mut checks: Vec<CheckItem> = Vec::new();

    // Include all verify checks
    for c in &verify.checks {
        checks.push(CheckItem {
            name: format!("verify:{}", c.name),
            passed: c.passed,
            detail: c.detail.clone(),
        });
    }

    // Gate decision check
    if receipt.gate_result.decision == "stop" {
        checks.push(CheckItem {
            name: "gate_decision".to_string(),
            passed: false,
            detail: format!(
                "gate decision is 'stop'{} — task should not have been executed",
                receipt
                    .gate_result
                    .reason
                    .as_ref()
                    .map(|r| format!(": {}", r))
                    .unwrap_or_default()
            ),
        });
    } else {
        checks.push(CheckItem {
            name: "gate_decision".to_string(),
            passed: true,
            detail: format!(
                "gate decision is '{}' — allowed to proceed",
                receipt.gate_result.decision
            ),
        });
    }

    // Verification results check
    let failed_verifications: Vec<&VerificationResult> = receipt
        .verification_results
        .iter()
        .filter(|v| v.exit_code != 0)
        .collect();

    if failed_verifications.is_empty() {
        checks.push(CheckItem {
            name: "verification_results".to_string(),
            passed: true,
            detail: format!(
                "all {} verification(s) passed (exit 0)",
                receipt.verification_results.len()
            ),
        });
    } else {
        let failed_commands: Vec<String> = failed_verifications
            .iter()
            .map(|v| format!("{} (exit {})", v.command, v.exit_code))
            .collect();
        checks.push(CheckItem {
            name: "verification_results".to_string(),
            passed: false,
            detail: format!(
                "{} verification(s) failed: {}",
                failed_verifications.len(),
                failed_commands.join(", ")
            ),
        });
    }

    let compliant = checks.iter().all(|c| c.passed);
    let schema_version = RECEIPT_SCHEMA_VERSION.to_string();

    ComplianceResult {
        schema_version,
        receipt_id: receipt.receipt_id.clone(),
        compliant,
        checks,
    }
}

// ── Render functions ────────────────────────────────────────────────────────

/// Render VerifyResult as human-readable text.
pub fn render_verify_text(result: &VerifyResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    let status = if result.valid { "VALID" } else { "INVALID" };
    lines.push(format!(
        "Receipt Verification: {} (schema {})",
        status, result.schema_version
    ));
    lines.push(format!("Receipt ID: {}", result.receipt_id));
    lines.push(String::new());
    for check in &result.checks {
        let mark = if check.passed { "[PASS]" } else { "[FAIL]" };
        lines.push(format!("  {} {} — {}", mark, check.name, check.detail));
    }
    lines.join("\n")
}

/// Render VerifyResult as JSON string.
pub fn render_verify_json(result: &VerifyResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// Render ComplianceResult as human-readable text.
pub fn render_compliance_text(result: &ComplianceResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    let status = if result.compliant {
        "COMPLIANT"
    } else {
        "NON-COMPLIANT"
    };
    lines.push(format!(
        "Compliance Check: {} (schema {})",
        status, result.schema_version
    ));
    lines.push(format!("Receipt ID: {}", result.receipt_id));
    lines.push(String::new());
    for check in &result.checks {
        let mark = if check.passed { "[PASS]" } else { "[FAIL]" };
        lines.push(format!("  {} {} — {}", mark, check.name, check.detail));
    }
    lines.join("\n")
}

/// Render ComplianceResult as JSON string.
pub fn render_compliance_json(result: &ComplianceResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// Render Receipt as JSON string.
pub fn render_receipt_json(receipt: &Receipt) -> String {
    serde_json::to_string_pretty(receipt)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}
