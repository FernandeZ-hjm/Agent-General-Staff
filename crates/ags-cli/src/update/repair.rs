use crate::context::{guard_writable_target, private_install_target};
use crate::receipt_bridge::emit_ags_action_receipt;
use crate::setup::run_private_apply;
use std::path::PathBuf;

pub(in crate::update) fn cmd_update_repair_local(
    target: Option<PathBuf>,
    apply: bool,
    force: bool,
    format: &str,
) {
    let rt_target = private_install_target(target.clone());
    if !apply {
        if format == "json" {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "command": "update repair-local",
                    "mode": "dry-run",
                    "apply_status": "dry-run",
                    "would": ["re-run ags setup --yes (rewrite AGS-owned runtime + thin-index)"],
                    "note": "no git pull, no cargo build; pass --apply to perform.",
                }))
                .unwrap()
            );
        } else {
            println!("AGS Update Repair-Local (dry-run)");
            println!(
                "  would rewrite AGS-owned runtime snippets + thin-index at {}",
                rt_target.display()
            );
            println!("  (no git pull, no cargo build) — run with --apply to perform.");
        }
        return;
    }
    guard_writable_target("ags update repair-local", &rt_target);
    // `None` preserves the machine's recorded Capability Route enrollment
    // (suite-only default if none) — repair never silently changes it.
    let (report, _target, plan_text) = run_private_apply(target.clone(), force, false);
    let passed = report.passed();
    let ar = receipt::build_action_receipt(
        "update-repair-local",
        Some(&rt_target.display().to_string()),
        receipt::GateResult {
            decision: if passed { "allow" } else { "stop" }.to_string(),
            reason: if passed {
                Some("local visibility drift repair".to_string())
            } else {
                Some("local visibility drift repair failed".to_string())
            },
        },
        vec![],
        vec![],
        vec![],
        vec![receipt::VerificationResult {
            command: "ags setup --yes (runtime/thin-index)".to_string(),
            exit_code: report.exit_code(),
            output_hash: receipt::sha256_hex(b"repair-local"),
        }],
        receipt::RollbackPlan::backup_restore(vec![]),
        if passed { "applied" } else { "failed" },
        passed,
    );
    let receipt_path = emit_ags_action_receipt(&ar).ok();
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "command": "update repair-local",
                "apply_status": if passed { "applied" } else { "failed" },
                "applied": passed,
                "target": rt_target.to_string_lossy(),
                "report": report,
                "receipt_ref": receipt_path.as_ref().map(|p| p.display().to_string()),
            }))
            .unwrap()
        );
    } else {
        println!("{plan_text}");
        println!();
        println!("{}", suite_doctor::render_text(&report));
        if let Some(p) = &receipt_path {
            println!("\n{}", receipt::render_action_receipt_summary_line(p));
        }
    }
    if !passed {
        std::process::exit(1);
    }
}
