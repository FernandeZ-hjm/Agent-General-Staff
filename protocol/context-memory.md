# Context Memory

Context memory provides cross-conversation continuity through native adapters
for Claude Code, Codex, and OMP without changing the cache-stable task-card
skeleton. Other hosts use the explicit read/capture fallback until they have a
native adapter and must not report `full`.

## Default Store

The suite-owned local memory store is:

```text
$HOME/.agents/memory/projects/<project-slug>/
```

Recommended files:

```text
context-capsule.md          # manual project charter
task-memory.md              # automatically refreshed task continuity summary
task-archive/               # full local receipt archive per task run
```

## Context Memory Authority

Context memory owns project truth.
If other local notes, summaries, or automation outputs conflict with context
memory, context memory wins.

`context-capsule.md` is the project charter and is manual-only.
`task-memory.md` stores task continuity facts.
`task-archive/` stores evidence and receipts.

The memory store is local. Do not publish it and do not copy it into public
suite releases. Projects with non-ASCII directory names should set
`project.slug` in `config/agent-project-profile.yaml`, or pass
`--project-slug`, so their local memory path is stable and does not collapse
to a generic fallback.

## Context Capsule Contract

`context-capsule.md` is a manual project charter. It must always contain this
manual block:

```markdown
## 项目设计目的

<只能人工修改。用于约束 AI 不偏离项目初衷、业务边界、产品方向。>
```

Rules for this block:

- runner / hook / capture must not overwrite it.
- automatic summaries must not rewrite it.
- it may change only when the user explicitly asks for a manual update.
- every task-start context path must read it before task execution.
- if the task goal conflicts with it, the agent must stop and report.

The same manual-only rule applies to project boundaries, core business
positioning, and principle-level decisions that require human judgment.

## Task-Card Use

Task cards must not paste long memory. Use the fixed `记忆胶囊` slot:

```text
记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
```

When the capsule exists, the executor may read it as stable project context.
Only short, task-relevant facts should be copied into `背景` or `实施要求`.
The executor must also read sibling `task-memory.md` when present before
starting work.

Task cards may also include a fixed `任务存档` slot:

```text
任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`
```

Before any memory exists this can be `无`. `ags run` does not execute or refresh
memory. The host's completed-task capture hook—or an explicit
`context-memory.sh capture`—refreshes `task-memory.md`, making it the single task
continuity entrypoint. Full evidence remains under `task-archive/<run-id>/`.

## Capture Policy

Memory capture is append-only and conservative:

- Archive each host-generated completed-task receipt under `task-archive/` when
  memory capture is enabled.
- Refresh `task-memory.md` from recent local task archives, including a compact
  excerpt of the latest delivery report.
- Prefer references to receipt files over copying logs.
- Do not overwrite `context-capsule.md`.
- Do not automatically update project design purpose, long-term boundaries,
  core business positioning, or principle-level decisions.
- Do not store secrets, credentials, raw `.env` values, private tokens, or long
  code snippets.
- Do not turn every session into a new rule or skill automatically.
- Extract reusable workflow ideas as proposals first; humans decide whether to
  promote them into rules, profiles, or skills.

## Host and Memory Integration

The lifecycle is host-specific. A hook from one host is never accepted as
evidence for another host.

| Host | Start/read adapter | Close adapter |
|---|---|---|
| Claude Code | `SessionStart` command hook | `Stop` command hook |
| Codex | `SessionStart` command hook | `SessionEnd` command hook (maximum 3 seconds) |
| OMP | extension `session_start`, injected once on the next `before_agent_start` through `systemPromptAppend` | extension `agent_settled` / `session_shutdown` |

All adapters use the same bounded reader and conservative close/capture bridge:

- `scripts/context-memory-start.py` resolves the current repository and emits
  `hookSpecificOutput.additionalContext`. It never writes project memory.
- `scripts/claude-stop-memory-capture.py` is the compatibility-named,
  host-neutral close bridge. It accepts Claude, Codex, and OMP event envelopes.
  Every supported close event writes a small receipt under
  `$HOME/.agents/memory-close-receipts/<host>/`; its status is `captured`,
  `skipped`, or `failed`. A normal conversation without a canonical task card is
  recorded as `skipped`; it does not pollute task memory.
- Only a paired canonical task card plus valid delivery closure is archived and
  delegated to `context-memory.sh capture`.
- `scripts/ags-memory-lifecycle-omp.js` is the OMP native extension installed at
  `$HOME/.omp/agent/extensions/ags-memory-lifecycle.js`.
- `scripts/raw-tool-call-stop-guard.js` remains Claude-specific and independent
  from memory capture.

Preflight reports the exact requested host, adapter, and closure state.
`full` requires that host's native start and close wiring, backing scripts,
memory files, and archive directory. Unsupported hosts report `unsupported`;
they never inherit another host's result.

Command responsibilities:

- `ags setup --yes --force` installs or refreshes the shared scripts and OMP
  extension. The compatibility `--register-claude` path still reconciles Claude
  MCP plus current-workspace hooks.
- `ags agents govern --agent <claude-code|codex|omp> --apply` performs the
  explicit host-adapter write. It structurally preserves unrelated hooks,
  atomically replaces AGS-owned JSON or extension content, and bootstraps the current
  repository's memory store. External MCP registration remains advice-only.
- Codex migration removes only the retired AGS
  `UserPromptSubmit -> memory-start-context.sh` entry; memory then loads once at
  `SessionStart`. Other user hooks remain intact.
- `ags init` creates the per-project memory store (capsule, `task-memory.md`,
  `task-archive/`) and registers the project. Host adapters remain machine-level
  and repository-aware.
- `ags doctor` aggregates every detected supported host; one complete Claude
  chain cannot hide missing Codex or OMP wiring. `ags session preflight --for
  <host>` reports the requested host only.

Boundary notes:

- The runner (`ags run`) prepares a LaunchPlan and returns
  `HOST_EXECUTION_REQUIRED`; it does not execute, verify, write the task receipt,
  or generate a delivery report. The host owns post-execution memory, receipt,
  and delivery-report writes.
- The startup reader, close bridge, Claude/Codex hooks, and OMP extension are
  distinct adapters over the same project-memory authority.
- No capture path may overwrite `context-capsule.md`.

Use `--no-memory` only when a task run should intentionally skip local memory
capture.

## Resume Behavior

On "continue", context compression, or task-notification resume:

1. Reread the task card.
2. Read the memory capsule if the task card names one.
3. Read `task-memory.md` beside the capsule if present.
4. Read a named task archive if the task card names one.
5. Run `git status --short`.
6. Honor the card's permission mode before mutation: `plan-only` remains
   non-mutating and waits for a newly issued executable task card;
   `execute-and-verify` resumes execution and verification. Task level alone
   does not rewrite this authority.

Memory can provide continuity, but it is not approval for write operations.
