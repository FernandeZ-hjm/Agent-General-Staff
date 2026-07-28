//! Thin CLI adapter for lifecycle-owned local repair.

use crate::context::{
    guard_writable_target, home_dir, private_install_target, source_root_or_exit,
};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::PathBuf;

pub(in crate::update) fn cmd_update_repair_local(
    target: Option<PathBuf>,
    apply: bool,
    force: bool,
    format: &str,
) {
    let runtime_target = private_install_target(target.clone());
    if !apply {
        let output = serde_json::json!({
            "command": "update repair-local",
            "mode": "dry-run",
            "apply_status": "dry-run",
            "would": ["re-run ags setup --yes (rewrite AGS-owned runtime + thin-index)"],
            "note": "no git pull, no cargo build; pass --apply to perform.",
        });
        crate::output::emit(format, &output, || {
            format!(
                "AGS Update Repair-Local (dry-run)\n  would rewrite AGS-owned runtime snippets + thin-index at {}\n  (no git pull, no cargo build) — run with --apply to perform.",
                runtime_target.display()
            )
        });
        return;
    }
    guard_writable_target("ags update repair-local", &runtime_target);
    let source_root = source_root_or_exit("ags setup");
    let home = home_dir();
    let outcome = ags_lifecycle::setup::apply_private(ags_lifecycle::setup::PrivateApplyRequest {
        source_root: &source_root,
        target: &runtime_target,
        home: &home,
        force,
        include_optional_extensions: false,
        register_claude: false,
    });
    let passed = outcome.report.passed();
    let exit_code = outcome.report.exit_code();
    let receipt = ags_evidence::build_action_receipt(
        "update-repair-local",
        Some(&runtime_target.display().to_string()),
        ags_evidence::GateResult {
            decision: if passed { "allow" } else { "stop" }.to_string(),
            reason: Some(if passed {
                "local visibility drift repair".to_string()
            } else {
                "local visibility drift repair failed".to_string()
            }),
        },
        vec![],
        vec![],
        vec![],
        vec![ags_evidence::VerificationResult {
            command: "ags setup --yes (runtime/thin-index)".to_string(),
            exit_code,
            output_hash: ags_evidence::sha256_hex(b"repair-local"),
        }],
        if passed { "applied" } else { "failed" },
        passed,
    );
    let receipt_path = emit_ags_action_receipt(&receipt).ok();
    let output = serde_json::json!({
        "command": "update repair-local",
        "apply_status": if passed { "applied" } else { "failed" },
        "applied": passed,
        "target": runtime_target.to_string_lossy(),
        "report": outcome.report,
        "receipt_ref": receipt_path.as_ref().map(|path| path.display().to_string()),
    });
    crate::output::emit(format, &output, || {
        let mut text = format!(
            "{}\n\n{}",
            outcome.plan_text,
            crate::update::render_setup_report(&outcome.report)
        );
        if let Some(path) = &receipt_path {
            text.push_str("\n\n");
            text.push_str(&ags_evidence::render_action_receipt_summary_line(path));
        }
        text
    });
    if !passed {
        std::process::exit(1);
    }
}
