use crate::capability::cmd_capability_verify;

/// `ags agents verify` — host AGS visibility (thin-index + AGS MCP). Delegates
/// to the canonical capability verify.
pub(in crate::agents) fn cmd_agents_verify(host: &str, strict: bool, format: &str) {
    cmd_capability_verify(host, strict, format);
}

// ── ags update — unified update (五段心智第 5 段) ─────────────────────────────
