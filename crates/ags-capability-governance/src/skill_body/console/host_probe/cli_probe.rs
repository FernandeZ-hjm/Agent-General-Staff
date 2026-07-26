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
    let (program, args, evidence_source): (&str, &[&str], &str) = match host {
        "claude-code" => ("claude", &["mcp", "list"], "`claude mcp list`"),
        "codex" => ("codex", &["mcp", "list"], "`codex mcp list`"),
        "omp" => (
            "codex",
            &["mcp", "list"],
            "inherited Codex registration source (`codex mcp list`); live OMP runtime probe NOT_RUN",
        ),
        _ => return HostMcpProbe::unavailable(format!("host '{host}' MCP registry")),
    };
    match ctx.runner.run(program, args) {
        CommandOutcome::Unavailable => HostMcpProbe::unavailable(evidence_source),
        // A non-zero exit means we could NOT enumerate the registry — treat it
        // as unavailable (→ degraded), not as an authoritative empty list. A
        // parsed empty/partial stdout on failure would wrongly report MCPs as
        // missing/incomplete.
        CommandOutcome::Ran { success: false, .. } => HostMcpProbe::unavailable(evidence_source),
        CommandOutcome::Ran {
            success: true,
            stdout,
        } => HostMcpProbe {
            available: true,
            servers: if matches!(host, "codex" | "omp") {
                parse_codex_mcp_list(&stdout)
            } else {
                parse_claude_mcp_list(&stdout)
            },
            evidence_source: evidence_source.to_string(),
            live_runtime_probe: host != "omp",
        },
    }
}

/// Parse `claude mcp list` output. Lines look like
/// `name: /path/to/cmd args - ✔ Connected`. Plugin-owned MCP names may contain
/// colons themselves, e.g. `plugin:claude-mem:mcp-search: node ...`, so split
/// on the first `: ` delimiter instead of the first raw colon.
pub(in super::super) fn parse_claude_mcp_list(stdout: &str) -> Vec<(String, bool)> {
    let mut servers = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, rest)) = line.split_once(": ") else {
            continue;
        };
        let name = name.trim();
        // Server names are single tokens; skip prose/header lines.
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            continue;
        }
        let connected = rest.contains("Connected") || rest.contains('✔') || rest.contains('✓');
        servers.push((name.to_string(), connected));
    }
    servers
}

/// Parse `codex mcp list` output — a whitespace-padded table with columns
/// `Name Command Args Env Cwd Status Auth`. Lenient: the first token of each
/// non-header row is the server name; the `Status` column (`enabled`/`disabled`)
/// is the best available connection signal codex exposes.
pub(in super::super) fn parse_codex_mcp_list(stdout: &str) -> Vec<(String, bool)> {
    let mut servers = Vec::new();
    for line in stdout.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        // Skip the header row and any rule/separator lines.
        if name == "Name" || name.chars().all(|c| c == '-' || c == '=') {
            continue;
        }
        // `disabled` contains `enabled` as a substring — check it first.
        let enabled = line.contains("enabled") && !line.contains("disabled");
        servers.push((name.to_string(), enabled));
    }
    servers
}
