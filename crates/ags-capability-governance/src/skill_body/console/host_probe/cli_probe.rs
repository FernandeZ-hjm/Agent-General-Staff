use super::*;

/// Cached result of probing one host's MCP protocol once per inventory.
pub(in super::super) type HostMcpProbe = ags_host_integration::HostMcpReport;

/// Probe a host's registered MCP servers via its CLI. Read-only. Unknown hosts
/// or a missing CLI yield an unavailable probe (→ degraded, never a panic).
pub(in super::super) fn probe_host_mcp(ctx: &ConsoleContext, host: &str) -> HostMcpProbe {
    ags_host_integration::HostAdapter::new(ctx.runner.as_ref()).inspect_mcp(host)
}
