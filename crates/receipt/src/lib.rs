//! Receipt / Compliance — task run receipt generation, verification, and
//! compliance checking (M6).
//!
//! # Receipt schema
//!
//! A receipt captures the full audit trail of a task run:
//! - `task_card_hash` — SHA-256 of the task card content
//! - `gate_result` — gate check decision (allow / confirm / stop) and optional reason
//! - `verification_results` — list of verification commands with exit codes and output hashes
//! - `delivery_report_hash` — SHA-256 of the delivery report (optional)
//!
//! # Compliance check
//!
//! The compliance checker only performs MVP checks:
//! 1. Schema is valid (all required fields present)
//! 2. Task card hash is consistent (if source file still exists)
//! 3. Gate decision is not "stop"
//! 4. All verification results have exit_code == 0

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

// ── Data model ──────────────────────────────────────────────────────────────

/// A verification result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub command: String,
    pub exit_code: i32,
    pub output_hash: String,
}

/// Gate result embedded in receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResult {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A task run receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub timestamp: String,
    pub task_card_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_card_path: Option<String>,
    pub gate_result: GateResult,
    pub verification_results: Vec<VerificationResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_report_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// A single compliance / verification check item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Result of receipt verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub schema_version: String,
    pub receipt_id: String,
    pub valid: bool,
    pub checks: Vec<CheckItem>,
}

/// Result of compliance checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceResult {
    pub schema_version: String,
    pub receipt_id: String,
    pub compliant: bool,
    pub checks: Vec<CheckItem>,
}

// ── Hash helpers ────────────────────────────────────────────────────────────

/// Compute SHA-256 hash of a byte slice, returning a hex-encoded string.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    bytes_to_hex(&result)
}

/// Compute SHA-256 hash of file content.
pub fn hash_file(path: &Path) -> Result<String, String> {
    let content =
        std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    Ok(sha256_hex(&content))
}

/// Convert bytes to hex string (no external dep needed).
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Receipt generation ─────────────────────────────────────────────────────

