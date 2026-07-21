# AGENTS.md

Before responding or executing tasks in this repository, also read and follow:

- `CLAUDE.md` — Rust core suite protocol.

@CLAUDE.md

## AGS: Standing Engineering Hub

Agent General Staff (AGS) is a standing engineering hub. After preflight, the
host reads `ags://capabilities/current-host`, keeps the complete conversation
context, and submits a typed `HostRouteProposal` to read-only
`ags_route_request`. AGS validates phase, authority, one exact `SkillTarget`,
and one closed `MachineCliTarget`; it never interprets natural language.
`DirectResponse` is exclusive. `ags_apply_action` is the sole effectful MCP
tool and consumes a connection-bound, server-held action by lease/action ID.
Compiler, Policy, Gate, and Runner consume structured contracts only.

When AGS MCP is available, every AGS-related task must explicitly call the MCP
`ags_preflight` tool first. CLI preflight is a fallback path only when MCP is not
available.

Do not jump to Light / Medium / Heavy classification from raw user requests.
Preflight is always required, but solution formation is conditional: use an
already supplied and approved solution when one exists; otherwise form one before
classification. "方案 OK" confirms the design but does not authorize mutation.
An explicit same-session modification
instruction enters `direct-edit`; an explicit task-card/handoff instruction enters
task-card generation. The task card template (`protocol/task-card-template.md`)
takes a confirmed handoff contract as input, not raw chat messages. `ags task
compile` requires both `--task-card-requested` and
`--confirmed-handoff-contract` because it generates a handoff artifact;
task cards are not a prerequisite for authorized host-native direct edits.

An input whose first non-empty line is the canonical `## 任务卡` header is an
existing execution contract, not a raw request. Validate it before request
classification: a valid card continues to policy resolution and the runner; an
invalid card stops with validation errors and must never fall through to task-card
generation.

Task cards have exactly two permission modes: `plan-only` and
`execute-and-verify`. Light and Medium default to direct execution. Heavy
defaults to `plan-only`, but an explicitly authorized Heavy
`execute-and-verify` card executes and verifies directly; Heavy adds only its
independent review gate. Destructive, external-write, credential, migration,
and release boundaries remain independent stop conditions.

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
is a compatibility wrapper that delegates to `ags verify --scope local`.

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
