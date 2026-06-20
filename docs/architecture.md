# AGS Architecture

This document describes the internal architecture of Agent General Staff 2.0
Public Edition. It covers the lifecycle phases, the Rust CLI crate dependency
graph, the AGS MCP host initialization adapter, the task-card-to-execution
pipeline, and the memory capsule mechanism.

## 1. AGS Lifecycle

The AGS governance lifecycle is a linear sequence of phases. Each phase gates
the next — no phase may be skipped or executed out of order.

```mermaid
flowchart TD
    A[User Request] --> B[1. Ambient Preflight]
    B --> B0{AGS MCP available?}
    B0 -->|Yes| B1[ags_preflight via AGS MCP]
    B0 -->|No| B1F[CLI fallback: ags session preflight]
    B1 --> B2[Read context capsule + task memory]
    B1F --> B2
    B2 --> B3[Check git status]
    B3 --> B4[Load protocol files]
    B4 --> C[2. Solution Phase]
    C --> C1[Understand request]
    C1 --> C2[Diagnose if needed]
    C2 --> C3[Form solution]
    C3 --> C4[Present to user]
    C4 --> D{User confirms?}
    D -->|方案 OK| E[3. Execution Contract]
    E --> F{Task-card instruction?}
    F -->|生成任务卡| G[4. Task-Card Instruction Gate ✅]
    F -->|No| F_WAIT[Wait — ags task compile blocks without --task-card-requested]
    F_WAIT --> F
    D -->|No| C
    G --> H[5. Routing Phase]
    H --> H1[Light / Medium / Heavy classification]
    H1 --> I[6. Task Card Generation]
    I --> J[7. Gate Check]
    J --> J1[ags task validate — hard gate]
    J1 -->|Pass| K[8. Policy Resolution]
    J1 -->|Fail| J_FAIL[Fix task card]
    J_FAIL --> I
    K --> K1[ags policy resolve — soft resolution]
    K1 --> L{stop_before_launch?}
    L -->|Yes| L_STOP[STOP: fix task card or get approval]
    L -->|No| M[9. Execution]
    M --> N[10. Verification]
    N --> O[11. Receipt Generation]
    O --> P[12. Task Memory Update]
    P --> Q[Done]

    style B fill:#e1f5fe
    style C fill:#fff3e0
    style E fill:#f3e5f5
    style G fill:#ffeb3b
    style H fill:#e8f5e9
    style J fill:#ffcdd2
    style K fill:#ffcdd2
    style M fill:#c8e6c9
    style O fill:#b3e5fc
```

**Key gates:**

| Gate | What It Blocks | Hard/Soft |
|---|---|---|
| AGS MCP initialization gate | AGS scenarios before `ags_preflight` completes | Hard, with CLI fallback only if MCP is unavailable |
| Task-card instruction gate | Routing before explicit "生成任务卡" | Hard |
| Task-card validation | Execution of invalid task cards | Hard |
| Policy resolution | Execution with wrong permission/parallelism | Soft (downgrades, never rejects) |
| Verification gate | Delivery claims without evidence | Per task card |

`ags_preflight` is the preferred kernel activation entry when AGS MCP is
available. `ags session preflight` is the equivalent CLI fallback, not the
primary path for MCP-capable hosts.

## 2. Rust CLI Crate Architecture

AGS is organized as a Rust workspace with multiple crates. Each crate has a
single responsibility.

```mermaid
graph TD
    A[ags-cli<br/>Binary Entry Point] --> B[clap CLI<br/>Subcommand Router]
    B --> C1[task-card-validator<br/>Task Card Validation]
    B --> C2[execution-policy<br/>Policy Resolution]
    B --> C3[suite-doctor<br/>Health Diagnostics]
    B --> C4[bootstrap-dry-run<br/>Bootstrap Simulation]
    B --> C5[workflow-sync-check<br/>Protocol Drift Check]
    B --> C6[ags-verify<br/>Scoped Verification]
    B --> C7[project-discovery<br/>Project Detection]
    B --> C8[receipt<br/>Receipt & Compliance]
    B --> C9[task-compiler<br/>Task Card Compilation]
    B --> C10[skill-governance<br/>Skill Management]
    B --> C11[capability-registry<br/>Capability Detection]
    B --> C12[runner<br/>Runner Launch]
    B --> C13[ags-mcp<br/>Host Initialization Adapter]

    C2 --> C1
    C2 --> C8
    C6 --> C1
    C6 --> C5
    C9 --> C1
    C10 --> C11
    C13 --> C7
    C13 --> C1
    C13 --> C6

    style A fill:#1565c0,color:#fff
    style B fill:#1976d2,color:#fff
    style C1 fill:#43a047,color:#fff
    style C2 fill:#43a047,color:#fff
    style C6 fill:#fb8c00,color:#fff
    style C8 fill:#8e24aa,color:#fff
```

