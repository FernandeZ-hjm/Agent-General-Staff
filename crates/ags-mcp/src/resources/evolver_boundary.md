# AGS MCP ↔ EvoMap MCP: Parallel-Call Boundary

> Stable URI: `ags://evolver-boundary`

---

## Architecture

AGS MCP and EvoMap MCP are **parallel peers**, not a broker/client hierarchy.

```
                    ┌──────────────────┐
                    │   MCP Host       │
                    │  (WorkBuddy /    │
                    │   Codex / Cursor │
                    │   / Claude Code) │
                    └───┬──────────┬───┘
                        │          │
              ┌─────────┘          └─────────┐
              ▼                              ▼
    ┌─────────────────┐          ┌─────────────────┐
    │    AGS MCP      │          │   EvoMap MCP    │
    │  (governance    │          │  (advisory      │
    │   authority)    │          │   memory)       │
    │                 │          │                 │
    │  Decides:       │          │  Advises on:    │
    │  - lifecycle    │          │  - design patterns
    │  - task level   │          │  - reusable methods
    │  - permission   │          │  - risk flags    │
    │  - review gate  │          │  - edge cases    │
    │  - verify gate  │          │                 │
    │  - release      │          │  Does NOT decide:│
    │    boundary     │          │  - task level    │
    │                 │          │  - permission    │
    │  Call:          │          │  - review gate   │
    │  - always       │          │  - verify gate   │
    │  - governance   │          │  - release       │
    │    gates        │          │    boundary      │
    │  - verification │          │                 │
    │                 │          │  Call:           │
    │                 │          │  - solution      │
    │                 │          │    formation     │
    │                 │          │    only          │
    └─────────────────┘          └─────────────────┘
```

---

## Authority Rules

### AGS MCP is the governance authority

AGS decides:
- Task lifecycle (preflight → solution → confirmation → routing → execution → receipt)
- Task level (Light / Medium / Heavy)
- Permission mode (read-only / plan-only / edit-with-confirmation / execute-and-verify)
- Parallelism (none / subagent / worktree / multi-session / agent-team)
- Review gate
- Verification gate
- Release boundary
- Protected-path handling
- Stop conditions

If a task card, protocol document, or project memory sets a rule,
AGS enforcement always wins. EvoMap output cannot override.

### EvoMap MCP is advisory memory

EvoMap provides:
- Design pattern suggestions
- Reusable method recall (Genes, Capsules, EvolutionEvents)
- Risk flags and edge case notes
- Past task approaches as reference

EvoMap influence is limited to **solution formation only**.
EvoMap output is **input** to the planner's solution text —
it does not automatically become project truth or task-card content.

### Conflict resolution

When EvoMap output conflicts with AGS protocol, project memory,
task cards, gates, or release boundaries:

1. Flag the conflict in the solution text.
2. Follow AGS — AGS authority always wins.
3. Do NOT silently adopt EvoMap suggestions that override AGS gates.

---

## What AGS MCP Does NOT Do

AGS MCP does **not**:

- Proxy, wrap, or broker EvoMap MCP calls
- Call EvoMap MCP search, list, or recall tools
- Return EvoMap Gene/Capsule/EvolutionEvent data
- Install or configure EvoMap MCP
- Require EvoMap MCP to be present for AGS tools to work
- Forward EvoMap recall results to the MCP host

`ags_solution_check` tool **recommends** EvoMap recall for non-trivial tasks
but records `recall_status: unavailable_or_not_called` — it is the **host's
responsibility** to call EvoMap MCP in parallel.

---

## Host Integration

### Correct pattern

```
0. Host detects AGS scenario (see trigger conditions).
1. Host calls AGS MCP `ags_preflight` — MANDATORY FIRST (Initialization Gate).
   If MCP unavailable: CLI fallback `ags session preflight --for <agent>`.
   If both unavailable: STOP. Record evidence: "AGS preflight: MCP|CLI, agent=<agent>, status=<ok|failed|fallback>".
2. Host reads AGS context (capsule, task memory, protocol files) surfaced by preflight.
3. Host calls AGS MCP `ags_solution_check` to check phase.
4. If `evomap_recall_recommended: true`:
   a. Host calls EvoMap MCP recall/search/list in parallel.
   b. Host incorporates EvoMap advisory results into solution formation.
5. Host presents solution to user, waits for confirmation.
6. After user issues task-card instruction:
   a. Host calls AGS MCP `ags_task_validate` on the task card.
   b. Host calls AGS MCP `ags_policy_resolve` on the validated task card.
7. Host dispatches execution per resolved policy.
8. Host calls AGS MCP `ags_verify_local` for verification.
```

### Anti-patterns (DO NOT DO)

```
- Skipping `ags_preflight` and going directly to ags_solution_check or other AGS tools
- Calling AGS MCP and expecting it to return EvoMap recall results
- Calling EvoMap MCP and expecting it to enforce AGS gates
- Building an MCP proxy that merges AGS + EvoMap into a single endpoint
- Having AGS MCP call EvoMap MCP internally
- Using EvoMap output to override task-card level, permission, or gate settings
- Reading AGS docs and manually simulating preflight instead of actually calling it
```

---

## Planner Recall Documentation

When using EvoMap during solution formation, the solution text must
include a structured recall section per `protocol/evolution-memory.md`:

| Field | Required values |
|-------|----------------|
| `status` | `available` / `unavailable` / `skipped` |
| `search` | `full` / `low_confidence_only` / `none` |
| `fetch` | `success` / `failed` / `not_attempted` |

And document: recall path, input signals, hit signals, cited Gene/Capsule,
adoption, rejection, impact, and confidence/limitations.

---

## Method Capture (Post-Task)

After task completion, EvoMap captures **reusable method only**.
It must NOT write project truth (context-capsule, task-memory, task-archive,
delivery report, receipt, verification evidence).

Evidence priority:
```
delivery report / receipt / verification result > git diff signal > fallback observation
```

- `successful method` = has authoritative success evidence.
- `observed method` = only git diff or fallback; must remain `observed`.
