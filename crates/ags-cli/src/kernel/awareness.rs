use crate::cli::{AgentAction, ProjectAction, ProtocolAction, SessionAction};
use std::path::Path;

/// Shared dispatch: `project detect`
fn cmd_project_detect(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "project detect: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let identity = ags_workspace_facts::detect_project(target);
    match format {
        "json" => println!("{}", ags_workspace_facts::render_json(&identity)),
        _ => println!(
            "{}",
            ags_workspace_facts::render_project_identity_text(&identity)
        ),
    }
    std::process::exit(ags_workspace_facts::project_detect_exit_code(&identity));
}
/// Shared dispatch: `protocol status`
fn cmd_protocol_status(target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "protocol status: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let status = ags_workspace_facts::check_protocol_status(target);
    match format {
        "json" => println!("{}", ags_workspace_facts::render_json(&status)),
        _ => println!(
            "{}",
            ags_workspace_facts::render_protocol_status_text(&status)
        ),
    }
    std::process::exit(ags_workspace_facts::protocol_status_exit_code(&status));
}
/// Shared dispatch: `agent instructions`
fn cmd_agent_instructions(for_agent: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "agent instructions: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let agent_type = match ags_workspace_facts::AgentType::from_str(for_agent) {
        Ok(at) => at,
        Err(e) => {
            eprintln!("agent instructions: {}", e);
            std::process::exit(2);
        }
    };

    let instructions = ags_workspace_facts::generate_agent_instructions(target, &agent_type);
    match format {
        "json" => println!("{}", ags_workspace_facts::render_json(&instructions)),
        _ => println!(
            "{}",
            ags_workspace_facts::render_agent_instructions_text(&instructions)
        ),
    }
    std::process::exit(instructions.exit_code);
}
/// Shared dispatch: `session preflight`
fn cmd_session_preflight(for_agent: &str, target: &Path, format: &str) {
    if !target.exists() {
        eprintln!(
            "session preflight: target does not exist — {}",
            target.display()
        );
        std::process::exit(1);
    }

    let agent_type = match ags_workspace_facts::AgentType::from_str(for_agent) {
        Ok(at) => at,
        Err(e) => {
            eprintln!("session preflight: {}", e);
            std::process::exit(2);
        }
    };

    let preflight = ags_workspace_facts::run_session_preflight(target, &agent_type);
    match format {
        "json" => println!("{}", ags_workspace_facts::render_json(&preflight)),
        _ => println!(
            "{}",
            ags_workspace_facts::render_session_preflight_text(&preflight)
        ),
    }
    std::process::exit(preflight.exit_code);
}

// ── M5 dispatch functions ─────────────────────────────────────────────────

pub(crate) fn run_project(action: ProjectAction) {
    match action {
        ProjectAction::Detect { target, format } => cmd_project_detect(&target, &format),
    }
}

pub(crate) fn run_protocol(action: ProtocolAction) {
    match action {
        ProtocolAction::Status { target, format } => cmd_protocol_status(&target, &format),
    }
}

pub(crate) fn run_agent(action: AgentAction) {
    match action {
        AgentAction::Instructions {
            for_agent,
            target,
            format,
        } => cmd_agent_instructions(&for_agent, &target, &format),
    }
}

pub(crate) fn run_session(action: SessionAction) {
    match action {
        SessionAction::Preflight {
            for_agent,
            target,
            format,
        } => cmd_session_preflight(&for_agent, &target, &format),
    }
}
