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

## Lifecycle Boundary: Preflight / Solution Are Not Runner Execution

The fields defined in this file (Permission mode, Parallelism, Execution
surface, launch args) apply ONLY after a task card has been formed from a
confirmed execution contract. They do NOT govern the earlier lifecycle phases:

- **Ambient preflight** (project detection, context reading, git status) is a
  read-only discovery phase. It runs before any task card exists and is not
  constrained by Permission mode or Parallelism.
- **Solution phase** (understanding, diagnosis, solution formation, user
  confirmation) is a framing phase performed by Codex / Cursor. It produces the
  execution contract that will become the task card input. It is not runner
  execution.

Only after the execution contract is formed, the user has explicitly issued a
task-card instruction, and the task card is compiled do the runtime adapter
fields take effect. The runner, resolver, and gate operate on the task card —
they do not govern how Codex / Cursor reach the solution.

**Value Route is not a runtime adapter field.** The Value Route recommendation
(`read-only-advisory` / `direct-edit` / `plan-first` / `claude-code-route` /
`stop-for-scope`; see `protocol/agent-task-protocol.md` §3.9) is an advisory
solution-phase signal about the execution-path *form*. It is distinct from
`Runtime adapter`, `Permission mode`, and `Execution surface`, and it never sets
or changes them. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients;
like any host other than `codex` / `claude-code` / `cursor` they map to
`Runtime adapter: generic` (M9 caps permission at `plan-only` without explicit
approval). Value Route does not change that mapping.

Two distinct layers must not be conflated. The `default_permission_mode` reported
by `ags agent instructions` / `ags session preflight` (e.g. `edit-with-confirmation`
for governed and generic hosts) is the host's **interactive discovery baseline** —
descriptive metadata surfaced during preflight, not an enforced write gate (AGS
MCP is read-only and stateless). The **enforced** write gate is the
execution-policy resolver acting on a task card's `Runtime adapter` field: M9 caps
`generic` at `plan-only` without explicit approval. A Tencent Agent client
(WorkBuddy / CodeBuddy-Code) carried as a generic host therefore still has its
actual task-card writes gated at `plan-only` by M9, regardless of the discovery
baseline shown in agent instructions.

