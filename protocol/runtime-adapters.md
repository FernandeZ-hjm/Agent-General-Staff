# Runtime Adapters

This file defines the generic execution fields used by Agent task cards and how
those fields map to specific agent runtimes.

Keep the task-card skeleton generic. Put runtime-specific behavior here.

## Purpose

The task card is an execution contract, not a Claude Code-only prompt. Cursor,
Codex, Claude Code, or another agent may execute the same contract when the
runtime adapter is explicit.

Use this file when generating, reviewing, or executing task cards that need to
state:

- who executes the task,
- where the task runs,
- what permissions are allowed,
- whether parallel work is allowed,
- what review gate is required,
- what evidence is required before claiming completion.

## Generic Fields

### Executor

Who performs the task.

Allowed values:

- `Codex`
- `Claude Code`
- `Cursor`
- `Human`
- `Other`

### Runtime Adapter

The mapping layer from generic task-card intent to runtime-specific behavior.

Allowed values:

- `codex-local`
- `claude-code`
- `cursor`
- `generic`

### Execution Surface

Where the task is executed.

Allowed values:

- `local-workspace` — local repository and shell access.
- `cli` — command-line agent runtime.
- `ide` — IDE-integrated agent runtime.
- `web` — browser-hosted or web app agent runtime.
- `remote-control` — local GUI or browser controlled through automation.
- `background-agent` — detached or long-running agent session.

### Permission Mode

What the executor may do without asking again.

Allowed values:

- `read-only` — inspect files, commands, docs, logs, and diffs only.
- `plan-only` — inspect and return a plan; do not edit files.
- `edit-with-confirmation` — propose or prepare edits, then wait at risk gates.
- `execute-and-verify` — edit within scope and run verification before delivery.
- `autonomous-low-risk` — perform low-risk edits and checks without extra prompts.

Use the narrowest permission mode that can complete the task safely.

Heavy tasks default to `plan-only`. They may move to
`edit-with-confirmation` or `execute-and-verify` only after explicit human
approval for the current task. Runtime capability, previous context, or a
continuation message is not approval.

### Parallelism

Whether the executor may split the work.

Allowed values:

- `none`
- `subagent`
- `worktree`
- `multi-session`
- `agent-team`

Default to `none`. Use parallelism only when the task can be split into bounded,
non-overlapping work with clear verification.

### Review Gate

The explicit review required before delivery or commit. Stop review hooks are
deprecated; they are no longer the final execution point or automatic blocking
mechanism. Task cards and explicit human action carry the review contract.

Required slot:

```markdown
Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
```

The selected task level determines the binding rule. The single canonical
mapping lives in `agent-task-protocol.md`; this file only defines where the
field appears and how runtime adapters preserve it.

### Verification Gate

The evidence required before claiming completion.

Required slots:

```markdown
Verification gate:
- commands:
- expected evidence:
- stop condition:
```

Examples of expected evidence:

- test command result,
- linter or syntax-check result,
- `git diff --stat`,
- generated report path,
- screenshot or browser check,
- dry-run output,
- delivery report.

Stop conditions must name when the executor should pause instead of continuing,
such as destructive action, baseline mutation, missing verification, unclear
scope, or risk higher than the task card declared.

## Default Profiles

### Codex Direct Execution

Use when the user asks Codex to execute directly.

```markdown
Executor: Codex
Runtime adapter: codex-local
Execution surface: local-workspace
Permission mode: execute-and-verify
Parallelism: none
Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
Verification gate:
- commands: <narrowest relevant verification command>
- expected evidence: changed files + command result + residual risk note
- stop condition: destructive action / unclear scope / risk escalation
```

### Claude Code Handoff

Use when generating a task card for Claude Code.

```markdown
Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: edit-with-confirmation
Parallelism: none
Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
Verification gate:
- commands: <task-card-specified commands>
- expected evidence: delivery report + command result + git diff summary
- stop condition: risk higher than task card / baseline mutation / missing verification
```

### Cursor Execution

Use when Cursor is expected to execute inside an IDE workflow.

```markdown
Executor: Cursor
Runtime adapter: cursor
Execution surface: ide
Permission mode: edit-with-confirmation
Parallelism: none
Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
Verification gate:
- commands: <project-specific checks>
- expected evidence: changed files + command result + short delivery summary
- stop condition: broad refactor / destructive action / unclear scope
```

### High-Risk Planning

Use for Heavy tasks before mutation.

```markdown
Executor: <Codex / Claude Code / Cursor>
Runtime adapter: <runtime>
Execution surface: <surface>
Permission mode: plan-only
Parallelism: none
Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
Verification gate:
- commands: read-only audit first
- expected evidence: root cause + implementation plan + verification plan
- stop condition: before mutation, wait for user confirmation
```

