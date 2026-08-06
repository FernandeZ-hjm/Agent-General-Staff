# ADR 0001: Workspace Service and Deep Module Boundaries

- Status: Accepted
- Introduced in: 0.3.1
- Current implementation: 0.5.0

## Context

Per-stdio governance state allowed multiple hosts in one project to observe
different capability hashes and connection-bound leases. Large source files
also mixed read models, host probes, lifecycle writes, snapshot publication,
and rendering.

## Decision

Use one service per canonical workspace and make MCP stdio a thin
`connect-or-start` proxy. Keep sessions, preflight bindings, and actions
client-local. Publish one static snapshot per host by
validate-then-atomic-replace during explicit lifecycle updates.

Workspace lifecycle uses the same service boundary. Host-native
SessionStart, Stop/guard, and SessionEnd adapters only translate their native
event schema into the shared lifecycle envelope. Memory reads, idempotency,
receipt-bound closure, and lifecycle state belong to the workspace service;
the host and user-level configuration do not own a second state machine.

The request path never rebuilds or compares live capability observations.
The daemon loads a host snapshot once, validates its sealed hashes, and reuses
that same object for preflight, resource reads, route, and apply. There is no
workspace capability bundle or bundle epoch. An explicit snapshot refresh
becomes active only after daemon restart/reconnect, which also invalidates old
sessions and leases.

Split the four largest modules by cohesive change reason:

- capability resolver: authority, catalog, snapshot compiler/validation,
  local third-party manifest and hashing;
- workspace facts: discovery, instruction projection, protocol audit,
  preflight and rendering;
- lifecycle: setup, project initialization, updates and onboarding assessment;
- MCP tools: wire, preflight, decision, apply and tests.

Cargo package names identify the twelve major architectural boundaries. In
0.3.2 the former support-package implementations moved under those owners and
their package manifests were removed. v0.3.6 exposes the current contract only;
retired packages and aliases do not remain as alternate authorities.

## Consequences

Four hosts can share one workspace service without sharing decision authority.
Capability scans occur only in explicit setup, update, or snapshot publication,
never while serving a request. Cross-session, cross-host,
cross-workspace, and replayed leases fail
closed. `ags mcp serve --transport stdio` remains the host entry;
connect-or-start is internal. Future refactors must preserve these boundaries
without retaining obsolete compatibility paths.