**Task-card request gate**: Between "solution OK" and routing/task card generation
there is a hard gate. `ags task compile` requires `--task-card-requested` before
it will output an executable task card. Without this flag, `executable_allowed`
is `false` and `block_reason` is `task_card_not_requested`. Codex/Cursor must
only pass this flag after the user has explicitly issued a task-card instruction
(\"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.).
\"方案 OK\" alone is not sufficient.

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
- `autonomous-low-risk` — reserved protocol target for future low-risk edits.
  Do not generate or execute task cards with this value until the Rust
  task-card-validator accepts it.

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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
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
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
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

`autonomous-low-risk` is reserved but not currently valid for generated task
cards. Generators must use `execute-and-verify` for Light tasks until the Rust
canonical gate implements the required Light-only, protected-path, and Heavy
prohibition checks.

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

## Receipt-First Execution

Receipt-first execution is a logging and foreground-interaction policy, not a
new workflow family and not a permission mode.

Use it when the executor should keep detailed process evidence in the runner
receipt package while keeping the foreground context limited to:

- phase summaries,
- explicit approval prompts,
- stop conditions,
- final delivery report pointers.

Receipt-first execution must not replace the task card, Review gate,
Verification gate, or Heavy confirmation rules. A user clicking approval
prompts is approval only for the specific action described in that prompt, not
a standing approval for unrelated writes, destructive commands, or scope
expansion.

Recommended receipt artifacts:

- `process-summary.md` for phase summaries, notable decisions, and approval
  points,
- `claude-output.log` for headless Claude Code output,
- `verification.log` for runner Verification gate command output,
- `delivery-report.md` for the final acceptance record.

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
- `autonomous-low-risk`: reserved; do not use until the Rust canonical gate
  accepts it.

Execution notes:

- Use existing repository tools and patterns before introducing new ones.
- Check `git status --short` before editing.
- Do not revert unrelated dirty changes.
- Run the narrowest meaningful verification before claiming completion.
- Stop review hooks are deprecated. Codex keeps `UserPromptSubmit` skill-alias
  sync and memory-context hooks, while Claude Code keeps those hooks plus the
  `PreToolUse(Bash)` RTK hook and the non-blocking Stop memory-capture hook.
  Reviews are explicit task-card or human gates.

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
- `autonomous-low-risk`: reserved; do not use until the Rust canonical gate
  accepts it.

Execution notes:

- The task card should name required project docs and paths instead of repeating
  all fixed protocol text.
- The task card should include exact verification commands when known.
- Claude Code must output the delivery report required by the project protocol.
- For Heavy tasks, start with `plan-only` and wait before mutation.
- On Heavy resume, reread the task card, `git status --short`, and
  `review_targets`; if mutation approval is not explicit in the current
  context, stop in plan mode.
- When launched with runner receipt-first mode, keep verbose process logs in
  the receipt package and keep foreground output to phase summaries, approval
  prompts, stop conditions, and delivery-report pointers.

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
- `autonomous-low-risk`: reserved; do not use until the Rust canonical gate
  accepts it.

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

## Execution-Policy Resolver

The `execution-policy` crate (`crates/execution-policy/`) is the resolver that
reads a validated task card and produces a structured resolution of **how** the
task should actually execute — what launch args to use, what to downgrade, and
whether to stop before launch.

### Relationship with validator and runner

```
task card text
      │
      ▼
┌─────────────────┐
│ task-card-       │  "Is this task card valid?"
│ validator        │  → pass / fail + error list
│ (hard gate)      │
└─────────────────┘
      │ (pass)
      ▼
┌─────────────────┐
│ execution-policy │  "How should this valid task card execute?"
│ resolver         │  → ResolvedExecutionPolicy
│ (read-only)      │    (launch args, downgrades, stop reasons)
└─────────────────┘
      │
      ▼
┌─────────────────┐
│ runner           │  Actually launches the executor
│ (future /        │  with resolved policy
│  scripts/)       │
└─────────────────┘
```

The validator is a **hard gate** — an invalid task card must be fixed before
proceeding.  The execution-policy resolver is a **soft resolution layer** — it
takes a valid task card and may downgrade permission or parallelism, but it
never rejects a valid card; it only adjusts the launch strategy and records why.

### Key resolution rules

The resolver enforces the following MUST rules (canonical rule IDs M1–M10).
> **命名空间说明**: 以下 M1-M10 是 execution-policy 规则编号，与 Roadmap M0-M8
> 里程碑编号在不同的命名空间中，互不相关。

| Rule | Description |
|---|---|
| M1–M3 | `ultracode` is thinking intensity only. It does **not** change permission mode, enable parallelism, or inject any launch arg. |
| M4 | Heavy tasks without `explicit_write_approval` are downgraded to `plan-only` and require a confirmation gate. |
| M5–M6 | `read-only` / `plan-only` effective permission modes must **never** produce write-type launch args. Active parallelism flags (`--parallel`, `--worktree`) and `--headless` are stripped. |
| M7 | `subagent`, `multi-session`, `agent-team` require Workflow authority `within-card` or `allowed`. `worktree` requires Workflow authority **not** `none`. |
| M8 | Every downgrade records a structured `DowngradeReason` with the before/after values and the triggering rule.  The `downgrade_reasons` list provides a full audit trail. |
| M9 | `generic` runtime adapter caps permission at `plan-only` without explicit approval. |
| M10 | Every downgrade records a structured reason. No downgrade = no reason entries. |

### CLI

Validate and resolve execution policy in one command:

```bash
ags policy resolve <task-card> --format text|json [--approve-writes]
```

The old `ags resolve-policy` is kept as a hidden backward-compatible alias.

The command runs the canonical task-card validator first; on validation failure
it prints errors to stderr and exits 1.  On success it outputs the resolved
policy in text or JSON format.  It is **read-only** — it never launches a
runner.

The optional `--approve-writes` flag sets `approval_source` to `cli-flag`,
allowing Heavy tasks to retain write-mode permissions without downgrade.
Without this flag (or a runner environment override), Heavy tasks are always
downgraded to `plan-only`.

### Default semantics

| Input field | When absent | Resolved value |
|---|---|---|
| `Execution effort:` | absent or empty | `"unknown"` |
| `Workflow authority:` | absent or empty | `"none"` |
| `approval_source` | (not in task card fields) | `none` |

The resolver does not accept `approval_source` from the task card text — only
an explicit CLI flag (`--approve-writes` → `cli-flag`) or runner environment
override (`AGS_APPROVE_WRITES=1` → `runner-env`) can set it to an approved
state.  Task card text is **never** an approval source.

### Stop vs confirmation gate

The resolver has two distinct launch-blocking mechanisms:

| Mechanism | Meaning | Runner behavior |
|---|---|---|
| `stop_before_launch=true` | Do **not** launch at all. | Runner must refuse to start. Task card must be rewritten or approval obtained. |
| `requires_confirmation_gate=true` | Launch but present a confirmation prompt before mutation. | Runner launches in plan mode, presents plan, waits for human approval before editing. |

`stop_before_launch` is set when:
- A Heavy task requests mutation without explicit write approval.
- Active parallelism (subagent, worktree, multi-session, agent-team) is
  requested but the effective permission mode forbids writes — the
  parallelism flags would create filesystem side effects incompatible
  with `read-only` or `plan-only`.
- `background-agent` execution surface is requested but the effective
  permission mode forbids writes — headless background execution could
  have side effects (process spawning, resource consumption) incompatible
  with `read-only` or `plan-only`.  The resolver records an M5 downgrade
  on `execution_surface` (before=`background-agent`, after=`cli`) and
  sets `stop_before_launch=true` with stop reason entry kind
  `background-surface-blocked-by-permission`.

`requires_confirmation_gate` is set for all Heavy tasks, regardless of
downgrade status.

### Execution surface values

The validator now accepts the full set of protocol-defined execution surface
values: `local-workspace`, `cli`, `ide`, `web`, `remote-control`,
`background-agent`.  All six values are aligned between validator, protocol,
and resolver.

### Resolved policy JSON schema

`ags policy resolve <task-card> --format json` is the stable machine contract
for runners.  The text format is human-readable diagnostics only; runners MUST
consume JSON.

| Field | Type | Stable values / semantics |
|---|---|---|
| `executor` | string | Validated task-card executor. |
| `runtime_adapter` | string | Validated task-card runtime adapter. |
| `effective_permission_mode` | string | `read-only`, `plan-only`, `edit-with-confirmation`, `execute-and-verify`. This is the only permission mode a runner may use. |
| `effective_parallelism` | string | `none`, `subagent`, `worktree`, `multi-session`, `agent-team`. This is the resolved value after authority and writability gates. |
| `effective_execution_surface` | string | `local-workspace`, `cli`, `ide`, `web`, `remote-control`, `background-agent`. If `background-agent` is blocked by `read-only` / `plan-only`, this becomes `cli`. |
| `allowed_launch_args` | string array | The exact runner CLI args allowed for launch. Runner MUST pass them verbatim and MUST NOT synthesize additional args from raw task-card fields. |
| `stop_before_launch` | boolean | If `true`, runner MUST refuse to launch. |
| `stop_reasons` | object array | Canonical stop reasons. This plural array is the only stop-reason field; legacy singular `stop_reason` MUST NOT be emitted or consumed. |
| `was_downgraded` | boolean | Whether any field was downgraded from the input card. |
| `downgrade_reasons` | object array | Full audit trail; each entry has `rule_id`, `field`, `before`, `after`, `reason`. |
| `requires_confirmation_gate` | boolean | If `true` and launch is not stopped, runner must present a confirmation gate before mutation. |
| `execution_effort` | string | Declared effort, defaulting to `unknown` when absent. |
| `is_exhaustive_mode` | boolean | `true` only for `Execution effort: ultracode`; it never grants permission or parallelism. |
| `approval_source` | string | `none`, `cli-flag`, or `runner-env`. Task-card text is never an approval source. |

Stopped policy invariant:

- If `stop_before_launch=true`, `allowed_launch_args` MUST be `[]`.
- Runners MUST check `stop_before_launch` before consuming any launch args.
- If `stop_before_launch=true`, `requires_confirmation_gate` does not authorize
  a launch; stop wins.

## Runner Auto Mode

`scripts/run-task-card.sh --auto` is a conservative orchestration layer. It MUST
consume the **resolved execution policy** from `ags policy resolve` to determine
launch flags — it must NOT derive flags directly from raw task-card fields.

### Correct auto-mode flow

```
task card
    │
    ▼
ags policy resolve --format json           ◄── resolver enforces M1–M10
    │
    ▼
run-task-card.sh --auto                    ◄── reads resolved policy JSON
    │  reads .effective_permission_mode
    │  reads .effective_parallelism
    │  reads .allowed_launch_args            ◄── authoritative, already gated
    │  reads .stop_before_launch
    │  reads .requires_confirmation_gate
    │  reads .runtime_adapter
    │
    ▼
launch or stop
```

### Why resolver-first

The resolver enforces M5/M6: `read-only` and `plan-only` effective permission
modes must never produce write-type launch args or active parallelism flags.
If the auto-mode runner reads raw task-card fields directly, it can produce
`--parallel`, `--worktree`, or `--headless` flags for a `read-only` card —
bypassing the resolver's writability gate entirely.

The auto-mode runner MUST:

1. Run `ags policy resolve <task-card> --format json` (with `--approve-writes`
   if the invoking context carries explicit approval).
2. Check `stop_before_launch` — if `true`, refuse to launch and surface all
   `stop_reasons` entries to the caller. Multiple independent gates can stop
   the same launch attempt.
3. Use `allowed_launch_args` verbatim as the CLI arguments for the runner.
4. If `requires_confirmation_gate` is `true`, present a confirmation prompt
   before enabling mutation in the runner session.
5. Never read `Parallelism:`, `Execution surface:`, or `Permission mode:`
   from the raw task card to decide launch flags — those values have already
   been resolved, downgraded, and gated by the resolver.

### Example: resolved policy → runner flags

Given a task card with `Permission mode: read-only`, `Parallelism: worktree`:

```json
{
  "effective_permission_mode": "read-only",
  "effective_parallelism": "none",
  "allowed_launch_args": [],
  "stop_before_launch": true,
  "stop_reasons": [
    { "kind": "writable-parallelism-blocked-by-permission", ... }
  ]
}
```

The auto-mode runner sees `stop_before_launch: true`, refuses to launch, and
reports the stop reason entries. It never generates `--parallel --worktree`,
and it receives no launch args at all because stopped policies expose
`allowed_launch_args: []`.

### Defaults preserved

Auto mode never upgrades `Permission mode`; Heavy tasks still default to
`plan-only` and require explicit current-task approval before mutation.
Receipt-first execution remains an explicit runner flag; auto mode does not
enable it implicitly.

## Learning Runner

`scripts/run-task-card.sh` is learning-enabled by default. Before launching
Claude Code, the runner validates the canonical task card, compiles a transient
Task IR / compiled brief, and injects the brief as an execution guardrail.

Rules:

- Task IR and compiled brief are not task-card formats.
- They must not be pasted into, appended to, or required as part of the
  canonical task-card skeleton.
- They are temporary by default and are deleted after the run.
- `--keep-ir` may retain them in the receipt package for compiler debugging.
- `--no-learning` disables the transient compile and learning-gap extraction
  for a run.
- Long-term retention is limited to `learning-gaps/` entries under local
  project memory when the runner detects reusable misses such as weak
  verification, executor delivery failure, nonzero execution, or compiler
  coverage gaps.
- Learning gaps are review proposals. They do not automatically update
  `context-capsule.md`, task-card templates, protocol files, validator rules, or
  project profiles.
