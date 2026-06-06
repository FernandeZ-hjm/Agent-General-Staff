# AGENTS.md

@/Users/hujiaming/.agents/rules/core.md
@/Users/hujiaming/.codex/RTK.md

Before responding or executing tasks in this repository, also read and follow:

- `CLAUDE.md` — Rust core suite candidate workspace protocol.

@CLAUDE.md

## AGS: Standing Engineering Hub

Agent Governance Suite (AGS) is a standing engineering hub for development work,
not a CLI toolbox you invoke separately. When a development request arrives, AGS
governance engages automatically: ambient preflight → solution formation → user
confirmation (\"方案 OK\") → user task-card instruction (\"生成任务卡\") → execution
contract → task routing → gate / execution / receipt.

Do not jump to Light / Medium / Heavy classification from raw user requests.
Always complete preflight and solution formation first. \"方案 OK\" only ends the
solution phase — a separate user task-card instruction is required before routing
and task card generation. The task card template (`protocol/task-card-template.md`)
takes a confirmed execution contract as input, not raw chat messages. The
`ags task compile` command requires `--task-card-requested` before it will output
an executable task card.

## Protocol Authority

This repository is the **Rust core suite candidate** for Agent Governance Suite.
Canonical protocol files and workspace entry points live in the development
private suite at:

```
/Volumes/AI Project/agent-governance-suite-private
```

The protocol files under `protocol/` in this repository are reference copies for
standalone Rust core execution. When the development private suite is available,
its `protocol/`, `AGENTS.md`, `CLAUDE.md`, and `WORKSPACE.md` are the authority.

## Kernel Activation — Session Preflight

`ags session preflight` is the default kernel activation wake-up entry point.
`ags verify --scope local|full|release` is the structured verification entry point
with stable `CheckItem` model and machine-readable JSON output. `scripts/verify.sh`
is now a compatibility wrapper that delegates to `ags verify --scope full`.

Before executing any task, agents should run:

```bash
ags session preflight --for codex     # Codex pre-execution lifecycle
ags session preflight --for claude-code  # Claude Code execution
ags session preflight --for cursor    # Cursor IDE workflow
```

This aggregates project identity, protocol status, agent instructions, memory
paths, stop conditions, warnings, failures, and next steps into a single
read-only report. It does NOT depend on skill governance — core kernel
activation is independent of third-party skill governance.

See `CLAUDE.md` M2 Agent Awareness Commands section for all M2 read-only
commands.
