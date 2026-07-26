# ADR 0001: Workspace Service and Deep Module Boundaries

- Status: Accepted
- Product version: 0.3.2

## Context

Per-stdio governance state allowed multiple hosts in one project to observe
different capability hashes and connection-bound leases. Large source files
also mixed read models, host probes, mutation planning, apply transactions,
rollback, snapshot publication, and rendering.

## Decision

Use one service per canonical workspace and make MCP stdio a thin
`connect-or-start` proxy. Keep sessions, preflight bindings, and actions
client-local. Publish capability bundles by validate-then-atomic-replace.

Split the four largest modules by cohesive change reason:

- capability governance: authority, catalog, snapshot compiler/validation,
  user overlay transaction, usage ledger, hashing;
- workspace facts: discovery, instruction projection, protocol audit,
  preflight and rendering;
- skill console: model, inventory, probe, actions, apply transaction,
  synchronization, dedupe and rendering;
- MCP tools: wire, preflight, decision, apply and tests.

Cargo package names identify the twelve major architectural boundaries. No
retired source path may retain an alternate routing, snapshot, lease, or
mutation implementation.

## Consequences

Four hosts can share scans and capability publication without sharing decision
authority. Cross-session, cross-host, cross-workspace, and replayed leases fail
closed. The human CLI and Machine CLI remain byte-compatible with v0.3.0 help
and output contracts. Future refactors move behavior behind these boundaries
before changing adapters.
