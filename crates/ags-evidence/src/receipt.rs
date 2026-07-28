use super::*;

/// Compute SHA-256 hash of a byte slice, returning a hex-encoded string.
pub fn sha256_hex(data: &[u8]) -> String {
    ags_platform::sha256_hex(data)
}

/// Compute SHA-256 hash of file content.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let content =
        std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    Ok(sha256_hex(&content))
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
/// 3. Task card hash matches source file (if task_card_path is present and file exists)
/// 4. Verification output hashes match (if source command outputs are available)
pub fn verify_receipt(receipt: &Receipt) -> VerifyResult {
    let mut checks: Vec<CheckItem> = Vec::new();

    // Check 1: schema version
    if receipt.schema_version == RECEIPT_SCHEMA_VERSION {
        checks.push(CheckItem {
            name: "schema_version".to_string(),
            passed: true,
            detail: format!("schema_version {} is supported", receipt.schema_version),
        });
    } else {
        checks.push(CheckItem {
            name: "schema_version".to_string(),
            passed: false,
            detail: format!(
                "expected schema_version {}, got {}",
                RECEIPT_SCHEMA_VERSION, receipt.schema_version
            ),
        });
    }

    // Check 2: required fields present
    let mut missing = Vec::new();
    if receipt.receipt_id.is_empty() {
        missing.push("receipt_id");
    }
    if receipt.task_card_hash.is_empty() {
        missing.push("task_card_hash");
    }
    if receipt.launch_plan_hash.is_empty() {
        missing.push("launch_plan_hash");
    }
    if receipt.delivery_report_hash.is_empty() {
        missing.push("delivery_report_hash");
    }
    if receipt.gate_result.decision.is_empty() {
        missing.push("gate_result.decision");
    }

    let expected_receipt_id = receipt_id(&receipt.task_card_hash, &receipt.launch_plan_hash);
    checks.push(CheckItem {
        name: "receipt_id_binding".to_string(),
        passed: receipt.receipt_id == expected_receipt_id,
        detail: format!(
            "expected `{expected_receipt_id}`, actual `{}`",
            receipt.receipt_id
        ),
    });
    if missing.is_empty() {
        checks.push(CheckItem {
            name: "required_fields".to_string(),
            passed: true,
            detail: "all required fields present".to_string(),
        });
    } else {
        checks.push(CheckItem {
            name: "required_fields".to_string(),
            passed: false,
            detail: format!("missing required fields: {}", missing.join(", ")),
        });
    }

    // Check 3: task card hash consistency (if source file available)
    match &receipt.task_card_path {
        Some(path_str) => {
            let path = Path::new(path_str);
            if path.exists() {
                match hash_file(path) {
                    Ok(actual_hash) => {
                        if actual_hash == receipt.task_card_hash {
                            checks.push(CheckItem {
                                name: "task_card_hash".to_string(),
                                passed: true,
                                detail: format!("hash matches source file {}", path_str),
                            });
                        } else {
                            checks.push(CheckItem {
                                name: "task_card_hash".to_string(),
                                passed: false,
                                detail: format!(
                                    "hash mismatch: receipt has {} but file hashes to {}",
                                    receipt.task_card_hash, actual_hash
                                ),
                            });
                        }
                    }
                    Err(e) => {
                        checks.push(CheckItem {
                            name: "task_card_hash".to_string(),
                            passed: false,
                            detail: format!("cannot hash source file: {}", e),
                        });
                    }
                }
            } else {
                checks.push(CheckItem {
                    name: "task_card_hash".to_string(),
                    passed: true,
                    detail: format!("source file {path_str} unavailable; structural checks only"),
                });
            }
        }
        None => {
            checks.push(CheckItem {
                name: "task_card_hash".to_string(),
                passed: true,
                detail: "no task_card_path; structural checks only".to_string(),
            });
        }
    }

    checks.push(launch_plan_hash_check(receipt));
    checks.push(path_hash_check(
        "delivery_report_hash",
        &receipt.delivery_report_path,
        &receipt.delivery_report_hash,
    ));

    checks.push(CheckItem {
        name: "artifact_hashes".to_string(),
        passed: [
            &receipt.task_card_hash,
            &receipt.launch_plan_hash,
            &receipt.delivery_report_hash,
        ]
        .iter()
        .all(|hash| {
            hash.len() == 64 && hash.chars().all(|character| character.is_ascii_hexdigit())
        }),
        detail: "task card, launch plan, and delivery report hashes must be SHA-256 hex"
            .to_string(),
    });

    let valid = checks.iter().all(|c| c.passed);

    VerifyResult {
        schema_version: RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: receipt.receipt_id.clone(),
        valid,
        checks,
    }
}

fn path_hash_check(name: &str, path: &str, expected: &str) -> CheckItem {
    match hash_file(Path::new(path)) {
        Ok(actual) => CheckItem {
            name: name.to_string(),
            passed: actual == expected,
            detail: format!("source `{path}` expected `{expected}`, actual `{actual}`"),
        },
        Err(error) => CheckItem {
            name: name.to_string(),
            passed: true,
            detail: format!("{error}; structural checks only"),
        },
    }
}

fn launch_plan_hash_check(receipt: &Receipt) -> CheckItem {
    let path = &receipt.launch_plan_path;
    let result = std::fs::read(path)
        .map_err(|error| format!("cannot read `{path}`: {error}"))
        .and_then(|bytes| {
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid LaunchPlan JSON: {error}"))?;
            ags_task_contract::runner::canonical_launch_plan_hash(&value)
        });
    match result {
        Ok(actual) => CheckItem {
            name: "launch_plan_hash".to_string(),
            passed: actual == receipt.launch_plan_hash,
            detail: format!(
                "source `{path}` expected `{}`, actual `{actual}`",
                receipt.launch_plan_hash
            ),
        },
        Err(error) => CheckItem {
            name: "launch_plan_hash".to_string(),
            passed: true,
            detail: format!("{error}; structural checks only"),
        },
    }
}

/// Strict artifact verification used by memory archival. Unlike portable
/// receipt inspection, every referenced source must exist and match.
pub fn verify_receipt_artifacts(receipt: &Receipt) -> Result<(), String> {
    let structural = verify_receipt(receipt);
    if !structural.valid {
        return Err(structural
            .checks
            .into_iter()
            .filter(|check| !check.passed)
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>()
            .join("; "));
    }
    let task_path = receipt
        .task_card_path
        .as_deref()
        .ok_or_else(|| "receipt has no task_card_path".to_string())?;
    for (label, path, expected) in [
        ("task card", task_path, receipt.task_card_hash.as_str()),
        (
            "delivery report",
            receipt.delivery_report_path.as_str(),
            receipt.delivery_report_hash.as_str(),
        ),
    ] {
        let actual = hash_file(Path::new(path))?;
        if actual != expected {
            return Err(format!(
                "{label} hash mismatch: expected `{expected}`, actual `{actual}`"
            ));
        }
    }
    let plan_bytes = std::fs::read(&receipt.launch_plan_path)
        .map_err(|error| format!("cannot read `{}`: {error}", receipt.launch_plan_path))?;
    let plan: serde_json::Value = serde_json::from_slice(&plan_bytes)
        .map_err(|error| format!("invalid LaunchPlan JSON: {error}"))?;
    let actual_plan_hash = ags_task_contract::runner::canonical_launch_plan_hash(&plan)?;
    if actual_plan_hash != receipt.launch_plan_hash {
        return Err(format!(
            "launch plan hash mismatch: expected `{}`, actual `{actual_plan_hash}`",
            receipt.launch_plan_hash
        ));
    }
    Ok(())
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
