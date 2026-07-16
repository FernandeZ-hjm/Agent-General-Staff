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

## Lifecycle Boundary: Request Routing Is Not Runner Execution

The fields defined in this file (Permission mode, Parallelism, Execution
surface, launch args) govern task-card handoff execution. They do not govern
the earlier lifecycle phases, bounded `direct-response`, or authorized
host-native `direct-edit`:

- **Ambient preflight** (project detection, context reading, git status) is a
  read-only discovery phase. It runs before any task card exists and is not
  constrained by Permission mode or Parallelism.
- **Request Router** is the only natural-language routing node. Its structured
  `RequestDecision` selects `DirectResponse`, `SkillDemand`, and/or one
  business-level `MachineCli` capability before task-level classification.
- **Solution phase** (understanding, diagnosis, solution formation, user
  confirmation) is conditional framing performed by Codex / Cursor only while
  material decisions remain unresolved. An already approved contract is reused
  instead of being designed again.
- **Direct edit** is host-native same-session execution only when an approved
  contract and an explicit live modification instruction are both present. It
  does not compile a task card, but still obeys independent protected-path,
  release, review, and verification boundaries. An unresolved or reopened
  design returns to solution formation.

Only after the user explicitly requests a task-card handoff and the card is
compiled do runtime-adapter fields take effect. The runner, resolver, and gate
operate on that card; they do not govern solution formation or direct edit.

**RequestDecision is not a runtime adapter field.** It is distinct from
`Runtime adapter`, `Permission mode`, and `Execution surface`, and never sets or
changes them. WorkBuddy and CodeBuddy-Code are Tencent Agent host clients;
like any host other than `codex` / `claude-code` / `cursor` they map to
`Runtime adapter: generic` (M9 caps permission at `plan-only` without explicit
approval). Request routing does not change that mapping.

Two distinct layers must not be conflated. The `default_permission_mode` reported
by `ags agent instructions` / `ags session preflight` (for example,
`execute-and-verify` for governed hosts) is the host's **interactive discovery baseline** —
descriptive metadata surfaced during preflight, not an enforced write gate (AGS
MCP is read-only and stateless). The **enforced** write gate is the
execution-policy resolver acting on a task card's `Runtime adapter` field: M9 caps
`generic` at `plan-only` without explicit approval. A Tencent Agent client
(WorkBuddy / CodeBuddy-Code) carried as a generic host therefore still has its
actual task-card writes gated at `plan-only` by M9, regardless of the discovery
baseline shown in agent instructions.

**Task-card handoff gate**: `ags task compile` requires both
`--task-card-requested` and `--confirmed-handoff-contract` before it will output
a handoff task card. Missing request evidence reports
`block_reason=task_card_not_requested`; missing structured contract evidence
reports `block_reason=handoff_contract_not_confirmed`; reopened solution work
reports `solution_formation_required`. This compiler gate does not apply to
authorized same-session `direct-edit`; \"方案 OK\" alone authorizes neither path.

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

- `plan-only` — inspect, diagnose, audit, or return a plan; do not edit files.
- `execute-and-verify` — edit within scope and run verification before delivery.

Use the narrowest permission mode that can complete the task safely.

