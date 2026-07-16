//! Agent Governance Suite unified CLI — binary entry point.
//!
//! AGS exposes a small human-facing facade:
//!
//! - `ags setup`   Global runtime setup so AGS is visible to host agents.
//! - `ags init`    Project onboarding into AGS governance.
//! - `ags doctor`  Health checks and safe repair suggestions.
//! - `ags skill`   Third-party skill & MCP management console: unified
//!   inventory, host visibility, and confirmation-protected propose/apply.
//! - `ags help`    Operator guidance.
//!
//! Kernel operations such as task validation, policy resolution, gates,
//! receipts, compliance, preflight, and release checks remain available to
//! AGS MCP, CI, and compatibility callers, but are hidden from the human CLI
//! command surface.
//!
//! `main.rs` is a thin entry point: it parses the CLI and routes each
//! top-level `Commands` variant to its owning lifecycle/kernel module. All
//! second-level action dispatch lives inside those modules.

use clap::Parser;

mod cli;
mod context;
mod file_plan;
mod host_platforms;
mod host_probe;
mod managed_projects;
mod output;
mod project_templates;
mod receipt_bridge;

mod agents;
mod capability;
mod doctor;
mod init;
mod kernel;
mod setup;
mod skill;
mod update;

use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // ── Five-segment lifecycle chain ──
        Commands::Setup {
            target,
            yes,
            force,
            register_claude,
            dry_run,
            format,
        } => setup::cmd_setup(target, yes, force, register_claude, dry_run, &format),
        Commands::Init {
            target,
            slug,
            dry_run,
            mode,
            migrate_tracked_overlay,
            format,
        } => init::run(
            &target,
            slug,
            dry_run,
            &format,
            &mode,
            migrate_tracked_overlay,
        ),
        Commands::Plan {
            profile,
            target,
            format,
        } => setup::cmd_private_plan(&profile, target, &format),
        Commands::Apply {
            profile,
            target,
            yes,
            force,
            register_claude,
            format,
        } => setup::cmd_private_apply(&profile, target, yes, force, &format, register_claude),
        Commands::Agents { action } => agents::run(action),
        Commands::Skill {
            action,
            format,
            fix,
        } => skill::run(action, &format, fix),
        Commands::Update { action } => update::run(action),
        Commands::Doctor {
            format,
            fix,
            repair,
            dry_run,
            target,
        } => doctor::run(&format, repair || fix, dry_run, &target),
        Commands::Capability { action } => capability::run(action),

        // ── Hidden kernel surface ──
        Commands::Task { action } => kernel::task::run(action),
        Commands::Policy { action } => kernel::policy::run(action),
        Commands::Sync { action } => kernel::sync::run(action),
        Commands::Bootstrap {
            dry_run,
            apply,
            target,
            format,
        } => kernel::bootstrap::run(dry_run, apply, target, &format),
        Commands::Gate { action } => kernel::gate::run(action),
        Commands::Project { action } => kernel::awareness::run_project(action),
        Commands::Protocol { action } => kernel::awareness::run_protocol(action),
        Commands::Agent { action } => kernel::awareness::run_agent(action),
        Commands::Receipt { action } => kernel::receipt::run(action),
        Commands::Compliance { action } => kernel::compliance::run(action),
        Commands::Session { action } => kernel::awareness::run_session(action),
        Commands::Release { action } => kernel::release::run(action),
        Commands::Rollback { action } => match action {
            cli::RollbackAction::Plan {
                profile,
                target,
                format,
            } => match profile {
                Some(profile) => {
                    setup::rollback::cmd_private_rollback_plan(&profile, target, &format)
                }
                None => kernel::rollback::cmd_rollback_plan(&format),
            },
        },
        Commands::Mcp { action } => kernel::mcp::run(action),
        Commands::Hooks { action } => kernel::hooks::run(action),
        Commands::Run {
            path,
            check_only,
            dry_run,
            approve_writes,
            current_task_approval,
            format,
        } => kernel::runner::run(
            &path,
            check_only,
            dry_run,
            approve_writes,
            current_task_approval,
            &format,
        ),
        Commands::Verify {
            action,
            scope,
            profile,
            format,
            target,
        } => {
            if let Some(profile) = profile {
                let install_target = if target == *"." {
                    None
                } else {
                    Some(target.clone())
                };
                setup::cmd_private_verify(&profile, install_target, &format);
            }
            kernel::verify::run(action, &scope, &format, &target);
        }

        // ── M0 backward-compatible aliases (hidden from help) ──
        Commands::TaskCardValidator { paths } => kernel::task::cmd_task_validate(&paths),
        Commands::ResolvePolicy {
            path,
            format,
            approve_writes,
            current_task_approval,
        } => kernel::policy::cmd_policy_resolve(
            &path,
            &format,
            approve_writes,
            current_task_approval,
        ),
        Commands::WorkflowSyncCheck {
            source,
            targets,
            target,
            target_name,
            allowlist,
            format,
        } => kernel::sync::cmd_sync_check(source, targets, target, target_name, allowlist, &format),
        Commands::SuiteDoctor { format } => {
            doctor::cmd_doctor(&format, false, false, std::path::Path::new("."))
        }
        Commands::BootstrapDryRun { format } => kernel::bootstrap::cmd_bootstrap_dry_run(&format),
    }
}
