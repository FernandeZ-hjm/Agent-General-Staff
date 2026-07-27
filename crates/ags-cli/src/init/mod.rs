//! Thin `ags init` CLI adapter (五段链路第 4 段).

use crate::context::{default_private_runtime_home, unix_timestamp};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::{Path, PathBuf};

fn emit_registration_receipt(
    registration: &ags_lifecycle::init::ManagedProjectRegistration,
) -> Option<PathBuf> {
    let receipt = ags_evidence::build_action_receipt(
        "init-register-project",
        Some(&registration.project_path),
        ags_evidence::GateResult {
            decision: "allow".to_string(),
            reason: Some("ags init managed-project registration".to_string()),
        },
        vec![],
        vec![ags_evidence::ReceiptWrite {
            op: "overwrite".to_string(),
            path: registration.registry_path.display().to_string(),
            from: None,
            detail: format!("managed-projects.yaml upsert ({})", registration.change),
        }],
        vec![],
        vec![],
        "applied",
        true,
    );
    emit_ags_action_receipt(&receipt).ok()
}

pub(crate) fn run(target: &Path, slug: Option<String>, dry_run: bool, format: &str, mode: &str) {
    let now = unix_timestamp();
    let request = ags_lifecycle::init::InitRequest {
        target: target.to_path_buf(),
        runtime_home: default_private_runtime_home(),
        now,
        slug,
        dry_run,
        mode: mode.to_string(),
    };
    match ags_lifecycle::init::execute(request) {
        Ok(mut output) => {
            let receipt = output
                .managed_project_registration()
                .and_then(emit_registration_receipt);
            output.set_managed_project_receipt(receipt);
            if format == "json" {
                println!("{}", output.render_json());
            } else {
                println!("{}", output.render_text());
            }
            if !output.succeeded() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
