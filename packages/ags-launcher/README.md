# @agent-governance-suite/launcher

Internal shared launcher used by the public AGS CLI and MCP packages. It
verifies the signed release index and exact platform artifact, maintains one
content-verified cache, and owns plan-bound AGS update activation and recovery.
For an initialized runtime, the same core update Plan seals the candidate
contract-v3 runtime profile; apply runs `ags setup --source-root <signed
runtime>`, verifies the five official Skills, and only then leaves the new
pointer active. Recovery converges the previous signed runtime profile before
restoring its binary pointer.

Install `@agent-governance-suite/cli` or `@agent-governance-suite/mcp`; this
package is their common runtime dependency, not a separate user interface.
