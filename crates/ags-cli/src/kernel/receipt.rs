use crate::cli::ReceiptAction;

/// Shared dispatch: `receipt generate`
fn cmd_receipt_generate(
    task_card: &str,
    gate_result: &str,
    gate_reason: Option<&str>,
    verifications: &[String],
    delivery_report: Option<&str>,
    format: &str,
) {
    use std::io::Read;

    // Read task card content
    let display_path = if task_card == "-" {
        "(stdin)".to_string()
    } else {
        task_card.to_string()
    };

    let content = if task_card == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("receipt generate: 读取失败 — {}", e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(task_card) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("receipt generate: cannot read task card — {}", e);
                std::process::exit(1);
            }
        }
    };

    // Compute task card hash
    let task_card_hash = receipt::sha256_hex(content.as_bytes());

    // Parse verification results
    let mut verification_results = Vec::new();
    for v in verifications {
        if let Some((cmd, code_str)) = v.rsplit_once(':') {
            let exit_code: i32 = match code_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    eprintln!(
                        "receipt generate: invalid verification format '{}' — expected CMD:EXIT_CODE",
                        v
                    );
                    std::process::exit(2);
                }
            };
            verification_results.push(receipt::VerificationResult {
                command: cmd.to_string(),
                exit_code,
                output_hash: String::new(), // no real output to hash
            });
        } else {
            eprintln!(
                "receipt generate: invalid verification format '{}' — expected CMD:EXIT_CODE",
                v
            );
            std::process::exit(2);
        }
    }

    // Compute delivery report hash if provided
    let delivery_hash = match delivery_report {
        Some(p) => match receipt::hash_file(std::path::Path::new(p)) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("receipt generate: cannot hash delivery report — {}", e);
                std::process::exit(1);
            }
        },
        None => None,
    };

    // Derive receipt_id from first 12 chars of task card hash
    let receipt_id = format!(
        "receipt-{}",
        &task_card_hash[..12.min(task_card_hash.len())]
    );

    let receipt = receipt::Receipt {
        schema_version: "2.0-m6".to_string(),
        receipt_id,
        timestamp: format!("unix-{}", {
            use std::time::SystemTime;
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        }),
        task_card_hash,
        task_card_path: if task_card == "-" {
            None
        } else {
            Some(display_path)
        },
        gate_result: receipt::GateResult {
            decision: gate_result.to_string(),
            reason: gate_reason.map(|s| s.to_string()),
        },
        verification_results,
        delivery_report_hash: delivery_hash,
        exit_code: None,
    };

    match format {
        "json" => println!("{}", receipt::render_receipt_json(&receipt)),
        _ => {
            // Text format: print JSON because text receipt is just the JSON body
            println!("{}", receipt::render_receipt_json(&receipt));
        }
    }
}
/// Shared dispatch: `receipt verify`
fn cmd_receipt_verify(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("receipt verify: cannot read receipt — {}", e);
            std::process::exit(1);
        }
    };

    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("receipt verify: invalid receipt JSON — {}", e);
            std::process::exit(1);
        }
    };

    let result = receipt::verify_receipt(&receipt);
    match format {
        "json" => println!("{}", receipt::render_verify_json(&result)),
        _ => println!("{}", receipt::render_verify_text(&result)),
    }

    if !result.valid {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: ReceiptAction) {
    match action {
        ReceiptAction::Generate {
            task_card,
            gate_result,
            gate_reason,
            verifications,
            delivery_report,
            format,
        } => cmd_receipt_generate(
            &task_card,
            &gate_result,
            gate_reason.as_deref(),
            &verifications,
            delivery_report.as_deref(),
            &format,
        ),
        ReceiptAction::Verify { path, format } => cmd_receipt_verify(&path, &format),
    }
}
