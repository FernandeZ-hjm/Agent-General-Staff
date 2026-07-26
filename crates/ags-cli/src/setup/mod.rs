//! Human CLI adapter for the private-runtime setup lifecycle.

pub(crate) mod memory;
pub(crate) mod rollback;

use crate::context::{
    guard_writable_target, home_dir, private_install_target, source_root_or_exit, unix_timestamp,
};
use crate::host_platforms::{
    cross_platform_init_json, cross_platform_init_plan, render_cross_platform_init_text,
    AGENT_PLATFORM_SPECS,
};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::PathBuf;

pub(crate) fn private_install_health_report(
    target: &std::path::Path,
    include_optional_extensions: bool,
) -> ags_verification::doctor::HealthReport {
    into_health_report(ags_lifecycle::setup::private_install_health_report(
        target,
        &home_dir(),
        include_optional_extensions,
    ))
}

fn into_health_report(
    report: ags_lifecycle::setup::SetupReport,
) -> ags_verification::doctor::HealthReport {
    let mut converted = ags_verification::doctor::HealthReport::new(report.title.clone());
    append_setup_report(&mut converted, report);
    converted
}

pub(in crate::setup) fn append_setup_report(
    target: &mut ags_verification::doctor::HealthReport,
    report: ags_lifecycle::setup::SetupReport,
) {
    for finding in report.findings {
        target.add(ags_verification::doctor::Finding {
            check_name: finding.check_name,
            status: match finding.status {
                ags_lifecycle::setup::SetupCheckStatus::Pass => {
                    ags_verification::doctor::CheckStatus::Pass
                }
                ags_lifecycle::setup::SetupCheckStatus::Fail => {
                    ags_verification::doctor::CheckStatus::Fail
                }
                ags_lifecycle::setup::SetupCheckStatus::Warn => {
                    ags_verification::doctor::CheckStatus::Warn
                }
                ags_lifecycle::setup::SetupCheckStatus::Skip => {
                    ags_verification::doctor::CheckStatus::Skip
                }
            },
            severity: match finding.severity {
                ags_lifecycle::setup::SetupSeverity::Info => {
                    ags_verification::doctor::Severity::Info
                }
                ags_lifecycle::setup::SetupSeverity::Warn => {
                    ags_verification::doctor::Severity::Warn
                }
                ags_lifecycle::setup::SetupSeverity::Fail => {
                    ags_verification::doctor::Severity::Fail
                }
            },
            message: finding.message,
            detail: finding.detail,
        });
    }
}

pub(crate) fn cmd_private_plan(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    let home = home_dir();
    let host_entries = AGENT_PLATFORM_SPECS
        .iter()
        .map(|host| ags_lifecycle::setup::SetupHostEntry {
            id: host.id.to_string(),
            display: host.display.to_string(),
            config_subdirs: host
                .config_subdirs
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
        })
        .collect::<Vec<_>>();
    let presentation = ags_lifecycle::setup::private_plan_presentation(
        &source_root,
        &target,
        &home,
        &host_entries,
        false,
    );
    let wizard = cross_platform_init_plan(&home, &|command| ags_platform::is_on_path(command));
    match format {
        "json" => {
            let mut value = presentation.install_json;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "cross_platform_init".to_string(),
                    cross_platform_init_json(&wizard),
                );
                object.insert(
                    "global_entry_protocol".to_string(),
                    presentation.global_entry_json,
                );
                object.insert(
                    "third_party_recommendations".to_string(),
                    presentation.recommendations_json,
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", presentation.install_text);
            println!();
            println!("{}", render_cross_platform_init_text(&wizard));
            println!();
            println!("{}", presentation.global_entry_text);
            println!();
            println!("{}", presentation.recommendations_text);
        }
    }
}

/// Core private-install apply without exiting. Output and exit policy remain in
/// the human adapter so update/setup callers can preserve their command
/// contracts while sharing one lifecycle mutation authority.
pub(crate) fn run_private_apply(
    target: Option<PathBuf>,
    force: bool,
    include_optional_extensions: bool,
    register_claude: bool,
) -> (ags_verification::doctor::HealthReport, PathBuf, String) {
    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    guard_writable_target("ags setup", &target);
    let result = ags_lifecycle::setup::apply_private(ags_lifecycle::setup::PrivateApplyRequest {
        source_root: &source_root,
        target: &target,
        home: &home_dir(),
        force,
        include_optional_extensions,
        register_claude,
        backup_stamp: unix_timestamp(),
    });
    (
        into_health_report(result.report),
        result.target,
        result.plan_text,
    )
}

