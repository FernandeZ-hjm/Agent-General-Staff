//! `ags setup` lifecycle (五段链路第 1 段).

mod apply;
mod global_entry;
mod memory;
mod plan;
mod recommendations;
pub(crate) mod rollback;
mod templates;
mod verify;

pub(crate) use verify::{cmd_private_verify, private_install_health_report};

use crate::context::{
    guard_writable_target, home_dir, private_install_target, source_root_or_exit, unix_timestamp,
};
use crate::host_platforms::{
    cross_platform_init_json, cross_platform_init_plan, render_cross_platform_init_text,
};
use crate::receipt_bridge::emit_ags_action_receipt;
use crate::setup::apply::{add_claude_registration_checks, write_install_file};
use crate::setup::global_entry::{
    global_entry_protocol_json, global_entry_protocol_plan, render_global_entry_protocol_text,
    write_ags_global_entry,
};
use crate::setup::plan::{
    capability_route_enrollment_json, cleanup_install_dir, private_install_plan,
    render_capability_route_enrollment_text, render_private_plan_json, render_private_plan_text,
};
use crate::setup::recommendations::{
    render_third_party_recommendations_text, third_party_recommendations_json,
};
use std::path::PathBuf;

pub(in crate::setup) const PRIVATE_INSTALL_SCHEMA: &str = "2.4-private-install";
pub(in crate::setup) fn claude_ags_command_path() -> PathBuf {
    home_dir().join(".claude").join("commands").join("ags.md")
}
fn codex_ags_named_skill_dir(name: &str) -> PathBuf {
    home_dir().join(".codex").join("skills").join(name)
}
pub(in crate::setup) fn codex_ags_named_skill_path(name: &str) -> PathBuf {
    codex_ags_named_skill_dir(name).join("SKILL.md")
}
pub(in crate::setup) fn codex_ags_named_skill_agent_metadata_path(name: &str) -> PathBuf {
    codex_ags_named_skill_dir(name)
        .join("agents")
        .join("openai.yaml")
}
pub(in crate::setup) fn retired_codex_ags_skill_dirs() -> Vec<PathBuf> {
    vec![
        codex_ags_named_skill_dir("ags"),
        codex_ags_named_skill_dir("ags-preflight"),
        codex_ags_named_skill_dir("ags-verify"),
        // `ags-capability` retired from the standard front-stage Codex set (2.7):
        // the `ags capability ...` CLI and Cross-Agent sync engine remain, but
        // the visible command skill is removed. Setup cleans the stale host entry.
        codex_ags_named_skill_dir("ags-capability"),
    ]
}
pub(crate) fn cmd_private_plan(
    profile: &str,
    target: Option<PathBuf>,
    format: &str,
    capability_route_mode: Option<capability_route::EnrollmentMode>,
) {
    if profile != "private" {
        eprintln!("ags plan: unsupported profile '{profile}'");
        std::process::exit(2);
    }
    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    // Resolve the EFFECTIVE mode the plan would write: an explicit flag wins;
    // otherwise preserve the machine's existing enrollment (suite-only default if
    // none). This makes the plan render match what apply would actually do.
    let effective_mode = effective_enrollment_mode(capability_route_mode, &target);
    let plan = private_install_plan(&source_root, &target, effective_mode);
    let wizard = cross_platform_init_plan(&home_dir(), &|c| ags_platform::is_on_path(c));
    match format {
        "json" => {
            let mut value: serde_json::Value =
                serde_json::from_str(&render_private_plan_json(&plan))
                    .unwrap_or_else(|_| serde_json::json!({}));
            if let Some(obj) = value.as_object_mut() {
                obj.insert(
                    "capability_route_enrollment".to_string(),
                    capability_route_enrollment_json(effective_mode, &target),
                );
                obj.insert(
                    "cross_platform_init".to_string(),
                    cross_platform_init_json(&wizard),
                );
                obj.insert(
                    "global_entry_protocol".to_string(),
                    global_entry_protocol_json(&global_entry_protocol_plan(&plan)),
                );
                obj.insert(
                    "third_party_recommendations".to_string(),
                    third_party_recommendations_json(&source_root, &home_dir()),
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", render_private_plan_text(&plan));
            println!();
            println!(
                "{}",
                render_capability_route_enrollment_text(effective_mode, &target)
            );
            println!();
            println!("{}", render_cross_platform_init_text(&wizard));
            println!();
            println!(
                "{}",
                render_global_entry_protocol_text(&global_entry_protocol_plan(&plan))
            );
            println!();
            println!(
                "{}",
                render_third_party_recommendations_text(&source_root, &home_dir())
            );
        }
    }
}

// ── Cross-platform initialization wizard ─────────────────────────────────────
//
// `ags setup` detects which Agent platforms are present on this machine and
// renders a cross-platform sync plan: after the primary Agent has the AGS-self
// entry, it shows the planned AGS-self MCP entry, AGS skill thin-index, and
// adopted-capability visibility sync for every detected platform, plus a
// drift-check command. This is PLAN-ONLY: AGS never runs an external host
// registrar/installer here. Actual cross-Agent capability sync lands in the
// `ags capability` command layer (a future release).
/// Core private-install apply WITHOUT exiting the process. Returns the health
/// report, the resolved target, and the plan text. Callers decide output and
/// exit so reusing paths (e.g. `ags update apply`) can still emit their own
/// receipt / JSON after the runtime writes complete.
/// Resolve the effective enrollment mode for an apply. `Some(mode)` is an
/// explicit operator choice; `None` (update / repair re-runs) PRESERVES the
/// machine's recorded enrollment, falling back to the `suite-only` default when
/// no usable evidence exists — never silently disabling routing.
fn effective_enrollment_mode(
    requested: Option<capability_route::EnrollmentMode>,
    target: &std::path::Path,
) -> capability_route::EnrollmentMode {
    if let Some(mode) = requested {
        return mode;
    }
    let existing = capability_route::read_enrollment(target);
    if existing.present {
        existing.mode
    } else {
        capability_route::EnrollmentMode::SuiteOnly
    }
}
pub(crate) fn run_private_apply(
    target: Option<PathBuf>,
    force: bool,
    register_claude: bool,
    capability_route_mode: Option<capability_route::EnrollmentMode>,
) -> (suite_doctor::HealthReport, PathBuf, String) {
    let source_root = source_root_or_exit("ags setup");
    let target = private_install_target(target);
    guard_writable_target("ags setup", &target);
    let mode = effective_enrollment_mode(capability_route_mode, &target);
    let plan = private_install_plan(&source_root, &target, mode);
    let plan_text_before_apply = render_private_plan_text(&plan);
    let backup_stamp = unix_timestamp();
    let mut report = suite_doctor::HealthReport::new("private-install-apply");

    for file in &plan.files {
        report.add(write_install_file(file, force, backup_stamp));
    }
    for dir in &plan.cleanup_dirs {
        report.add(cleanup_install_dir(dir, force, backup_stamp));
    }
    if register_claude {
        add_claude_registration_checks(&mut report);
        memory::add_workspace_memory_capture(&mut report, &home_dir(), &source_root, backup_stamp);
    }
    // Incremental managed-block write of the AGS-owned global entry (under the
    // runtime target — never a host config). Confirm-gated: only the apply path
    // reaches here.
    report.add(write_ags_global_entry(&target));
    (report, target, plan_text_before_apply)
}
pub(crate) fn cmd_private_apply(
    profile: &str,
    target: Option<PathBuf>,
    yes: bool,
    force: bool,
    format: &str,
    register_claude: bool,
    capability_route_mode: Option<capability_route::EnrollmentMode>,
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
        run_private_apply(target, force, register_claude, capability_route_mode);

    match format {
        "json" => {
            let output = serde_json::json!({
                "schema_version": PRIVATE_INSTALL_SCHEMA,
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
            println!("{}", plan_text_before_apply);
            println!();
            println!("{}", suite_doctor::render_text(&report));
        }
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
    capability_route_mode: Option<capability_route::EnrollmentMode>,
) {
    let did_apply = yes && !dry_run;
    let mut apply_code: Option<i32> = None;
    let mut receipt_path: Option<PathBuf> = None;
    if did_apply {
        // Use the NON-exiting apply helper so the setup receipt + next-step are
        // actually reached (cmd_private_apply exits the process). `None` (no flag)
        // PRESERVES the machine's existing enrollment, keeping `ags setup --yes`
        // idempotent — it never downgrades an `adopted`/`review-all` machine to the
        // `suite-only` default. An explicit flag is `Some(..)` and wins.
        let (report, rt_target, plan_text) = run_private_apply(
            target.clone(),
            force,
            register_claude,
            capability_route_mode,
        );
        match format {
            "json" => {
                let output = serde_json::json!({
                    "schema_version": PRIVATE_INSTALL_SCHEMA,
                    "profile": "private",
                    "target": rt_target.to_string_lossy(),
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
                println!("{}", suite_doctor::render_text(&report));
            }
        }
        // Machine-readable receipt evidence — emitted for EVERY setup apply path.
        let passed = report.passed();
        let ar = receipt::build_action_receipt(
            "setup-apply",
            Some(&rt_target.display().to_string()),
            receipt::GateResult {
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
            vec![receipt::VerificationResult {
                command: "ags setup --yes".to_string(),
                exit_code: report.exit_code(),
                output_hash: receipt::sha256_hex(b"setup-applied"),
            }],
            receipt::RollbackPlan::backup_restore(vec![]),
            if passed { "applied" } else { "failed" },
            passed,
        );
        receipt_path = emit_ags_action_receipt(&ar).ok();
        apply_code = Some(report.exit_code());
    }
    if format != "json" {
        let source_root = source_root_or_exit("ags setup");
        println!();
        println!(
            "{}",
            memory::render_memory_capture_plan(&home_dir(), &source_root, register_claude)
        );
    }
    // Always show the Global Entry Protocol Templates gate + wizard.
    cmd_private_plan("private", target, format, capability_route_mode);
    if did_apply && format != "json" {
        if let Some(p) = &receipt_path {
            println!("\n{}", receipt::render_action_receipt_summary_line(p));
        }
        print_setup_agent_governance_next_step();
    }
    if let Some(code) = apply_code {
        std::process::exit(code);
    }
}
/// After `ags setup --yes`, guide the operator to upgrade to machine-wide Agent
/// governance, listing the Agent hosts detected on this machine.
fn print_setup_agent_governance_next_step() {
    let home = home_dir();
    let plan = cross_platform_init_plan(&home, &|c| ags_platform::is_on_path(c));
    let detected: Vec<&str> = plan
        .platforms
        .iter()
        .filter(|p| p.detected)
        .map(|p| p.id.as_str())
        .collect();
    println!();
    println!("Next step — upgrade to machine-wide Agent governance?");
    println!("下一步：是否升级为本机全局 Agent 治理内核？");
    if detected.is_empty() {
        println!("  No Agent hosts detected yet. Install a host CLI (claude / codex), then:");
    } else {
        println!("  Governable Agent hosts detected: {}", detected.join(", "));
    }
    println!("  • `ags agents scan`    inventory hosts + AGS MCP registration");
    println!("  • `ags agents govern`  plan AGS MCP onboarding (advise-only)");
    println!("  • then `ags skill` to govern skills, `ags init` to onboard a project.");
}

#[cfg(test)]
mod enrollment_preserve_tests {
    use super::effective_enrollment_mode;
    use capability_route::{enrollment_file_path, render_enrollment_json, EnrollmentMode};
    use std::path::{Path, PathBuf};

    fn tmp(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("ags-setup-enr-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn write_mode(home: &Path, mode: EnrollmentMode) {
        let p = enrollment_file_path(home);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, render_enrollment_json(mode, "ags setup")).unwrap();
    }

    /// Codex review fix: a routine `ags setup --yes` WITHOUT `--capability-route`
    /// must PRESERVE an existing adopted/review-all enrollment, never downgrade it
    /// to the suite-only default.
    #[test]
    fn omitted_flag_preserves_existing_adopted_and_review_all() {
        let h = tmp("adopted");
        write_mode(&h, EnrollmentMode::Adopted);
        assert_eq!(effective_enrollment_mode(None, &h), EnrollmentMode::Adopted);
        let _ = std::fs::remove_dir_all(&h);

        let h = tmp("review");
        write_mode(&h, EnrollmentMode::ReviewAll);
        assert_eq!(
            effective_enrollment_mode(None, &h),
            EnrollmentMode::ReviewAll
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn omitted_flag_defaults_suite_only_when_no_evidence() {
        let h = tmp("none");
        assert_eq!(
            effective_enrollment_mode(None, &h),
            EnrollmentMode::SuiteOnly
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn explicit_flag_overrides_existing_enrollment() {
        // An explicit choice wins even over an existing broader enrollment.
        let h = tmp("override");
        write_mode(&h, EnrollmentMode::ReviewAll);
        assert_eq!(
            effective_enrollment_mode(Some(EnrollmentMode::Off), &h),
            EnrollmentMode::Off
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    /// Preserve must be byte-idempotent: re-rendering the preserved mode yields the
    /// exact existing file, so a setup refresh reports "unchanged" (no --force, no
    /// downgrade).
    #[test]
    fn preserve_re_renders_identical_bytes() {
        let h = tmp("idem");
        write_mode(&h, EnrollmentMode::Adopted);
        let existing = std::fs::read_to_string(enrollment_file_path(&h)).unwrap();
        let mode = effective_enrollment_mode(None, &h);
        assert_eq!(existing, render_enrollment_json(mode, "ags setup"));
        let _ = std::fs::remove_dir_all(&h);
    }
}
