use crate::cli::ComplianceAction;

/// Shared dispatch: `compliance check`
fn cmd_compliance_check(path: &str, format: &str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("compliance check: cannot read receipt — {}", e);
            std::process::exit(1);
        }
    };

    let receipt: receipt::Receipt = match serde_json::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("compliance check: invalid receipt JSON — {}", e);
            std::process::exit(1);
        }
    };

    let result = receipt::check_compliance(&receipt);
    match format {
        "json" => println!("{}", receipt::render_compliance_json(&result)),
        _ => println!("{}", receipt::render_compliance_text(&result)),
    }

    if !result.compliant {
        std::process::exit(1);
    }
}

// ── Release dispatch ───────────────────────────────────────────────────────

pub(crate) fn run(action: ComplianceAction) {
    match action {
        ComplianceAction::Check { path, format } => cmd_compliance_check(&path, &format),
    }
}
