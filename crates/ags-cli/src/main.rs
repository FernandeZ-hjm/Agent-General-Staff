//! Agent Governance Suite unified CLI — binary entry point.
//!
//! AGS exposes a small human-facing facade:
//!
//! - `ags setup`   Global runtime setup so AGS is visible to host agents.
//! - `ags init`    Project onboarding into AGS governance.
//! - `ags doctor`  Health checks and safe repair suggestions.
//! - `ags skill`   Read-only skill inventory and host visibility verification.
//! - `ags help`    Operator guidance.
//!
//! Kernel operations such as task validation, policy resolution, gates,
//! receipts, compliance, preflight, and release checks remain available to
//! AGS MCP and CI, but are hidden from the human CLI command surface.
//!
//! `main.rs` is a thin entry point: it parses the CLI and routes each
//! top-level `Commands` variant to its owning lifecycle/kernel module. All
//! second-level action dispatch lives inside those modules.

use clap::Parser;

mod cli;
mod context;
mod host_platforms;
mod output;
mod receipt_bridge;

mod agents;
mod capability;
mod doctor;
mod init;
mod kernel;
mod onboarding;
mod setup;
mod skill;
mod update;

use cli::{Cli, Commands};

const CLI_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() {
    let worker = std::thread::Builder::new()
        .name("ags-main".to_string())
        .stack_size(CLI_WORKER_STACK_SIZE)
        .spawn(run_cli)
        .unwrap_or_else(|error| {
            eprintln!("ags: could not start CLI worker: {error}");
            std::process::exit(1);
        });
    if worker.join().is_err() {
        eprintln!("ags: CLI worker terminated unexpectedly");
        std::process::exit(1);
    }
}

fn run_cli() {
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
        Commands::Onboarding { action } => onboarding::run(action),
        Commands::Init {
            target,
            slug,
            dry_run,
            mode,
            format,
        } => init::run(&target, slug, dry_run, &format, &mode),
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
        Commands::Skill { action, format } => skill::run(action, &format),
        Commands::Update { action } => update::run(action),
        Commands::Doctor {
            format,
            fix,
            dry_run,
            target,
        } => doctor::run(&format, fix, dry_run, &target),
        Commands::Capability { action } => capability::run(action),

        // ── Hidden kernel surface ──
        Commands::Task { action } => kernel::task::run(action),
        Commands::Policy { action } => kernel::policy::run(action),
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
        Commands::Memory { action } => kernel::memory::run(action),
        Commands::Host { action } => kernel::host::run(action),
        Commands::Compliance { action } => kernel::compliance::run(action),
        Commands::Session { action } => kernel::awareness::run_session(action),
        Commands::Release { action } => kernel::release::run(action),
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
            public_root,
        } => {
            if let Some(profile) = profile {
                let install_target = if target == *"." {
                    None
                } else {
                    Some(target.clone())
                };
                setup::cmd_private_verify(&profile, install_target, &format);
            }
            kernel::verify::run(action, &scope, &format, &target, public_root.as_deref());
        }
    }
}
