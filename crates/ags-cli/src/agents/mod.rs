//! `ags agents` lifecycle (五段链路第 2 段) — host governance dispatch.

mod govern;
mod host_specs;
mod scan;
mod verify;

use crate::cli::AgentsAction;

pub(crate) fn run(action: AgentsAction) {
    match action {
        AgentsAction::Scan { format } => scan::cmd_agents_scan(&format),
        AgentsAction::Govern {
            agent,
            apply,
            format,
        } => govern::cmd_agents_govern(agent.as_deref(), apply, &format),
        AgentsAction::Verify {
            host,
            strict,
            format,
        } => verify::cmd_agents_verify(&host, strict, &format),
    }
}