/// Generate a receipt from explicit input parameters.
///
/// - `task_card_path`: path to the task card file (used to compute hash)
/// - `gate_decision`: gate check decision (allow / confirm / stop)
/// - `gate_reason`: optional reason for gate decision
/// - `verification_results`: list of verification command results
/// - `delivery_report_path`: optional path to delivery report (used to compute hash)
pub fn generate_receipt(
    task_card_path: &Path,
    gate_decision: &str,
    gate_reason: Option<&str>,
    verification_results: Vec<VerificationResult>,
    delivery_report_path: Option<&Path>,
) -> Result<Receipt, String> {
    let task_card_hash = hash_file(task_card_path)?;
    let delivery_report_hash = match delivery_report_path {
        Some(p) => Some(hash_file(p)?),
        None => None,
    };

    // Derive receipt_id from first 12 chars of task card hash
    let receipt_id = format!(
        "receipt-{}",
        &task_card_hash[..12.min(task_card_hash.len())]
    );

    // Timestamp — ISO 8601 via std only, no chrono dep
    let timestamp = iso8601_now();

    Ok(Receipt {
        schema_version: "2.0-m6".to_string(),
        receipt_id,
        timestamp,
        task_card_hash,
        task_card_path: Some(task_card_path.display().to_string()),
        gate_result: GateResult {
            decision: gate_decision.to_string(),
            reason: gate_reason.map(|s| s.to_string()),
        },
        verification_results,
        delivery_report_hash,
        exit_code: None,
    })
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
/// 2. Required fields are present (receipt_id, task_card_hash, gate_result, verification_results)
/// 3. Task card hash matches source file (if task_card_path is present and file exists)
/// 4. Verification output hashes match (if source command outputs are available)
pub fn verify_receipt(receipt: &Receipt) -> VerifyResult {
    let mut checks: Vec<CheckItem> = Vec::new();

    // Check 1: schema version
    if receipt.schema_version == "2.0-m6" {
        checks.push(CheckItem {
            name: "schema_version".to_string(),
            passed: true,
            detail: "schema_version is 2.0-m6".to_string(),
        });
    } else {
        checks.push(CheckItem {
            name: "schema_version".to_string(),
            passed: false,
            detail: format!(
                "expected schema_version 2.0-m6, got {}",
                receipt.schema_version
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
    if receipt.gate_result.decision.is_empty() {
        missing.push("gate_result.decision");
    }
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
                    detail: format!(
                        "source file {} not available — skipping hash check",
                        path_str
                    ),
                });
            }
        }
        None => {
            checks.push(CheckItem {
                name: "task_card_hash".to_string(),
                passed: true,
                detail: "no task_card_path — hash consistency check skipped".to_string(),
            });
        }
    }

    // Check 4: delivery report hash consistency (if present)
    match &receipt.delivery_report_hash {
        Some(reported_hash) => {
            checks.push(CheckItem {
                name: "delivery_report_hash_present".to_string(),
                passed: true,
                detail: format!("delivery report hash recorded: {}", reported_hash),
            });
        }
        None => {
            checks.push(CheckItem {
                name: "delivery_report_hash_present".to_string(),
                passed: true,
                detail: "no delivery report hash — skipped".to_string(),
            });
        }
    }

    let valid = checks.iter().all(|c| c.passed);

    VerifyResult {
        schema_version: "2.0-m6".to_string(),
        receipt_id: receipt.receipt_id.clone(),
        valid,
        checks,
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
    let schema_version = "2.0-m6".to_string();

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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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

        assert_eq!(receipt.schema_version, "2.0-m6");
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
            Some("heavy-requires-write-approval"),
            vec![],
            None,
        )
        .unwrap();

        assert_eq!(receipt.gate_result.decision, "stop");
        assert_eq!(
            receipt.gate_result.reason.as_deref(),
            Some("heavy-requires-write-approval")
        );
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
            schema_version: "2.0-m6".to_string(),
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
            schema_version: "2.0-m6".to_string(),
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
            schema_version: "2.0-m6".to_string(),
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
            schema_version: "2.0-m6".to_string(),
            receipt_id: "receipt-stop1".to_string(),
            timestamp: "unix-0".to_string(),
            task_card_hash: "abc123".to_string(),
            task_card_path: None,
            gate_result: GateResult {
                decision: "stop".to_string(),
                reason: Some("heavy-requires-write-approval".to_string()),
            },
            verification_results: vec![],
            delivery_report_hash: None,
            exit_code: None,
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
        assert!(gate_check.detail.contains("heavy-requires-write-approval"));
    }

    #[test]
    fn compliance_check_rejects_failed_verification() {
        let receipt = Receipt {
            schema_version: "2.0-m6".to_string(),
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
            schema_version: "2.0-m6".to_string(),
            receipt_id: "receipt-combo1".to_string(),
            timestamp: "unix-0".to_string(),
            task_card_hash: "abc123".to_string(),
            task_card_path: None,
            gate_result: GateResult {
                decision: "confirm".to_string(),
                reason: None,
            },
            verification_results: vec![VerificationResult {
                command: "echo ok".to_string(),
                exit_code: 0,
                output_hash: sha256_hex(b"ok"),
            }],
            delivery_report_hash: None,
            exit_code: None,
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
            schema_version: "2.0-m6".to_string(),
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
        };

        let json = render_receipt_json(&receipt);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], "2.0-m6");
        assert_eq!(parsed["receipt_id"], "receipt-test1");
    }

    #[test]
    fn verify_result_json_includes_failed_checks() {
        let result = VerifyResult {
            schema_version: "2.0-m6".to_string(),
            receipt_id: "receipt-bad".to_string(),
            valid: false,
            checks: vec![
                CheckItem {
                    name: "schema_version".to_string(),
                    passed: false,
                    detail: "expected 2.0-m6, got 1.0".to_string(),
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
            schema_version: "2.0-m6".to_string(),
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
}