**Crate responsibilities:**

| Crate | Responsibility | Primary consumer |
|---|---|---|
| `ags-cli` | CLI entry point, clap routing | Users, CI |
| `task-card-validator` | Canonical task-card format gate | `execution-policy`, `task-compiler`, `ags verify` |
| `execution-policy` | Resolve how a valid task card should execute (M1–M10 rules) | Runner, scripts |
| `suite-doctor` | Health diagnostics, missing-file detection | Users, preflight |
| `bootstrap-dry-run` | Simulate project bootstrap without writing | Users, `ags bootstrap` |
| `workflow-sync-check` | Multi-target protocol drift detection | `ags verify --scope full` |
| `ags-verify` | Scoped verification orchestrator (`local`/`full`/`release`) | Users, CI, preflight |
| `project-discovery` | Detect project identity and AGS integration | `ags_preflight`, `ags session preflight` |
| `receipt` | Receipt generation, verification, compliance check | Runner, verification gate |
| `task-compiler` | Compile execution contract into canonical task card | Codex, Cursor |
| `skill-governance` | Skill scan, check, propose, install, adopt, ignore | Users |
| `capability-registry` | Detect available capabilities (MCP, tools, skills) | `skill-governance` |
| `runner` | Launch executor with resolved policy | `scripts/run-task-card.sh` |
| `ags-mcp` | Expose read-only AGS governance tools/resources/prompts over stdio MCP; requires `ags_preflight` first | MCP hosts: Codex, Claude Code, Cursor, WorkBuddy |

## 3. AGS MCP Host Initialization Adapter

AGS MCP is the suite's host initialization adapter. It is not a governed
third-party MCP and should not be listed with governed external MCPs. It exposes
the AGS governance kernel over stdio so MCP-capable hosts can call
`ags_preflight` before any other AGS action.

```mermaid
flowchart LR
    HOST[MCP Host<br/>Codex / Claude Code / Cursor / WorkBuddy]
    AGSMCP[AGS MCP<br/>ags mcp serve --transport stdio]
    PREFLIGHT[ags_preflight<br/>mandatory first call]
    PHASE[ags_solution_check<br/>phase gate]
    TOOLS[Read-only AGS tools<br/>agent instructions / protocol status / task validate / verify local]
    CLI[CLI fallback<br/>ags session preflight]

    HOST --> AGSMCP
    AGSMCP --> PREFLIGHT
    PREFLIGHT --> PHASE
    PREFLIGHT --> TOOLS
    HOST -. MCP unavailable .-> CLI

    style AGSMCP fill:#1565c0,color:#fff
    style PREFLIGHT fill:#ffeb3b,stroke:#f57f17
    style CLI fill:#e0e0e0
```

**Boundary rules:**

- AGS MCP is the mandatory governance interface for AGS scenarios when present.
- `ags_preflight` must be the first AGS MCP tool call.
- AGS MCP does not proxy, wrap, install, or require external advisory MCPs.
  Hosts call AGS MCP and any optional advisory MCP separately when both are
  available.
- CLI preflight remains a supported fallback when the host cannot call AGS MCP.

## 4. Task-Card to Execution Pipeline

This diagram shows the data flow from a raw task card through validation, policy
resolution, and execution to the final receipt.

```mermaid
flowchart LR
    subgraph Input
        TC[Task Card<br/>markdown text]
    end

    subgraph Validation["Hard Gate"]
        V[task-card-validator]
        VF[Format checks<br/>Field validation<br/>Combination checks<br/>Authority Gate<br/>Contradiction detection]
        TC --> V
        V --> VF
    end

    subgraph Resolution["Soft Resolution"]
        PR[execution-policy<br/>resolver]
        RULES[M1-M10 rules<br/>Downgrade engine<br/>Launch arg synthesis]
        V -->|pass| PR
        PR --> RULES
    end

    subgraph Policy["Resolved Policy"]
        RP[ResolvedExecutionPolicy]
        RULES --> RP
        RP --> RP_FIELDS["effective_permission_mode<br/>effective_parallelism<br/>effective_execution_surface<br/>allowed_launch_args<br/>stop_before_launch<br/>requires_confirmation_gate"]
    end

    subgraph Execute["Execution"]
        RUN{stop_before_launch?}
        RP --> RUN
        RUN -->|true| STOP[STOP: refuse launch]
        RUN -->|false| LAUNCH[Launch executor<br/>with allowed_launch_args]
        LAUNCH --> CONF{confirmation_gate?}
        CONF -->|true| WAIT[Present plan<br/>Wait for approval]
        WAIT --> EXEC[Execute]
        CONF -->|false| EXEC
    end

    subgraph Receipt
        RC[receipt crate]
        EXEC --> RC
        RC --> RCOUT[Receipt JSON<br/>+ Compliance check]
    end

    style V fill:#d32f2f,color:#fff
    style PR fill:#f57c00,color:#fff
    style RP fill:#388e3c,color:#fff
    style STOP fill:#d32f2f,color:#fff
    style RCOUT fill:#1976d2,color:#fff
```