## Adapter Selection Rules

Choose the adapter before filling task-specific details.

| User request | Executor | Runtime adapter | Execution surface |
|---|---|---|---|
| "you execute", "你直接做", "你来改" | `Codex` | `codex-local` | `local-workspace` |
| "give me a Claude Code task card", "给 Claude Code 任务卡" | `Claude Code` | `claude-code` | `cli` |
| "Cursor execute", "给 Cursor 任务卡" | `Cursor` | `cursor` | `ide` |
| "human checklist", "我自己执行" | `Human` | `generic` | `local-workspace` |
| external or unknown agent | `Other` | `generic` | choose the narrowest known surface |

If the user does not name an executor, follow the project operating protocol.
For this suite's default collaboration model, Codex or Cursor frames the task
and Claude Code may execute bounded implementation work.

## Task-Level Defaults

Use task level to choose the initial permission mode. Do not escalate permission
just because the runtime supports it.

| Task level | Default permission | Default parallelism | Confirmation behavior |
|---|---|---|---|
| Light | `execute-and-verify` | `none` | no extra confirmation unless stop condition triggers |
| Medium | `edit-with-confirmation` | `none` | give short root cause or design note, then execute if scope is clear |
| Heavy | `plan-only` | `none` | return root cause, design, implementation plan, and verification plan; wait for explicit human approval before mutation |

Upgrade to `plan-only` when any of these are true:

- baseline data, vector stores, databases, migrations, or historical outputs are involved,
- the task could delete, overwrite, reinstall, publish, or irreversibly mutate state,
- the user asks for an audit, dry-run, staged rollout, or traceable decision,
- the executor sees risk higher than the task card declared.

Use `autonomous-low-risk` only when all are true:

- the task is Light,
- the affected files are scoped and non-sensitive,
- verification is cheap and explicit,
- no destructive command, network publish, dependency install, or baseline mutation is needed.

## Resume / Compression Recovery Rules

When the session resumes from "continue", context compression,
task-notification, or a background-agent handoff:

- Heavy executors must reread the task card, run `git status --short`, and
  reconfirm `review_targets`.
- If the current context does not contain explicit human approval for mutation,
  the executor must stop at the plan / confirmation gate.
- "Continue", a resume notification, an earlier plan, or a compressed summary
  must not be treated as approval to write for a Heavy task.
- If the task card, target repositories, or approval state cannot be confirmed,
  stop and report instead of executing.

## Execution Surface Rules

| Surface | Use when | Avoid when |
|---|---|---|
| `local-workspace` | local files, shell commands, tests, scripts, docs | browser session or remote UI is the source of truth |
| `cli` | command-line agent runtime or generated terminal prompt | IDE state or user profile/session is required |
| `ide` | Cursor or editor-native workflows | headless automation is required |
| `web` | browser-hosted agent or web-only interface | local repo mutation is required but unavailable |
| `remote-control` | GUI or browser automation is required | the task can be handled by file or CLI APIs |
| `background-agent` | detached long-running agent work | user needs immediate interactive confirmation |

## Parallelism Rules

Default to `none`.

Allow `subagent` when:

- the side task is read-only or has a disjoint write scope,
- the parent executor can review the result,
- the result is useful but not on the immediate critical path.

Allow `worktree` when:

- implementation can be isolated from the current working tree,
- branches or worktrees are acceptable in the project,
- the task card states how to verify and merge or discard the result.

Allow `multi-session` only when:

- each session has a non-overlapping scope,
- the owner is explicit,
- there is a final integration and verification step.

Allow `agent-team` only when the user explicitly requests experimental
multi-agent execution or the runtime has a documented team mode. Treat it as
unsupported for generic agents.

## Permission Downgrade Rules

The executor must downgrade permission and stop when:

- the workspace has unrelated dirty changes that affect the target files,
- required files or docs are missing,
- verification commands are unavailable or unsafe,
- the task requires secrets or credentials,
- the task would mutate protected data, generated baselines, or external state,
- the requested runtime cannot express the declared permission mode safely.

When downgrading, report the current evidence and the narrowest safe next step.

## Runtime Mappings

### `codex-local`

Use for Codex executing in the current local workspace.

Permission mapping:

- `read-only`: inspect files, docs, command output, and diffs only; do not edit.
- `plan-only`: return diagnosis, options, or implementation plan; do not edit.
- `edit-with-confirmation`: explain intended edits before applying them when the
  task is Medium or higher, or when the user asked to approve the plan first.
- `execute-and-verify`: edit scoped files, run targeted verification, and report
  changed files, verification evidence, and residual risk.
- `autonomous-low-risk`: allowed only for small local edits with obvious checks.

Execution notes:

