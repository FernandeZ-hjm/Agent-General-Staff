# Context Memory

Context memory provides cross-conversation continuity for Codex, Cursor, and
Claude Code without changing the cache-stable task-card skeleton.

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

Before any memory exists this can be `无`. Runner execution refreshes
`task-memory.md` by default, making it the single automatically refreshed task
continuity entrypoint. Full evidence remains under `task-archive/<run-id>/`.

## Capture Policy

Memory capture is append-only and conservative:

- Archive each `ags run` receipt under `task-archive/` when memory capture is
  enabled.
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

## Runner Integration

Task-start context read:

- At task start, the executing agent reads `context-capsule.md` and
  `task-memory.md` for the current repository when the task card names them.
- This read is read-only and must not write memory files.
- Automatic injection through host `UserPromptSubmit` hooks is planned (not yet
  implemented). Until then, the task-start read is driven by AGS preflight and
  the executor's documented startup steps, not by an installed prompt hook.

Task-end capture:

The paste-to-Claude `Stop`-hook capture path is **implemented** as a first-class
product mechanism. The canonical scripts live in the suite:

- `scripts/raw-tool-call-stop-guard.js` — a Claude Code `Stop` hook guard that
  catches raw tool-call markup leaks before the turn ends.
- `scripts/context-memory.sh` — `status` / `init` / `capture RECEIPT_DIR`.
  `init` creates the project memory store and capsule (create-if-missing);
  `capture` archives a receipt under `task-archive/` and refreshes
  `task-memory.md`. It never overwrites `context-capsule.md`.
- `scripts/claude-stop-memory-capture.py` — a Claude Code `Stop` hook that reads
  the transcript, detects a pasted task card plus its delivery report, builds a
  local receipt package, and delegates the write to `context-memory.sh capture`.
  It is conservative: no task card → skip; no delivery report → skip; duplicate
  transcript → skip; the capsule is never written directly.

Command responsibilities:

- `ags setup --yes --register-claude` installs the raw guard plus both memory
  scripts to
  `$HOME/.agents/scripts/`, merges the capture step into the current AGS
  workspace's Claude `Stop` pipeline (order: raw guard → project memory capture)
  while preserving existing hooks, and
  bootstraps the workspace capsule via `context-memory.sh init`.
- `ags init` creates the per-project memory store (capsule, `task-memory.md`,
  `task-archive/`) and registers the project. It does **not** install host
  hooks: the installed capture bridge is cwd-aware and resolves each project's
  memory by repository, so one host-level hook serves every onboarded project.
- `ags doctor` reports the chain state: capture scripts present, Stop hook
  wired, raw guard preserved, `task-memory.md` freshness, and capsule
  design-purpose integrity.

Boundary notes:

- The receipt-first runner (`ags run`) writes the task receipt and delivery
  report. `scripts/run-task-card.sh` is a thin wrapper that delegates planning
  to `ags run` (`--check-only`, `--dry-run`, `--approve-writes`,
  `--format text|json`); it does not itself copy a receipt package.
- A transient compile/learning pipeline and `learning-gaps/` proposal capture
  remain planned (not yet implemented). When added, any such proposals must not
  be promoted into rules without human review.
- The capture path must not overwrite `context-capsule.md`.

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
