# AGS Global Governance Kernel

> Stable URI: `ags://global-kernel`

## Initialization Gate

Before any other AGS action, call `ags_preflight(agent, target?)`. If MCP is unavailable, use `ags session preflight --for <agent> --target <path>`. Failure stops the AGS scenario. Preflight binds the current connection to host/target and exposes `ags://capabilities/current-host`.

## Canonical Flow

```text
human request
→ host keeps complete conversation context
→ host reads current-host SkillCard and third-party catalog; a third-party
  natural-language candidate requires both routable metadata and Ready
  availability, while host-native MCP also requires live tool visibility
→ HostRouteProposal
→ ags_route_request (strictly read-only)
→ DirectResponse | exact SkillSelection | host-native edit | server-held action
→ ags_apply_action(lease_id, action_id) only for a held action
```

The host is the only natural-language semantic node. AGS rejects raw request input and never falls back to keywords or similarity search.

## Proposal Rules

- `DirectResponse` is exclusive and terminal.
- Otherwise, at most one exact `SkillTarget`, one exact `McpTarget`, and one closed `MachineCliTarget` may coexist.
- A SkillTarget carries only `skill_id`, optional `entrypoint`, and `snapshot_hash`.
- An McpTarget carries only canonical `mcp_id`, optional registered `tool`, and `snapshot_hash`; AGS returns host-native dispatch metadata and never proxies the third-party server.
- Confirmed same-session direct edit is host-native and does not compile a task card.
- Existing canonical task cards validate first and use `TaskPrepareExecution`; they do not re-enter solution formation.
- Explicit task-card generation requires handoff intent plus a confirmed,
  closed contract. In host Plan mode, the final decision-complete artifact is
  the canonical task card and uses `--host-plan-mode-final`; Plan UI approval
  switches to execution mode and dispatches the exact card without regeneration.

## Resolve / Apply Boundary

`ags_route_request` launches no process and writes no file. Effectful actions remain in the current MCP connection and are bound by a one-shot `DecisionLease` over host, target, proposal, scope, registry, snapshot, and policy hashes. `ags_apply_action` is the only effectful MCP tool and accepts only lease/action references plus an optional controlled outcome. New preflight, new route, connection reset, binding drift, or any consumption invalidates the old lease.

## Capability Resolution

Capability Resolver validates exact Skill and MCP identifiers against a preflight-bound `HostCapabilitySnapshot`. It has no keyword, similarity, or fallback path. Missing or stale state fails closed. A stale preflight stays bound for `DirectResponse`, reports `NEEDS_USER_DECISION` plus `capability_catalog.refresh.argv`, and blocks `SkillTarget` / `McpTarget` / `MachineCliTarget` until the user explicitly authorizes the machine-local snapshot write and the host runs preflight again. Preflight never refreshes the snapshot silently.

## Runner Boundary

`TaskPrepareExecution` runs validate → policy → gate → LaunchPlan. An allowed plan returns `HOST_EXECUTION_REQUIRED`. Runner does not launch the host, execute the task, verify results, write the final receipt, or claim completion.

## Advisory-system boundary

Third-party memory and advisory systems are parallel peers of AGS MCP, never governance authorities. Their output may inform solution formation but cannot change AGS task level, execution mode, review/verification gates, lease admission, or release boundaries.

## Completion

The host applies the relevant review and verification gates and emits receipts for writes. Full protocol: `ags://protocol/agent-task-protocol`, `ags://protocol/task-routing`, and `protocol/skill-governance.md` in the authority workspace.