Task level does not change the permission mode. Task level (Light / Medium /
Heavy) is a risk/review tier; the permission mode is the execution authority.
The only permission modes are plan-only and execute-and-verify;
execute-and-verify runs directly and includes verification. A Heavy task keeps
its declared permission mode — it is never downgraded by task level. Heavy adds
its independent review gate, not an extra planning round. When a
Heavy card does not declare a permission mode, the compiler fills the
conservative `plan-only` default (an explicit permission mode is always
preserved). Runtime capability, previous context, or a continuation message
cannot rewrite the task card's authority or skip the review gate.

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
Permission mode: execute-and-verify
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
Permission mode: execute-and-verify
Parallelism: none
Review gate:
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。
Verification gate:
- commands: <project-specific checks>
- expected evidence: changed files + command result + short delivery summary
- stop condition: broad refactor / destructive action / unclear scope
```

### High-Risk Planning

Use only when the confirmed Heavy contract is a planning/audit pass.

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
- stop condition: stop without mutation; later implementation requires an execute-and-verify task card
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

Task level is a risk/review tier, not the execution authority. The table below is
the compiler's permission default *when a card omits `Permission mode:`* — it does
not override an explicitly declared permission. Do not escalate permission just
because the runtime supports it.

| Task level | Default permission (when unspecified) | Default parallelism | Execution behavior |
|---|---|---|---|
| Light | `execute-and-verify` | `none` | execute and verify directly; independent stop conditions still apply |
| Medium | `execute-and-verify` | `none` | give a short root cause or design note when useful, then execute and verify without pausing |
| Heavy | `plan-only` | `none` | default only when unspecified. Heavy plan returns root cause, design, implementation plan, and verification plan without writing; explicit Heavy `execute-and-verify` runs and verifies directly and still requires the independent Heavy review gate |

Select `plan-only` when any of these are true:

- the confirmed contract asks only for diagnosis, an audit, a dry-run report, or
  a plan,
- implementation authority is absent or the writable scope is unresolved,
- the executor sees risk outside the task card and must stop for a revised
  execution contract.

Data, migration, deletion, publishing, credential, external-write, and other
protected operations remain independent stop conditions. They do not create a
third permission mode.

## Resume / Compression Recovery Rules

When the session resumes from "continue", context compression,
task-notification, or a background-agent handoff:

- Heavy executors must reread the task card, run `git status --short`, and
  reconfirm `review_targets`.
- The executor must honor the confirmed task card's `Permission mode`:
  `plan-only` remains non-mutating, while `execute-and-verify` resumes execution
  and verification.
- "Continue", a resume notification, an earlier plan, or a compressed summary
  must not be treated as authority to rewrite the permission mode.
- If the task card, target repositories, permission mode, or review state cannot
  be confirmed, stop and report instead of executing.

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
Verification gate, or independent stop conditions. A user clicking approval
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

## Subtask Scope Rules

`子任务编排` declares a splittable structure; it never itself fires a subagent or
workflow (the claude-code adapter / runner translates it under the resolved
policy). When subtasks ARE used, their scope is restricted:

- Subtasks may contain ONLY parallelizable work: read-only audit / analysis,
  bounded implementation, documentation sync, or test addition.
- The following MUST stay with the main executor and may NOT be delegated to a
  subtask: the final verification run, the delivery report, `git` commit / push,
  and any release gate.
- All subtask results merge into a single diff; the main executor then runs the
  unified verification, reads the full output, and writes the delivery report.

Rationale: only the main executor has continuity across all phases and can make
coherent verification / commit / delivery decisions. Subtasks are
work-generation units; their output is material the main executor integrates,
verifies, and delivers.

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

- `plan-only`: inspect and return diagnosis, audit findings, options, or an
  implementation plan; do not edit.
- `execute-and-verify`: edit scoped files, run targeted verification, and report
  changed files, verification evidence, and residual risk.

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

- `plan-only`: inspect repository state and report findings or a plan only; use
  Claude Code plan mode, such as `--permission-mode plan`, when launched from CLI.
- `execute-and-verify`: edit within task-card scope and run verification before
  the delivery report.

Execution notes:

- The task card should name required project docs and paths instead of repeating
  all fixed protocol text.
- The task card should include exact verification commands when known.
- Claude Code must output the delivery report required by the project protocol.
- For Heavy tasks: if the card is `plan-only`, return the diagnosis/plan without
  mutation; if it declares `execute-and-verify`, execute and verify directly.
  Task level does not downgrade the permission mode or add another planning
  round.
- On Heavy resume, reread the task card, `git status --short`, and
  `review_targets`; then honor the confirmed `Permission mode` (`plan-only`
  remains non-mutating, `execute-and-verify` resumes).
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

- `plan-only`: use planning or chat mode to inspect code, docs, and diffs and
  return findings or a plan without editing.
- `execute-and-verify`: edit scoped files and run verification commands.

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

- Prefer `plan-only` unless the task card is self-contained and explicitly
  authorizes `execute-and-verify`.
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
| M4 | Task level never rewrites the permission mode. A resolved `execute-and-verify` card runs directly; Heavy adds no extra planning round. `current-task-approval` / `approve-writes` remain structured audit/hint signals; `approve-writes` may still act as the M9 generic-adapter capability override. |
| M5–M6 | `plan-only` must **never** produce write-type launch args. Active parallelism flags (`--parallel`, `--worktree`) and `--headless` are stripped. |
| M7 | `subagent`, `multi-session`, `agent-team` require Workflow authority `within-card` or `allowed`. `worktree` requires Workflow authority **not** `none`. |
| M8 | Every downgrade records a structured `DowngradeReason` with the before/after values and the triggering rule.  The `downgrade_reasons` list provides a full audit trail. |
| M9 | `generic` runtime adapter caps permission at `plan-only` without explicit approval. |
| M10 | Every downgrade records a structured reason. No downgrade = no reason entries. |

### CLI

Validate and resolve execution policy in one command:

```bash
ags policy resolve <task-card> --format text|json [--current-task-approval] [--approve-writes]
```

The old `ags resolve-policy` is kept as a hidden backward-compatible alias.

The command runs the canonical task-card validator first; on validation failure
it prints errors to stderr and exits 1.  On success it outputs the resolved
policy in text or JSON format.  It is **read-only** — it never launches a
runner.

The optional `--current-task-approval` flag sets `approval_source` to
`current-task-instruction`. It is an audit/hint signal only — task level no
longer downgrades the permission mode, so a Heavy card is already executable
when its declared permission mode is `execute-and-verify`. The signal does not
rewrite a `plan-only` card.

The optional `--approve-writes` flag sets `approval_source` to `cli-flag`. It is
likewise an audit/hint signal; it may additionally act as the M9 generic-adapter
capability override. Neither signal is required for a Heavy card to execute —
task level never downgrades an explicitly declared permission mode.

### Default semantics

| Input field | When absent | Resolved value |
|---|---|---|
| `Execution effort:` | absent or empty | `"unknown"` |
| `Workflow authority:` | absent or empty | `"none"` |
| `approval_source` | (not in task card fields) | `none` |

`Execution effort` accepts the neutral execution-intensity values `low` /
`normal` / `high` / `exhaustive` (default `unknown` when absent). The exhaustive
tier (`exhaustive`) sets `is_exhaustive_mode`; `ultracode` is retained only as a
parse-compatible legacy alias mapping to the same exhaustive semantics and must
not be generated into the front-stage task card. Host-private depth/workflow
trigger words are translated to execution behavior only by the claude-code
adapter / runner from the resolved policy — never read from the task-card body.

The resolver does not accept `approval_source` from the task card text — only
structured launch inputs can set it: `--current-task-approval` →
`current-task-instruction`, `--approve-writes` → `cli-flag`, or runner
environment override (`AGS_APPROVE_WRITES=1` → `runner-env`). Task card text is
**never** an approval source.

### Stop before launch

The resolver has one launch-blocking mechanism:

| Mechanism | Meaning | Runner behavior |
|---|---|---|
| `stop_before_launch=true` | Do **not** launch at all. | Runner must refuse to start. The task card or execution context must be corrected before another attempt. |

`stop_before_launch` is set when:
- Active parallelism (subagent, worktree, multi-session, agent-team) is
  requested but the effective permission mode forbids writes — the
  parallelism flags would create filesystem side effects incompatible
  with `plan-only`.
- `background-agent` execution surface is requested but the effective
  permission mode forbids writes — headless background execution could
  have side effects (process spawning, resource consumption) incompatible
  with `plan-only`.  The resolver records an M5 downgrade
  on `execution_surface` (before=`background-agent`, after=`cli`) and
  sets `stop_before_launch=true` with stop reason entry kind
  `background-surface-blocked-by-permission`.

Protected destructive, external-write, credential, migration, and release
actions may add their own action-specific stop/approval gates. Those gates are
orthogonal to task-card permission and do not create another permission mode.

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
| `effective_permission_mode` | string | `plan-only` or `execute-and-verify`. This is the only permission mode a runner may use. |
| `effective_parallelism` | string | `none`, `subagent`, `worktree`, `multi-session`, `agent-team`. This is the resolved value after authority and writability gates. |
| `effective_execution_surface` | string | `local-workspace`, `cli`, `ide`, `web`, `remote-control`, `background-agent`. If `background-agent` is blocked by `plan-only`, this becomes `cli`. |
| `allowed_launch_args` | string array | The exact runner CLI args allowed for launch. Runner MUST pass them verbatim and MUST NOT synthesize additional args from raw task-card fields. |
| `stop_before_launch` | boolean | If `true`, runner MUST refuse to launch. |
| `stop_reasons` | object array | Canonical stop reasons. This plural array is the only stop-reason field; legacy singular `stop_reason` MUST NOT be emitted or consumed. |
| `was_downgraded` | boolean | Whether any field was downgraded from the input card. |
| `downgrade_reasons` | object array | Full audit trail; each entry has `rule_id`, `field`, `before`, `after`, `reason`. |
| `execution_effort` | string | Declared effort, defaulting to `unknown` when absent. |
| `is_exhaustive_mode` | boolean | `true` for the exhaustive execution-effort tier (`Execution effort: exhaustive`, or the legacy `ultracode` alias); it never grants permission or parallelism. |
| `approval_source` | string | `none`, `current-task-instruction`, `cli-flag`, or `runner-env`. Task-card text is never an approval source. |

Stopped policy invariant:

- If `stop_before_launch=true`, `allowed_launch_args` MUST be `[]`.
- Runners MUST check `stop_before_launch` before consuming any launch args.

## Script Wrapper (`run-task-card.sh`) → `ags run`

`scripts/run-task-card.sh` is a **thin compatibility wrapper** that preserves the
historical script entry point. It performs no validation, gate, policy, adapter,
or receipt logic of its own. It forwards the task-card path and a small fixed set
of flags to the canonical Rust runner `ags run`, which owns the entire
resolver-first launch contract described below.

The wrapper's only real flags are:

- `--check-only` — stop after the gate check; exit `0` if allowed and `1` if
  stopped.
- `--dry-run` — emit the full launch plan without executing.
- `--current-task-approval` — pass live current-task approval through to the
  resolver as an audit/hint signal (task level does not downgrade the permission
  mode).
- `--approve-writes` — pass the write-approval audit/hint signal through to the
  resolver (may act as the M9 generic-adapter capability override).
- `--format text|json` — output format passed through to `ags run`
  (default `text`).

The task-card path must come FIRST; options follow it. Beyond argument
forwarding, the only extra behavior the wrapper adds is signal forwarding (it
kills the child `ags run` on cancellation) and a best-effort, post-task update
notifier that runs after a normal, real execution. The wrapper never reads raw
task-card fields and never synthesizes launch flags.

### Flow

```
task card
    │
    ▼
