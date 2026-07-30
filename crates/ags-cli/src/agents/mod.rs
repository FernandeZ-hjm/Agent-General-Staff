//! `ags agents` lifecycle (五段链路第 2 段) — host governance dispatch.

mod govern;
mod scan;
mod verify;

use crate::cli::AgentsAction;

pub(crate) fn run(action: AgentsAction) {
    match action {
        AgentsAction::Scan { format } => scan::cmd_agents_scan(&format),
        AgentsAction::Govern {
            agent,
            target,
            apply,
            format,
        } => govern::cmd_agents_govern(agent.as_deref(), &target, apply, &format),
        AgentsAction::Verify {
            host,
            strict,
            format,
        } => verify::cmd_agents_verify(&host, strict, &format),
    }
}
