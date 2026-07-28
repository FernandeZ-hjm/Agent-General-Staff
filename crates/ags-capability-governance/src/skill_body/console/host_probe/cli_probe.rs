use super::*;

/// Cached result of probing one host's MCP registry once per inventory.
pub(in super::super) struct HostMcpProbe {
    /// Whether the host CLI was runnable. False → MCP checks are degraded.
    pub(super) available: bool,
    /// (server name, connected/enabled) pairs parsed from `<host> mcp list`.
    pub(super) servers: Vec<(String, bool)>,
    /// Reader-facing evidence source. OMP currently inherits Codex config, so
    /// its source-config probe must not be presented as a live OMP runtime test.
    pub(super) evidence_source: String,
    /// True only when this probe observed the requested host's own live
    /// registry/runtime surface. OMP's inherited Codex source is deliberately
    /// false: it proves registration availability, not an OMP connection.
    pub(super) live_runtime_probe: bool,
}

impl HostMcpProbe {
    fn unavailable(evidence_source: impl Into<String>) -> Self {
        Self {
            available: false,
            servers: Vec::new(),
            evidence_source: evidence_source.into(),
            live_runtime_probe: false,
        }
    }

    pub(super) fn find(&self, name: &str) -> Option<bool> {
        self.servers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, connected)| *connected)
    }
}

/// Probe a host's registered MCP servers via its CLI. Read-only. Unknown hosts
/// or a missing CLI yield an unavailable probe (→ degraded, never a panic).
pub(in super::super) fn probe_host_mcp(ctx: &ConsoleContext, host: &str) -> HostMcpProbe {
    let Some(probe) =
        ags_host_integration::platform_spec(host).and_then(|platform| platform.mcp_probe)
    else {
        return HostMcpProbe::unavailable(format!("host '{host}' MCP registry"));
    };
    match ctx.runner.run(probe.program, probe.args) {
        CommandOutcome::Unavailable => HostMcpProbe::unavailable(probe.evidence_source),
        // A non-zero exit means we could NOT enumerate the registry — treat it
        // as unavailable (→ degraded), not as an authoritative empty list. A
        // parsed empty/partial stdout on failure would wrongly report MCPs as
        // missing/incomplete.
        CommandOutcome::Ran { success: false, .. } => {
            HostMcpProbe::unavailable(probe.evidence_source)
        }
        CommandOutcome::Ran {
            success: true,
            stdout,
        } => HostMcpProbe {
            available: true,
            servers: ags_host_integration::parse_mcp_list(probe.format, &stdout)
                .into_iter()
                .map(|entry| (entry.name, entry.active))
                .collect(),
            evidence_source: probe.evidence_source.to_string(),
            live_runtime_probe: probe.live_runtime_probe,
        },
    }
}