- Use existing repository tools and patterns before introducing new ones.
- Check `git status --short` before editing.
- Do not revert unrelated dirty changes.
- Run the narrowest meaningful verification before claiming completion.
- Stop review hooks are deprecated. Codex keeps only the `UserPromptSubmit`
  skill-alias sync hook, while Claude Code keeps skill-alias sync plus the
  `PreToolUse(Bash)` RTK hook. Reviews are explicit task-card or human gates.
- Optional hard blocking is a future stage. If enabled later, it must be an
  explicit option such as `scripts/configure-review-hooks.mjs
  --enable-stop-review-gate` or a suite-owned runner gate that checks an
  approval marker for Heavy resume. Do not rely on `.claude/task_level` as a
  current Codex plugin blocking mechanism.

Parallelism mapping:

- `subagent`: only when the user explicitly asks for delegated agent work.
- `worktree`: create or use a worktree only when the user asks or the task card
  explicitly requires isolation.
- `multi-session` and `agent-team`: treat as unsupported unless the user provides
  a concrete runtime.

### `claude-code`

Use for task cards intended to be pasted into or launched with Claude Code.

Permission mapping:

- `read-only`: inspect repository state and report findings only.
- `plan-only`: use Claude Code plan mode, such as `--permission-mode plan` when
  launched from CLI.
- `edit-with-confirmation`: use the normal interactive mode and stop at declared
  risk gates.
- `execute-and-verify`: edit within task-card scope and run verification before
  the delivery report.
- `autonomous-low-risk`: use only when the task card explicitly allows low-risk
  autonomous execution.

Execution notes:

- The task card should name required project docs and paths instead of repeating
  all fixed protocol text.
- The task card should include exact verification commands when known.
- Claude Code must output the delivery report required by the project protocol.
- For Heavy tasks, start with `plan-only` and wait before mutation.
- On Heavy resume, reread the task card, `git status --short`, and
  `review_targets`; if mutation approval is not explicit in the current
  context, stop in plan mode.

Parallelism mapping:

- `subagent`: use Claude Code subagents for bounded investigation or side tasks.
- `worktree`: use git worktrees for independent implementation branches.
- `multi-session`: use multiple Claude Code sessions only with separate scopes.
- `agent-team`: use only for explicitly approved experimental multi-agent work.

### `cursor`

Use for Cursor or an IDE-native agent executing the task.

Permission mapping:

- `read-only`: inspect code, docs, and diffs without editing.
- `plan-only`: use planning or chat mode and return a plan.
- `edit-with-confirmation`: use agent mode, but stop at task-card risk gates.
- `execute-and-verify`: edit scoped files and run verification commands.
- `autonomous-low-risk`: allowed only for small local edits with obvious checks.

Execution notes:

- Keep task-card facts project-local; do not bake global suite internals into
  project-specific prompts.
- Use IDE context only as supporting evidence; final claims still need commands,
  diffs, screenshots, or other explicit evidence.
- Stop before broad refactors unless the task card explicitly authorizes them.

Parallelism mapping:

- `subagent`: use only if the Cursor environment has an equivalent delegated
  agent mechanism.
- `worktree`: require explicit task-card authorization.
- `multi-session` and `agent-team`: require explicit task-card authorization.

### `generic`

Use when the runtime is unknown or external.

- Prefer `read-only` or `plan-only` unless the task card is self-contained.
- Do not assume tool-specific commands, hooks, worktrees, or agent teams exist.
- State required evidence in generic terms.
- Ask the user to choose a runtime adapter before write operations when the risk
  is Medium or Heavy.

## Task-Card Authoring Rules

- Keep generic fields in the task card.
- Put tool-specific command hints in `Runtime adapter` notes or this file.
- Do not create separate full task-card templates for each tool.
- Do not encode one machine's paths as adapter defaults.
- If the user names a tool, select the matching runtime adapter.
- If the user says "you execute", use `codex-local`.
- If the user asks for a Claude Code prompt or task card, use `claude-code`.
- If the user asks for Cursor execution, use `cursor`.
- If no executor is specified, follow the project operating protocol.

## Runner Auto Mode

`scripts/run-task-card.sh --auto` is a conservative orchestration layer. It reads
task-card fields and resolves runner flags without changing the task card:

- `Runtime adapter: claude-code` or `Executor: Claude Code` -> enable `--claude`.
- `Execution surface: background-agent` -> enable `--claude --headless`.
- `Parallelism: subagent | multi-session | agent-team` -> enable `--claude --parallel`.
- `Parallelism: worktree` -> enable `--claude --parallel --worktree`.

Auto mode never upgrades `Permission mode`; Heavy tasks still default to
`plan-only` and require explicit current-task approval before mutation.
