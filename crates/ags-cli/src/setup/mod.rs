//! Human CLI adapter for the shared runtime setup lifecycle.

use crate::context::{
    guard_writable_target, home_dir, runtime_install_target, source_root_or_exit,
};
use crate::host_platforms::{
    cross_platform_init_json, cross_platform_init_plan, render_cross_platform_init_text,
    AGENT_PLATFORM_SPECS,
};
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
    let detected = detected_lifecycle_hosts();
    resolve_lifecycle_selection_with_detected(target, selection, &detected)
}

fn resolve_lifecycle_selection_with_detected(
    target: &std::path::Path,
    selection: Option<&str>,
    detected: &[String],
) -> Result<Vec<String>, String> {
    let supported = ags_host_integration::lifecycle_specs()
        .map(|spec| spec.host_id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let mut hosts = match selection.map(str::trim) {
        Some("detected") => detected.to_vec(),
        Some("none") => {
            return Err(
                "setup requires at least one Host; `none` is not a valid installation target"
                    .to_string(),
            )
        }
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect(),
        None if target.join("install-manifest.json").is_file() => {
            ags_lifecycle::setup::approved_lifecycle_hosts(target)?
        }
        None => detected.to_vec(),
    };
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() {
        return Err("setup requires at least one detected or explicitly selected Host".to_string());
    }
    if let Some(host) = hosts.iter().find(|host| !supported.contains(*host)) {
        return Err(format!(
            "unsupported lifecycle host `{host}`; supported: {}",
            supported.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(hosts)
}

pub(crate) fn runtime_install_health_report(
    target: &std::path::Path,
    run_mcp_smoke: bool,
) -> ags_verification::doctor::HealthReport {
    ags_lifecycle::setup::runtime_install_health_report(target, &home_dir(), run_mcp_smoke)
}

pub(crate) fn cmd_runtime_plan(
    profile: &str,
    target: Option<PathBuf>,
    required_skill_authority_root: Option<&std::path::Path>,
    format: &str,
) {
    if profile != "runtime" {
        eprintln!("ags plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let source_root = source_root_or_exit("ags setup");
    let target = runtime_install_target(target);
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
    let approved = ags_lifecycle::setup::approved_lifecycle_hosts(&target).unwrap_or_default();
    let detected = detected_lifecycle_hosts();
    let installed = target.join("install-manifest.json").is_file();
    let planned_hosts = if installed {
        approved.clone()
    } else {
        detected.clone()
    };
    let presentation = ags_lifecycle::setup::runtime_plan_presentation(
        &source_root,
        &target,
        &home,
        &host_entries,
        &planned_hosts,
        required_skill_authority_root,
    );
    let wizard = cross_platform_init_plan(&home, &|command| ags_platform::is_on_path(command));
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
        if installed {
            ""
        } else {
            " detected-hosts-selected-on-apply"
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
                "selection_required": planned_hosts.is_empty(),
                "planned_hosts": planned_hosts,
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

/// Core runtime-install apply without exiting. Output and exit policy remain in
/// the human adapter so update/setup callers can preserve their command
/// contracts while sharing one lifecycle mutation authority.
pub(crate) fn run_runtime_apply(
    target: Option<PathBuf>,
    force: bool,
    approved_lifecycle_hosts: Option<&[String]>,
    required_skill_authority_root: Option<&std::path::Path>,
) -> ags_lifecycle::setup::RuntimeApplyResult {
    let source_root = source_root_or_exit("ags setup");
    let target = runtime_install_target(target);
    guard_writable_target("ags setup", &target);
    ags_lifecycle::setup::apply_runtime_with_activation(
        ags_lifecycle::setup::RuntimeApplyRequest {
            source_root: &source_root,
            target: &target,
            home: &home_dir(),
            force,
            approved_lifecycle_hosts,
            suite_skill_authority_root: required_skill_authority_root,
        },
        ags_mcp::workspace_capability_runtime_activator(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_setup(
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    lifecycle_hosts: Option<&str>,
    required_skill_authority_root: Option<PathBuf>,
    recover_plan_hash: Option<&str>,
    dry_run: bool,
    format: &str,
) {
    if let Some(plan_hash) = recover_plan_hash {
        if yes || dry_run || lifecycle_hosts.is_some() || required_skill_authority_root.is_some() {
            eprintln!("ags setup: --recover-plan-hash cannot be combined with setup planning or apply options");
            std::process::exit(2);
        }
        let runtime_target = runtime_install_target(target);
        guard_writable_target("ags setup recovery", &runtime_target);
        let receipt = ags_lifecycle::maintenance::recover_runtime_setup_plan_with_activation(
            &runtime_target,
            plan_hash,
            ags_mcp::workspace_capability_runtime_activator(),
        )
        .unwrap_or_else(|error| {
            eprintln!("ags setup recovery: {error}");
            std::process::exit(1);
        });
        crate::output::emit(format, &receipt, || {
            format!(
                "Recovered runtime setup plan {}\nReceipt: {}",
                receipt.plan_hash, receipt.receipt_id
            )
        });
        return;
    }
    if yes && !dry_run {
        let runtime_target = runtime_install_target(target.clone());
        let approved = resolve_lifecycle_selection(&runtime_target, lifecycle_hosts)
            .unwrap_or_else(|error| {
                eprintln!("ags setup: {error}");
                std::process::exit(2);
            });
        let result = run_runtime_apply(
            target.clone(),
            force,
            Some(&approved),
            required_skill_authority_root.as_deref(),
        );
        let output = serde_json::json!({
            "schema_version": ags_lifecycle::setup::RUNTIME_INSTALL_SCHEMA,
            "profile": "runtime",
            "target": result.target.to_string_lossy(),
            "approved_lifecycle_hosts": approved,
            "force": force,
            "result": result,
        });
        crate::output::emit(format, &output, || {
            format!(
                "{}\n\n{}",
                result.plan_text,
                ags_verification::doctor::render_text(&result.report)
            )
        });
        if !crate::output::is_json(format) {
            print_setup_agent_governance_next_step();
        }
        std::process::exit(result.report.exit_code());
    }

    cmd_runtime_plan(
        "runtime",
        target,
        required_skill_authority_root.as_deref(),
        format,
    );
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
    use super::resolve_lifecycle_selection_with_detected;

    #[test]
    fn lifecycle_selection_defaults_to_detected_then_preserves_and_deduplicates() {
        let runtime = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_lifecycle_selection_with_detected(
                runtime.path(),
                None,
                &["codex".to_string()],
            )
            .unwrap(),
            vec!["codex"]
        );
        assert!(resolve_lifecycle_selection_with_detected(runtime.path(), None, &[]).is_err());
        assert!(resolve_lifecycle_selection_with_detected(
            runtime.path(),
            Some("none"),
            &["codex".to_string()],
        )
        .is_err());
        assert_eq!(
            resolve_lifecycle_selection_with_detected(
                runtime.path(),
                Some("cursor, codex, cursor"),
                &[],
            )
            .unwrap(),
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
            resolve_lifecycle_selection_with_detected(runtime.path(), None, &[]).unwrap(),
            vec!["claude-code", "codex"]
        );
        assert!(resolve_lifecycle_selection_with_detected(
            runtime.path(),
            Some("unknown-host"),
            &[],
        )
        .is_err());
    }
}
