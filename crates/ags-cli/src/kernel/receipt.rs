use crate::cli::ReceiptAction;
/// Shared dispatch: `receipt verify`
fn cmd_receipt_verify(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("receipt verify: cannot read receipt — {}", e);
            std::process::exit(1);
        }
    };

    let receipt: ags_evidence::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("receipt verify: invalid receipt JSON — {}", e);
            std::process::exit(1);
        }
    };

    let result = ags_evidence::verify_receipt(&receipt);
    crate::output::emit_rendered(
        format,
        || ags_evidence::render_verify_json(&result),
        || ags_evidence::render_verify_text(&result),
    );

    if !result.valid {
        std::process::exit(1);
    }
}

pub(crate) fn run(action: ReceiptAction) {
    match action {
        ReceiptAction::Verify { path, format } => cmd_receipt_verify(&path, &format),
    }
}