run-task-card.sh                           ◄── thin wrapper; forwards args only
    │
    ▼
ags run <task-card> [flags]                ◄── canonical runner owns all logic:
    │  validates the canonical task card        validation, gate, policy resolve,
    │  resolves execution policy (M1–M10)        adapter, receipt planning
    │  applies authority / writability gates
    │
    ▼
launch or stop
```

### Resolver-first contract (enforced by `ags run`)

The resolver enforces M5/M6: `plan-only` must never produce write-type launch
args or active parallelism flags.
`ags run` therefore drives launch from the **resolved execution policy**
(`ags policy resolve`), not from raw task-card fields. If a runner bypassed the
resolver and used unprocessed task-card values directly, it could produce
`--parallel`, `--worktree`, or `--headless` flags for a `plan-only` card —
bypassing the resolver's writability gate entirely.

`ags run` MUST:

1. Resolve the execution policy via `ags policy resolve <task-card> --format json`
   (with `--current-task-approval` or `--approve-writes` when the invoking
   context carries the matching structured approval).
2. Check `stop_before_launch` — if `true`, refuse to launch and surface all
   `stop_reasons` entries to the caller. Multiple independent gates can stop
   the same launch attempt.
3. Use `allowed_launch_args` verbatim as the CLI arguments for the launched
   session.
4. Never read `Parallelism:`, `Execution surface:`, or `Permission mode:`
   from the raw task card to decide launch flags — those values have already
   been resolved, downgraded, and gated by the resolver.
5. Run the runtime skill-tag availability gate (the third gate) on the
   launch-plan path. After the policy gate, `ags run` extracts the card's
   trailing `[skill: …]` tags, derives the active host from the resolved
   `runtime_adapter` (`claude-code` / `codex-local`→`codex` / `cursor`;
   `generic`/unknown → host-agnostic, fail-closed), and runs the equivalent of
   `ags gate skill-tags`. Any tag the live machine snapshot does not judge
   `available` forces `gate_decision=stop` (`gate_error_kind=skill_tags_unavailable`),
   empties launch args, and skips the receipt. This makes the third gate
   automatic on the main task-card execution chain — not only the manual
   `ags gate skill-tags` subcommand. `--check-only` stops at the offline policy
   gate and does NOT run the runtime skill-tag gate, preserving the validator's
   offline static determinism. The `LaunchPlan.skill_tags_gate` field carries the
   per-tag verdicts and `snapshot_hash` for audit.

### Example: resolved policy → launch flags

Given a task card with `Permission mode: plan-only`, `Parallelism: worktree`:

```json
{
  "effective_permission_mode": "plan-only",
  "effective_parallelism": "none",
  "allowed_launch_args": [],
  "stop_before_launch": true,
  "stop_reasons": [
    { "kind": "writable-parallelism-blocked-by-permission", ... }
  ]
}
```

`ags run` sees `stop_before_launch: true`, refuses to launch, and reports the
stop reason entries. It never generates `--parallel --worktree`, and it receives
no launch args at all because stopped policies expose `allowed_launch_args: []`.

### Defaults preserved

`ags run` never upgrades `Permission mode`. Task level never downgrades it
either: a Heavy card keeps its declared permission mode.
When a Heavy card omits `Permission mode:`, the compiler default is `plan-only`;
an explicit `execute-and-verify` mode is always preserved and runs without an
extra planning round.
Receipt-first execution remains an explicit runner flag; it is never enabled
implicitly.

### Planned — standalone auto-orchestration (not implemented)

> **Status: planned, not implemented in the current wrapper.** A standalone
> `run-task-card.sh --auto` orchestration mode — where the script itself reads
> the resolved policy JSON and decides launch flags — does **not** exist. The
> wrapper supports only `--check-only`, `--dry-run`, `--current-task-approval`,
> `--approve-writes`, and `--format`; it delegates the entire resolver-first launch contract above to
> `ags run`. Treat the resolver-first rules in this section as the contract
> `ags run` already enforces, not as a separate script-level auto mode.

## Planned — Learning Runner (not implemented)

> **Status: planned, not implemented in the current wrapper.** Neither
> `scripts/run-task-card.sh` nor `ags run` is "learning-enabled by default."
> The wrapper has no Task IR / compiled-brief compile step, no `--no-learning`
> or `--keep-ir` flags, and writes no `learning-gaps/` entries. The flags it
> actually accepts are `--check-only`, `--dry-run`, `--current-task-approval`,
> `--approve-writes`, and `--format`. The design below records the intended future capability and its
> boundary rules so they are not reinvented incompatibly; do not describe any
> of it as live behavior.

The planned learning runner would, before launching the executor, validate the
canonical task card, compile a transient Task IR / compiled brief, and inject the
brief as an execution guardrail.

Intended rules (planned):

- Task IR and compiled brief are not task-card formats.
- They must not be pasted into, appended to, or required as part of the
  canonical task-card skeleton.
- They would be temporary by default and deleted after the run.
- A `--keep-ir` flag would retain them in the receipt package for compiler
  debugging.
- A `--no-learning` flag would disable the transient compile and learning-gap
  extraction for a run.
- Long-term retention would be limited to `learning-gaps/` entries under local
  project memory when the runner detects reusable misses such as weak
  verification, executor delivery failure, nonzero execution, or compiler
  coverage gaps.
- Learning gaps would be review proposals. They must not automatically update
  `context-capsule.md`, task-card templates, protocol files, validator rules, or
  project profiles.
