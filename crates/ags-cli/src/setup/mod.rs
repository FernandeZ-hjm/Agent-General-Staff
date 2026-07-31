//! Human CLI adapter for the private-runtime setup lifecycle.

use crate::context::{
    guard_writable_target, home_dir, private_install_target, source_root_or_exit,
};
use crate::host_platforms::{
    cross_platform_init_json, cross_platform_init_plan, render_cross_platform_init_text,
    AGENT_PLATFORM_SPECS,
};
use crate::receipt_bridge::emit_ags_action_receipt;
use std::path::PathBuf;

fn detected_lifecycle_hosts() -> Vec<String> {
    let home = home_dir();
    let mut hosts = cross_platform_init_plan(&home, &|command| ags_platform::is_on_path(command))
        .platforms
        .into_iter()
        .filter(|host| {
            host.detected
                && ags_host_integration::platform_spec(&host.id)
                    .and_then(|spec| spec.lifecycle)
                    .is_some()
        })
        .map(|host| host.id)
        .collect::<Vec<_>>();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn resolve_lifecycle_selection(
    target: &std::path::Path,
    selection: Option<&str>,
) -> Result<Vec<String>, String> {
    let supported = ags_host_integration::lifecycle_specs()
        .map(|spec| spec.host_id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let mut hosts = match selection.map(str::trim) {
        Some("detected") => detected_lifecycle_hosts(),
        Some("none") => Vec::new(),
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect(),
        None if target.join("install-manifest.json").is_file() => {
            ags_lifecycle::setup::approved_lifecycle_hosts(target)?
        }
        None => {
            return Err(
                "first write-mode setup requires --lifecycle-hosts <ids|detected|none>".to_string(),
            )
        }
    };
    hosts.sort();
    hosts.dedup();
    if let Some(host) = hosts.iter().find(|host| !supported.contains(*host)) {
        return Err(format!(
            "unsupported lifecycle host `{host}`; supported: {}",
            supported.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(hosts)
}

pub(crate) fn private_install_health_report(
    target: &std::path::Path,
    include_optional_extensions: bool,
    run_mcp_smoke: bool,
) -> ags_verification::doctor::HealthReport {
    ags_lifecycle::setup::private_install_health_report(
        target,
        &home_dir(),
        include_optional_extensions,
        run_mcp_smoke,
    )
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
    let approved = ags_lifecycle::setup::approved_lifecycle_hosts(&target).unwrap_or_default();
    let detected = detected_lifecycle_hosts();
    let pending = detected
        .iter()
        .filter(|host| !approved.contains(host))
        .cloned()
        .collect::<Vec<_>>();
    let text = format!(
        "{}\nLifecycle approval: approved=[{}] pending=[{}]{}\n\n{}\n\n{}\n\n{}",
        presentation.install_text,
        approved.join(", "),
        pending.join(", "),
        if target.join("install-manifest.json").exists() {
            ""
        } else {
            " selection-required"
        },
        render_cross_platform_init_text(&wizard),
        presentation.global_entry_text,
        presentation.recommendations_text
    );
    let mut value = presentation.install_json;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "cross_platform_init".to_string(),
            cross_platform_init_json(&wizard),
        );
        object.insert(
            "lifecycle_approval".to_string(),
            serde_json::json!({
                "detected_hosts": detected,
                "approved_hosts": approved,
                "pending_hosts": pending,
                "selection_required": !target.join("install-manifest.json").exists(),
            }),
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
    crate::output::emit(format, &value, || text);
}

/// Core private-install apply without exiting. Output and exit policy remain in
/// the human adapter so update/setup callers can preserve their command
/// contracts while sharing one lifecycle mutation authority.
pub(crate) fn run_private_apply(
    target: Option<PathBuf>,
    force: bool,
    include_optional_extensions: bool,
    register_claude: bool,
    approved_lifecycle_hosts: Option<&[String]>,
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
        approved_lifecycle_hosts,
    });
    (result.report, result.target, result.plan_text)
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
        run_private_apply(target, force, false, register_claude, None);
    let output = serde_json::json!({
        "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
        "profile": profile,
        "target": target.to_string_lossy(),
        "register_claude": register_claude,
        "force": force,
        "report": report,
    });
    crate::output::emit(format, &output, || {
        format!(
            "{plan_text_before_apply}\n\n{}",
            ags_verification::doctor::render_text(&report)
        )
    });
    std::process::exit(report.exit_code());
}

pub(crate) fn cmd_private_verify(profile: &str, target: Option<PathBuf>, format: &str) {
    if profile != "private" {
        eprintln!("ags verify: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let target = private_install_target(target);
    let report = private_install_health_report(&target, false, true);
    let output = serde_json::json!({
        "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
        "profile": profile,
        "target": target.to_string_lossy(),
        "report": report,
    });
    crate::output::emit(format, &output, || {
        ags_verification::doctor::render_text(&report)
    });
    std::process::exit(report.exit_code());
}

pub(crate) fn cmd_setup(
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    register_claude: bool,
    lifecycle_hosts: Option<&str>,
    dry_run: bool,
    format: &str,
) {
    let did_apply = yes && !dry_run;
    let mut apply_code: Option<i32> = None;
    let mut receipt_path: Option<PathBuf> = None;
    if did_apply {
        let runtime_target = private_install_target(target.clone());
        let approved = resolve_lifecycle_selection(&runtime_target, lifecycle_hosts)
            .unwrap_or_else(|error| {
                eprintln!("ags setup: {error}");
                std::process::exit(2);
            });
        let (report, runtime_target, plan_text) = run_private_apply(
            target.clone(),
            force,
            false,
            register_claude,
            Some(&approved),
        );
        let output = serde_json::json!({
            "schema_version": ags_lifecycle::setup::PRIVATE_INSTALL_SCHEMA,
            "profile": "private",
            "target": runtime_target.to_string_lossy(),
            "register_claude": register_claude,
            "approved_lifecycle_hosts": approved,
            "force": force,
            "report": report,
        });
        crate::output::emit(format, &output, || {
            format!(
                "{plan_text}\n\n{}",
                ags_verification::doctor::render_text(&report)
            )
        });
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
            if passed { "applied" } else { "failed" },
            passed,
        );
        receipt_path = emit_ags_action_receipt(&receipt).ok();
        apply_code = Some(report.exit_code());
    }
    if !crate::output::is_json(format) {
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
    if did_apply && !crate::output::is_json(format) {
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

#[cfg(test)]
mod tests {
    use super::resolve_lifecycle_selection;

    #[test]
    fn lifecycle_selection_requires_first_choice_then_preserves_and_deduplicates() {
        let runtime = tempfile::tempdir().unwrap();
        assert!(resolve_lifecycle_selection(runtime.path(), None).is_err());
        assert_eq!(
            resolve_lifecycle_selection(runtime.path(), Some("cursor, codex, cursor")).unwrap(),
            vec!["codex", "cursor"]
        );

        std::fs::write(
            runtime.path().join("install-manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "lifecycle": {
                    "approved_hosts": ["claude-code", "codex"],
                    "selection_source": "setup"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_lifecycle_selection(runtime.path(), None).unwrap(),
            vec!["claude-code", "codex"]
        );
        assert!(resolve_lifecycle_selection(runtime.path(), Some("unknown-host")).is_err());
    }
}
