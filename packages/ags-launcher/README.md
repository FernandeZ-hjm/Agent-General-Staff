# @agent-governance-suite/launcher

Internal bootstrap launcher used by the public AGS CLI and MCP packages. It
verifies the package-pinned signed release index and exact platform artifact,
maintains the content-verified initial cache, then starts the requested Rust
binary with the verified cache identity in its environment.

It does not intercept `ags update`, choose a later version, apply an upgrade,
or emit update reminders. The Rust `ags upgrade` deep module owns signed
checking, sealed planning, activation, verification, recovery and host-global
seven-day reminder state for native, npm and MCP entrances alike.

The verification marker and pointer stay byte-compatible with Rust's v3
runtime identity: one canonical digest covers the five executables, and one
path/size/content digest covers `runtime/ags-skills`.

Install `@agent-governance-suite/cli` or `@agent-governance-suite/mcp`; this
package is their common bootstrap dependency, not a maintenance interface.
