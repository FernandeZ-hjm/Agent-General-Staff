# AGS Global Governance Kernel

> Stable URI: `ags://global-kernel`

## Initialization Gate

Before any other AGS action, call `ags_preflight(agent, target?)`. If MCP is unavailable, use `ags session preflight --for <agent> --target <path>`. Failure stops the AGS scenario. Preflight binds the current connection to host/target and exposes `ags://capabilities/current-host`.

## Canonical Flow

```text
human request
→ host keeps complete conversation context
→ host reads current-host SkillCard catalog
→ HostRouteProposal
→ ags_route_request (strictly read-only)
→ DirectResponse | exact SkillSelection | host-native edit | server-held action
→ ags_apply_action(lease_id, action_id) only for a held action
```

The host is the only natural-language semantic node. AGS rejects legacy raw request input and never falls back to keywords, similarity search, or SkillDemand routing.

## Proposal Rules

- `DirectResponse` is exclusive and terminal.
- Otherwise, at most one exact `SkillTarget` and one closed `MachineCliTarget` may coexist.
- A SkillTarget carries only `skill_id`, optional `entrypoint`, and `snapshot_hash`.
- Confirmed same-session direct edit is host-native and does not compile a task card.
- Existing canonical task cards validate first and use `TaskPrepareExecution`; they do not re-enter solution formation.
- Task-card generation requires explicit handoff intent plus a confirmed, closed handoff contract.

## Resolve / Apply Boundary

`ags_route_request` launches no process and writes no file. Effectful actions remain in the current MCP connection and are bound by a one-shot `DecisionLease` over host, target, proposal, scope, registry, snapshot, and policy hashes. `ags_apply_action` is the only effectful MCP tool and accepts only lease/action references plus an optional controlled outcome. New preflight, new route, connection reset, binding drift, or any consumption invalidates the old lease.

## Skill Resolution

Skill Resolver validates exact identifiers against a preflight-bound `HostCapabilitySnapshot`. It has no keyword, similarity, or fallback path. Missing or stale state fails closed; refresh explicitly with `ags capability snapshot --host <host> --write`.

## Runner Boundary

`TaskPrepareExecution` runs validate → policy → gate → LaunchPlan. An allowed plan returns `HOST_EXECUTION_REQUIRED`. Runner does not launch the host, execute the task, verify results, write the final receipt, or claim completion.

## Completion

The host applies the relevant review and verification gates and emits receipts for writes. Full protocol: `ags://protocol/agent-task-protocol`, `ags://protocol/task-routing`, and `protocol/skill-governance.md` in the authority workspace.