**The two-gate architecture:**

1. **Validator (HARD gate)**: An invalid task card must be fixed before anything
   else. The validator checks format, required fields, field values, field
   combinations, protected paths, contradictions, and the Execution Authority Gate.
   Failure is fatal — no soft recovery, no downgrade, just stop and fix.

2. **Policy resolver (SOFT gate)**: A valid task card may still need adjustment.
   The resolver applies M1–M10 rules to downgrade permission, strip forbidden
   parallelism, block background execution for read-only cards, and add
   confirmation gates. It never rejects a valid card — it adjusts launch strategy
   and records every downgrade with audit-trail entries.

**Core invariant**: Runners MUST consume `allowed_launch_args` from the resolved
policy, NOT synthesize args from raw task-card fields. This ensures the M5/M6
writability gate (read-only/plan-only cards never produce write-type launch args)
cannot be bypassed.

## 5. Memory Capsule & Task Archive Mechanism

AGS provides durable project memory through a layered mechanism that grows with
project usage. The memory system is separate from the AGS public distribution —
only blank templates are shipped; real memory is user-grown state.

```mermaid
flowchart TD
    subgraph "Stable (Manual)"
        CC[context-capsule.md<br/>Manual-maintained<br/>Project charter + stable facts]
    end

    subgraph "Task Lifecycle"
        TM[task-memory.md<br/>Auto-refreshed<br/>Latest task index]
        TA[task-archive/<br/>Per-task archives<br/>Full audit trail]
    end

    subgraph "Session Entry"
        SP[ags_preflight<br/>or CLI preflight fallback]
        SP --> CC
        SP --> TM
    end

    subgraph "Task Execution"
        TASK[Task executed]
        TASK --> DR[Delivery Report]
        TASK --> RC2[Receipt JSON]
    end

    subgraph "Auto-Archive (Stop Hook)"
        DR --> ARCHIVE[Stop hook detects<br/>delivery report + receipt]
        ARCHIVE --> TM_UPDATE[Update task-memory.md<br/>with latest task summary]
        ARCHIVE --> TA_WRITE[Write full archive to<br/>task-archive/{{timestamp}}-archive.md]
    end

    subgraph "Next Session"
        NS[Next agent session]
        NS --> SP2[ags_preflight<br/>or CLI preflight fallback]
        SP2 --> CC2[Read context-capsule.md]
        SP2 --> TM2[Read task-memory.md]
        TM2 --> TA2[Read recent task archives]
        CC2 --> RULES2[Enforce project design purpose]
    end

    style CC fill:#e8f5e9
    style TM fill:#fff3e0
    style TA fill:#fce4ec
    style ARCHIVE fill:#e3f2fd
```

**Memory layers:**

| Layer | Maintainer | Content | Lifetime |
|---|---|---|---|
| `context-capsule.md` | Human | Project charter, stable facts, design-purpose, boundaries | Persistent, only manual edits |
| `task-memory.md` | Auto (Stop hook) | Rolling index of latest tasks with archive links | Persistent, auto-refreshed |
| `task-archive/` | Auto (Stop hook) | Full per-task archives with delivery reports and receipts | Persistent, append-only |
| `progress-log.md` | Auto (context-memory.sh) | Continuous progress log | Persistent, append-only |
| Delivery report | Executor | Per-task summary of changes, verification, risks | Per task, archived |
| Receipt | Runner | Structured JSON audit trail | Per task, archived |

**Safety rules:**

- Context capsule is manual-only. Automated scripts must not overwrite it.
- Task memory is auto-refreshed but human-reviewable.
- Memory capsule state is advisory context, not proof of current repository state.
- Real memory capsules and task archives are user-grown state. The AGS public
  distribution ships only blank templates under `templates/memory/`.
- `protocol/project-profile.md` and `protocol/context-memory.md` are public-safe
  protocol skeletons, not real memory.

**Integration flow:**

```
New project
  → ags bootstrap --apply
  → creates blank templates/memory/*
  → human fills context-capsule.md
  → tasks execute, Stop hook archives results
  → task-memory.md grows
  → next agent reads capsule + memory on preflight
```
