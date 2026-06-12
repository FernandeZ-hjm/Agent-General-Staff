# AGS Global Governance Kernel

> Stable URI: `ags://global-kernel`
> Applicable to any project. Does NOT assume the target is an AGS-governed repo.
> AGS MCP and EvoMap MCP are parallel peers — call both. AGS wins on governance gates.

---

## AGS Initialization Gate (MANDATORY FIRST)

**Before any other AGS tool or lifecycle phase**, the host MUST complete the
AGS Initialization Gate. This is a non-negotiable requirement for all AGS
scenarios.

### Trigger conditions — an AGS scenario is active when:

- The repo is under AGS governance (root `AGENTS.md` + `CLAUDE.md`, or
  `ags project detect` returns `suite` / `integrated`)
- The task involves task cards, skill governance, MCP governance,
  multi-agent routing, release boundaries, or execution policy
- AGS Evolver boundary is involved
- The user references AGS protocol, task cards, or governance gates

### Legal invocation paths (in priority order):

1. **MCP (preferred)**: call `ags_preflight` with `agent` + optional `target`
2. **CLI fallback**: `ags session preflight --for <agent> [--target <path>]`
3. **Both unavailable**: STOP — do not continue AGS scenario tasks

### Prohibition rules:

- Do NOT read docs and manually simulate preflight
- Do NOT skip preflight based on model memory or user oral description
- Do NOT enter solution formation, routing, or execution without preflight evidence
- `ags_solution_check` is a phase gate, NOT a preflight substitute — preflight
  must complete first

### Evidence format:

```
AGS preflight: MCP|CLI, agent=<agent-id>, status=<ok|failed|fallback>
```

### After preflight succeeds:

- Proceed to the mandatory development lifecycle below
- All other AGS tools (`ags_solution_check`, `ags_task_validate`, `ags_policy_resolve`,
  `ags_verify_local`) may now be called

---

## Mandatory Development Lifecycle

All development, debugging, review, commit, and task-card work must follow
this lifecycle. **Do not skip phases.**

### 0. Initialization Gate (precedes all other phases)

Call `ags_preflight` (MCP) or `ags session preflight --for <agent>` (CLI fallback).
Record evidence. Do NOT proceed to step 1 without it.

### 1. Ambient Preflight

Read project context, protocol files, context capsule, and task memory.
Run `git status --short` to record current state.

- If the task goal conflicts with the capsule's `## 项目设计目的`, STOP and report.
- This phase is read-only context gathering — no task classification yet.

### 2. Solution Formation

- Understand the request, clarify ambiguities, diagnose issues.
- For **non-trivial tasks** (Medium, Heavy, or any development/architecture/refactoring/release/governance change): **call EvoMap MCP in parallel for advisory method recall.**
- AGS MCP does NOT call EvoMap MCP — hosts must call both MCPs.
- Form a concrete solution or implementation approach.
- **Do NOT classify as Light/Medium/Heavy during this phase.**

### 3. User Confirmation

Present the solution and wait for explicit user approval ("方案 OK").
Do NOT proceed to routing without confirmation.

### 4. Task-Card Instruction Gate (HARD GATE)

**"方案 OK" alone only ends step 3. It does NOT authorize a task card.**

The user must explicitly issue a task-card instruction:
- "生成任务卡"
- "按这个方案出任务卡"
- "交给 Claude Code 执行"
- "帮我写个任务卡拉去执行"

Without this instruction, `ags task compile` blocks executable output with:
- `executable_allowed: false`
- `block_reason: task_card_not_requested`

**Three-gate threshold**: 方案 OK → 任务卡指令 → 任务分级路由.

### 5. Execution Contract → Routing

Based on the **confirmed solution** (not the raw user request),
classify the task as Light, Medium, or Heavy per `protocol/task-routing.md`.

### 6. Gate / Execution / Receipt

- Validate task card through `ags task validate`
- Resolve execution policy through `ags policy resolve`
- Execute per resolved policy
- Verify with narrowest relevant verification
- Output a delivery report per `protocol/agent-task-protocol.md`

---

## Critical Rules

1. Initialization Gate: call `ags_preflight` (MCP) or `ags session preflight --for <agent>` (CLI fallback) FIRST.
2. Do NOT jump to Light/Medium/Heavy classification from raw user requests.
3. Always complete preflight + solution formation + user confirmation first.
4. "方案 OK" ≠ task card approval.
5. Raw user chat ≠ executable task card.
6. AGS is the governance authority (lifecycle, gates, task level, permission mode, review gate, verification gate, release boundary).
7. AGS MCP is the host initialization adapter and mandatory governance interface — NOT a governed third-party MCP.
8. EvoMap provides advisory method recall during solution formation only.
9. EvoMap must not decide task level, permission mode, review gate, verification gate, or release boundary.
10. If EvoMap output conflicts with AGS protocol, project memory, task card, or gates — AGS wins.

---

## EvoMap Parallel-Call Boundary

```
┌──────────────┐     ┌──────────────┐
│  AGS MCP     │     │  EvoMap MCP  │
│  (host       │     │  (advisory   │
│   adapter,   │     │   memory)    │
│   mandatory  │     │              │
│   first)     │     │              │
└──────┬───────┘     └──────┬───────┘
       │                    │
       │  ─── parallel ───  │
       │  (not brokered)    │
       ▼                    ▼
   Host agent calls both MCPs independently.
   AGS decides; EvoMap advises.
```

- AGS MCP and EvoMap MCP are **parallel peers** — neither wraps the other.
- AGS MCP does NOT proxy, broker, or call EvoMap MCP.
- Hosts must configure and call both MCPs separately.
- If EvoMap MCP is unavailable, the solution proceeds without it,
  but the planner must state the reason explicitly.

---

## AGS vs Governed MCPs

AGS MCP is structurally distinct from third-party MCPs:

- **AGS MCP** = `suite_interfaces` in `manifests/mcp-registry.yaml` — host
  initialization adapter, mandatory governance interface, NOT a governed object.
- **Third-party MCPs** = `mcps` in `manifests/mcp-registry.yaml` — `context7`,
  `gep`, etc. — reviewed, registered, and managed by AGS governance.

AGS is the governance authority; it is not in the governed MCP list.

---

## Task Level Defaults

| Level | Blast radius | Permission default | Review gate |
|-------|-------------|-------------------|-------------|
| Light | Single file, narrow path | `execute-and-verify` | Light diff review |
| Medium | Cross-file, module boundary | `edit-with-confirmation` | Codex review |
| Heavy | Data, migration, architecture, baseline | `plan-only` | Human adversarial review |

**Escalate when in doubt.** Escalation triggers include data loss,
irreversible writes, baseline mutation, cross-file protocol changes,
hook/runtime adapter changes, or ambiguous domain rules.

---

## Stop Conditions

Stop and report before proceeding when:
- Task conflicts with context-capsule project design purpose.
- AGS preflight (MCP + CLI fallback) is unavailable.
- Requires writing WorkBuddy global config.
- Requires installing or enabling EvoMap MCP.
- Requires real tokens, node_secret, or `~/.evolver/settings.json`.
- Requires modifying stable/public worktree.
- Would change AGS lifecycle/gate semantics.
- Would proxy EvoMap MCP into AGS MCP or build an MCP broker.
