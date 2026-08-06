# @agent-governance-suite/launcher

Internal shared launcher used by the public AGS CLI and MCP packages. It
verifies the signed release index and exact platform artifact, maintains one
content-verified cache, and owns plan-bound AGS update activation and recovery.
For an initialized runtime, the same core update Plan seals the candidate
setup projection; apply must verify the runtime and required suite Skills
before the pointer is healthy, and recovery restores both runtime state and
the previous binary pointer.

Install `@agent-governance-suite/cli` or `@agent-governance-suite/mcp`; this
package is their common runtime dependency, not a separate user interface.
