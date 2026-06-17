//! AGS MCP Server — host initialization adapter and mandatory governance
//! interface over MCP (Model Context Protocol).
//!
//! Exposes AGS governance tools, resources, and prompts via stdio JSON-RPC,
//! enabling Tencent Agent (WorkBuddy, CodeBuddy-Code), Codex, Cursor, Claude
//! Code and other MCP hosts to call AGS governance gates as a global capability.
//!
//! # Initialization Gate
//!
//! `ags_preflight` is the **mandatory first call** for all AGS scenarios.
//! Hosts MUST complete preflight (MCP or CLI fallback `ags session preflight
//! --for <agent>`) before invoking any other AGS tool. `ags_solution_check`
//! is a phase gate, NOT a preflight substitute.
//!
//! # Identity
//!
//! AGS MCP is the suite's own host adapter — NOT a governed third-party MCP.
//! In `manifests/mcp-registry.yaml`, `ags` resides under `suite_interfaces:`,
//! not alongside governed MCPs like `context7` or `gep` under `mcps:`.
//!
//! # EvoMap Boundary
//!
//! AGS MCP and EvoMap MCP are **parallel peers**. AGS MCP is the governance
//! authority; EvoMap MCP provides advisory method recall during solution
//! formation only. AGS MCP does NOT proxy, wrap, or broker EvoMap MCP calls.
//!
//! # Usage
//!
//! ```bash
//! ags mcp serve --transport stdio
//! ```

mod prompts;
mod protocol;
mod resources;
mod server;
mod tools;

pub use server::run_mcp_server;
