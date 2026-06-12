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

## Context vs Evolution Memory

Context memory owns project truth.
Evolution memory owns reusable method.
If they conflict, context memory wins.

`context-capsule.md` is the project charter and is not modified by Evolver.
`task-memory.md` stores task continuity facts and is not written by Evolver.
`task-archive/` stores evidence and is not reinterpreted by Evolver as project
experience.

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

- Archive each runner receipt under `task-archive/` when memory capture is
  enabled.
- Refresh `task-memory.md` from recent local task archives, including a compact
  excerpt of the latest delivery report.
- Store Learning Runner coverage gaps under `learning-gaps/` when runner
  detects reusable misses; do not store transient Task IR or compiled briefs
  there.
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

Task-start hook:

- `scripts/memory-start-context.sh` reads `context-capsule.md` and
  `task-memory.md` for the current repository.
- It is read-only and must not write memory files.
- It is installed into both Claude Code and Codex `UserPromptSubmit` runtime
  hooks by `scripts/bootstrap.sh --apply`.

Task-end capture:

- `scripts/run-task-card.sh` copies the receipt package into `task-archive/`
  and refreshes `task-memory.md` after the receipt and delivery report are
  written.
- `scripts/run-task-card.sh` also runs a transient compile/learning pipeline by
  default. It may write small `learning-gaps/` proposals, but it must not
  promote those proposals into rules without human review.
- `scripts/claude-stop-memory-capture.py` covers the paste-to-Claude workflow:
  when a Claude Code `Stop` hook sees both a task card and a delivery report in
  the transcript, it builds a local receipt and delegates to
  `scripts/context-memory.sh capture`.
- The runner prints the final `delivery-report.md` to the foreground only after
  memory capture succeeds, so the displayed report is already represented in
  local task memory.
- Neither capture path overwrites `context-capsule.md`.

Use `--no-memory` only when a task run should intentionally skip local memory
capture.

## Resume Behavior

On "continue", context compression, or task-notification resume:

1. Reread the task card.
2. Read the memory capsule if the task card names one.
3. Read `task-memory.md` beside the capsule if present.
4. Read a named task archive if the task card names one.
5. Run `git status --short`.
6. For Heavy tasks, stop at the confirmation gate unless current-context
   mutation approval is explicit.

Memory can provide continuity, but it is not approval for write operations.
