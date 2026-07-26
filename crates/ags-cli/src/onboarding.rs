use crate::cli::OnboardingAction as CliOnboardingAction;
use crate::context::{capability_authority_root_or_exit, home_dir};
use crate::receipt_bridge::emit_ags_action_receipt;
use ags_host_integration::{claude_mcp_list_line, codex_mcp_list_line, mcp_server_ids};
use ags_lifecycle::{assess_public_with_resolution, find_action, AssessContext, OnboardingPlan};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub(crate) fn run(action: CliOnboardingAction) {
    match action {
        CliOnboardingAction::Plan {
            target,
            host,
            format,
        }
        | CliOnboardingAction::Status {
            target,
            host,
            format,
        } => {
            let plan = plan_or_exit(&target, &host);
            print_plan(&plan, &format);
        }
        CliOnboardingAction::Apply {
            item,
            plan_hash,
            target,
            host,
            yes,
            format,
        } => {
            if !yes {
                eprintln!(
                    "ags onboarding apply: refused — --yes is required for the single selected item"
                );
                std::process::exit(2);
            }
            let plan = plan_or_exit(&target, &host);
            if plan_hash != plan.plan_hash {
                eprintln!(
                    "ags onboarding apply: refused — plan hash changed (reviewed {plan_hash}, current {}); run `ags onboarding plan` again",
                    plan.plan_hash
                );
                std::process::exit(2);
            }
            let action = find_action(&plan, &item).unwrap_or_else(|error| {
                eprintln!("ags onboarding apply: refused — {error}");
                std::process::exit(2);
            });
            let executable = current_ags().unwrap_or_else(|error| {
                eprintln!("ags onboarding apply: {error}");
                std::process::exit(1);
            });
            let result =
                ags_lifecycle::execute_action(action, &executable).unwrap_or_else(|error| {
                    eprintln!("ags onboarding apply: {error}");
                    std::process::exit(1);
                });
            let success = result.success;
            let receipt = ags_evidence::build_action_receipt(
                "onboarding-apply",
                Some(&plan.target),
                ags_evidence::GateResult {
                    decision: if success { "allow" } else { "stop" }.to_string(),
                    reason: (!success).then(|| format!("onboarding item {item} failed")),
                },
                vec![],
                vec![],
                vec![],
                vec![ags_evidence::VerificationResult {
                    command: format!("ags onboarding apply --item {item}"),
                    exit_code: result.exit_code.unwrap_or(1),
                    output_hash: ags_evidence::sha256_hex(
                        format!("{}\n{}", result.stdout, result.stderr).as_bytes(),
                    ),
                }],
                onboarding_rollback_plan(action),
                if success { "applied" } else { "failed" },
                success,
            );
            let receipt_path = emit_ags_action_receipt(&receipt)
                .ok()
                .map(|path| path.display().to_string());
            let output = ApplyOutput {
                schema_version: "0.3.0-onboarding-apply",
                plan_hash: &plan.plan_hash,
                item_id: &item,
                success,
                exit_code: result.exit_code,
                stdout: result.stdout,
                stderr: result.stderr,
                receipt_path,
                requires_repreflight: true,
            };
            print_serialized(&output, &format);
            if !success {
                std::process::exit(result.exit_code.unwrap_or(1));
            }
        }
        CliOnboardingAction::Verify {
            target,
            host,
            format,
        } => {
            let plan = plan_or_exit(&target, &host);
            print_plan(&plan, &format);
            if !plan.ready {
                std::process::exit(1);
            }
        }
    }
}

fn onboarding_rollback_plan(
    action: &ags_lifecycle::OnboardingAction,
) -> ags_evidence::RollbackPlan {
    let steps = ags_lifecycle::rollback_advice(action)
        .into_iter()
        .map(|advice| ags_evidence::RollbackStep {
            affected_path: advice.affected_path,
            inverse_op: "manual-confirm".to_string(),
            backup_path: None,
            inverse_command: advice.inverse_command,
            detail: advice.detail,
        })
        .collect();
    ags_evidence::RollbackPlan::manual_confirm(steps)
}

fn plan_or_exit(target: &Path, host: &str) -> OnboardingPlan {
    let source_root = std::env::current_dir()
        .ok()
        .filter(|root| root.join("manifests/onboarding-public.yaml").is_file())
        .unwrap_or_else(|| capability_authority_root_or_exit("ags onboarding"));
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ags"));
    let registered_mcp_ids = mcp_server_ids(host).unwrap_or_default();
    let host_home = home_dir();
    let third_party =
        ags_capability_governance::third_party_manifest::resolve_third_party_manifest(&source_root)
            .unwrap_or_else(|error| {
                eprintln!("ags onboarding: {error}");
                std::process::exit(1);
            });
    let snapshot = ags_capability_governance::build_capability_snapshot_with_roots_and_manifest(
        &source_root,
        host,
        &ags_capability_governance::locate_runtime_home(),
        &host_home,
        &third_party,
    )
    .unwrap_or_else(|error| {
        eprintln!("ags onboarding: capability snapshot build failed: {error:?}");
        std::process::exit(1);
    });
    let active_skill_ids = snapshot
        .active_skills
        .iter()
        .map(|skill| skill.skill_id.clone())
        .collect::<Vec<_>>();
    assess_public_with_resolution(
        &AssessContext {
            source_root: &source_root,
            home: &host_home,
            target,
            host,
            ags_executable: &executable,
            mcp_connected: false,
            host_registered: probe_ags_registration(host),
            registered_mcp_ids: &registered_mcp_ids,
            active_skill_ids: &active_skill_ids,
        },
        &third_party,
    )
    .unwrap_or_else(|error| {
        eprintln!("ags onboarding: {error}");
        std::process::exit(1);
    })
}

fn probe_ags_registration(host: &str) -> Option<bool> {
    match host {
        "claude-code" => claude_mcp_list_line("ags")
            .ok()
            .map(|entry| entry.is_some()),
        "codex" => codex_mcp_list_line("ags").ok().map(|entry| entry.is_some()),
        _ => None,
    }
}

fn current_ags() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("cannot resolve AGS executable: {error}"))
}

fn print_plan(plan: &OnboardingPlan, format: &str) {
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(plan).unwrap_or_default());
        return;
    }
    println!("AGS Public Onboarding {}", plan.schema_version);
    println!("Host: {}", plan.host);
    println!("Target: {}", plan.target);
    println!("Plan hash: {}", plan.plan_hash);
    println!(
        "Capability registry: {} ({}, {})",
        plan.manifest_source, plan.manifest_freshness, plan.manifest_hash
    );
    if let Some(reason) = &plan.manifest_fallback_reason {
        println!("Registry fallback: {reason}");
    }
    println!(
        "State: {}",
        if plan.ready {
            "active-ready"
        } else {
            "bootstrap-required"
        }
    );
    for item in &plan.items {
        println!(
            "  - {:<36} {:<26} {}{}",
            item.id,
            format!("{:?}", item.state).to_ascii_lowercase(),
            item.reason,
            if item.action.is_some() {
                " [explicit apply available]"
            } else {
                ""
            }
        );
    }
    println!("Excluded: {}", plan.excluded_capabilities.join(", "));
}

fn print_serialized<T: Serialize>(value: &T, _format: &str) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

#[derive(Serialize)]
struct ApplyOutput<'a> {
    schema_version: &'static str,
    plan_hash: &'a str,
    item_id: &'a str,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    receipt_path: Option<String>,
    requires_repreflight: bool,
}