pub(crate) fn cmd_private_apply(
    profile: &str,
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    format: &str,
    register_claude: bool,
) {
    if profile != "private" {
        eprintln!("ags apply: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    if !yes {
        eprintln!("ags setup: --yes is required for write mode.");
        eprintln!("Review `ags setup` first.");
        std::process::exit(2);
    }

    let (report, target, plan_text_before_apply) =
        run_private_apply(target, force, false, register_claude);
    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
                "profile": profile,
                "target": target.to_string_lossy(),
                "register_claude": register_claude,
                "force": force,
                "report": report,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => {
            println!("{plan_text_before_apply}");
            println!();
            println!("{}", ags_verification::doctor::render_text(&report));
        }
    }
    std::process::exit(report.exit_code());
}

pub(crate) fn cmd_private_verify(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags verify: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let report = private_install_health_report(&target, false);
    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
                "profile": profile,
                "target": target.to_string_lossy(),
                "report": report,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).unwrap_or_default()
            );
        }
        _ => println!("{}", ags_verification::doctor::render_text(&report)),
    }
    std::process::exit(report.exit_code());
}

pub(crate) fn cmd_setup(
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    register_claude: bool,
    dry_run: bool,
    format: &str,
) {
    let did_apply = yes && !dry_run;
    let mut apply_code: Option<i32> = None;
    let mut receipt_path: Option<PathBuf> = None;
    if did_apply {
        let (report, runtime_target, plan_text) =
            run_private_apply(target.clone(), force, false, register_claude);
        match format {
            "json" => {
                let output = serde_json::json!({
                    "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
                    "profile": "private",
                    "target": runtime_target.to_string_lossy(),
                    "register_claude": register_claude,
                    "force": force,
                    "report": report,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).unwrap_or_default()
                );
            }
            _ => {
                println!("{plan_text}");
                println!();
                println!("{}", ags_verification::doctor::render_text(&report));
            }
        }
        let passed = report.passed();
        let receipt = ags_evidence::build_action_receipt(
            "setup-apply",
            Some(&runtime_target.display().to_string()),
            ags_evidence::GateResult {
                decision: if passed { "allow" } else { "stop" }.to_string(),
                reason: if passed {
                    None
                } else {
                    Some("setup apply had failures".to_string())
                },
            },
            vec![],
            vec![],
            vec![],
            vec![ags_evidence::VerificationResult {
                command: "ags setup --yes".to_string(),
                exit_code: report.exit_code(),
                output_hash: ags_evidence::sha256_hex(b"setup-applied"),
            }],
            ags_evidence::RollbackPlan::backup_restore(vec![]),
            if passed { "applied" } else { "failed" },
            passed,
        );
        receipt_path = emit_ags_action_receipt(&receipt).ok();
        apply_code = Some(report.exit_code());
    }
    if format != "json" {
        let source_root = source_root_or_exit("ags setup");
        println!();
        println!(
            "{}",
            ags_lifecycle::setup::render_memory_capture_plan(
                &home_dir(),
                &source_root,
                register_claude,
            )
        );
    }
    cmd_private_plan("private", target, format);
    if did_apply && format != "json" {
        if let Some(path) = &receipt_path {
            println!(
                "\n{}",
                ags_evidence::render_action_receipt_summary_line(path)
            );
        }
        print_setup_agent_governance_next_step();
    }
    if let Some(code) = apply_code {
        std::process::exit(code);
    }
}

fn print_setup_agent_governance_next_step() {
    let plan = cross_platform_init_plan(&home_dir(), &|command| ags_platform::is_on_path(command));
    let detected: Vec<&str> = plan
        .platforms
        .iter()
        .filter(|platform| platform.detected)
        .map(|platform| platform.id.as_str())
        .collect();
    println!();
    println!("Next step — upgrade to machine-wide Agent governance?");
    println!("下一步：是否升级为本机全局 Agent 治理内核？");
    if detected.is_empty() {
        println!("  No Agent hosts detected yet. Install a host CLI (claude / codex / omp), then:");
    } else {
        println!("  Governable Agent hosts detected: {}", detected.join(", "));
    }
    println!("  • `ags agents scan`    inventory hosts + AGS MCP registration");
    println!("  • `ags agents govern`  preview onboarding; add `--agent <host> --apply` for AGS-owned memory wiring");
    println!("  • then `ags skill` to govern skills, `ags init` to onboard a project.");
}
