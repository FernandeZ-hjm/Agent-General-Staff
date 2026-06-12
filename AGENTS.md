# AGENTS.md

Before responding or executing tasks in this repository, also read and follow:

- `CLAUDE.md` — Rust core suite protocol.

@CLAUDE.md

## AGS: Standing Engineering Hub

Agent Governance Suite (AGS) is a standing engineering hub for development work,
not a CLI toolbox you invoke separately. Public AGS exposes Claude Code `/ags`,
Codex `$ags-setup` / `$ags-init` / `$ags-skill` / `$ags-doctor`, and the
`ags mcp serve --transport stdio` kernel bridge. When a development request
arrives, AGS governance engages automatically: ambient preflight → solution
formation → user confirmation ("方案 OK") → user task-card instruction
("生成任务卡") → execution contract → task routing → gate / execution / receipt.

When AGS MCP is available, every AGS-related task must explicitly call the MCP
`ags_preflight` tool first. CLI preflight is a fallback path only when MCP is not
available.

Do not jump to Light / Medium / Heavy classification from raw user requests.
Always complete preflight and solution formation first. "方案 OK" only ends the
solution phase — a separate user task-card instruction is required before routing
and task card generation. The task card template (`protocol/task-card-template.md`)
takes a confirmed execution contract as input, not raw chat messages. The
`ags task compile` command requires `--task-card-requested` before it will output
an executable task card.

## Protocol Authority

This repository is the **public distributable edition** of the Agent Governance
Suite. Canonical protocol files live under `protocol/` and are self-contained
within this repository. No private infrastructure or private repositories are
required to build, run, or use AGS.

## Kernel Activation — Session Preflight

`ags session preflight` is the default kernel activation wake-up entry point.
`ags mcp serve --transport stdio` is the public MCP server entry point.
`ags verify --scope local|full|release` is the structured verification entry point
with stable `CheckItem` model and machine-readable JSON output. `scripts/verify.sh`
is a compatibility wrapper that delegates to `ags verify --scope full`.

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

## Project Entry Integration

User projects often already have their own `AGENTS.md` and `CLAUDE.md`. Do not
replace those files with the public suite copies. To integrate AGS into a user
project, use the incremental managed-block workflow:

```bash
ags project integrate --target /path/to/repo --dry-run
ags project integrate --target /path/to/repo --confirm
```

The integration command preserves user-authored content, updates only the
`<!-- AGS:BEGIN managed-entry v2 -->` block when present, and stops on conflicting
entry-file instructions instead of overwriting them.
