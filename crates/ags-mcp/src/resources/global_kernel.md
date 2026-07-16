# AGS Global Governance Kernel

> Stable URI: `ags://global-kernel`

## Initialization Gate

Before any other AGS action, call `ags_preflight(agent, target?)`. If MCP is unavailable, use `ags session preflight --for <agent> --target <path>`. Failure stops the AGS scenario.

## Canonical Flow

```text
human request
→ host-owned conversation context
→ ags_route_request
→ RequestDecision
→ DirectResponse | SkillDemand | MachineCli
```

`ags_route_request` is the only natural-language AGS entry. The router is stateless; the host retains conversation context and resubmits it after missing information is supplied.

## RequestDecision Rules

- `DirectResponse` is exclusive and terminal.
- At most one `Skill` and one `MachineCli` may coexist as peer targets.
- `DirectResponse + MachineCli` and `DirectResponse + Skill` are invalid.
- Bounded transformations and already-approved formatting work use DirectResponse.
- Large, boundary-uncertain architecture requests may select a system-architecture SkillDemand.
- Existing canonical task cards use `MachineCli::TaskExecute`.
- Task-card generation requires explicit handoff intent and `confirmed_handoff_contract=true`.
- Missing information returns `InsufficientContext { missing }`; AGS does not maintain a conversation state machine.

## Skill Resolution

Skill Resolver consumes only closed `SkillDemand` values. It validates the machine ActiveSkillTable snapshot and maps the demand through registry `demand_routes`. It does not parse natural language or choose fallbacks. Missing/stale state returns `skill_snapshot_stale`; refresh explicitly with `ags capability snapshot --host <host> --write`.

## Machine CLI

MCP invokes the real `ags` executable with fixed argv and no shell. Router capabilities are business-level actions; CLI owns internal validate → policy → gate → run → verify → receipt orchestration.

## Compiler Boundary

Compiler validates and compiles a structured, confirmed handoff contract. It does not inspect raw user language, select skills, or reopen solution formation.
