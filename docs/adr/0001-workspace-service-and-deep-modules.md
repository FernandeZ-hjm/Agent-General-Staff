# ADR 0001: Target-routed workspace services and the deep control plane

- Status: Accepted
- Current contract: v0.4.20 / contract v2

## Context

Binding stdio transport, workspace identity, and action state as one object was
safe only for per-project hosts. A Generic Agent may use one global MCP process
for concurrent sessions in several workspaces. In that environment, daemon cwd
or last-writer binding can route a correct request to the wrong project.

Separately, human commands, hidden machine commands, MCP tools, and lifecycle
handlers had accumulated parallel semantics. The command count and duplicated
orchestration made both safety review and ordinary use harder.

## Decision

Keep transport, workspace resolution, and security binding independent:

1. The stdio connection owns transport identity and normalized host identity.
2. Every request resolves a canonical workspace from explicit context, one
   unique MCP root, or adapter cwd, in that order.
3. The router authenticates or reuses a distinct session with that workspace's
   daemon and creates an immutable `WorkspaceBinding`.
4. `open` and `decide` seal the operation plan to the binding. `apply` consumes
   only its action reference on the same binding.

The router stores no global current workspace. Daemon process cwd, HOME,
managed-project guesses, and fuzzy matching never decide governance identity.

All domain orchestration moves into one deep `ags-control-plane` Module. A typed
operation registry is the single declaration source for CLI, JSON, MCP schema,
help metadata, policy, plan, and handler routing. The external behavioral
Interface is only `open`, `decide`, and `apply`.

## Consequences

- One Generic Agent connection can interleave A -> B -> A safely while reusing
  authenticated sessions after the first handshake.
- Cross-workspace, cross-host, cross-connection, restarted-session, replayed,
  and tampered action references fail closed.
- Official host adapters remain useful probes/hooks but are not an admission
  allowlist.
- The CLI contracts to ten top-level commands and MCP to two tools.
- Contract v2 removes older command, tool, wire, and schema surfaces instead of
  carrying aliases or compatibility translation.
- Stable/public promotion, release, installation, and third-party MCP
  registration remain outside this decision and require their own authority.
