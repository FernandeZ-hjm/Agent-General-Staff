# Agent Task Routing

This document defines how Cursor should understand, form solutions for, and route
development tasks. Routing (Light / Medium / Heavy classification) happens only
after the solution is formed and confirmed — never from the raw user request
alone.

Use this file before any development, debugging, review, architecture, migration,
data, or handoff task. The goal is for Cursor to replace the old Codex
orchestration role inside this development suite while still being able to
delegate bounded implementation work to Claude Code CLI when useful.

## Lifecycle Order (Mandatory)

A development request must flow through these phases in order. Do not skip ahead
to classification or execution before the earlier phases are complete:

1. **Ambient preflight** — detect project identity, read context capsule and task
   memory, load protocol files, check git status.
2. **Solution phase** — understand the request, diagnose when needed, form a
   concrete solution or implementation approach.
3. **User confirmation** — present the solution and wait for explicit user
   approval. Do not proceed to routing without confirmation.
4. **Execution decision** — the user either explicitly authorizes same-session
   `direct-edit`, explicitly requests a task-card handoff, or leaves the session
   read-only. \"方案 OK\" alone authorizes neither mutation path.
5. **Routing** — classify the confirmed solution as Light / Medium / Heavy using
   the criteria in this file, then choose direct execution or handoff.
6. **Gate / execution / receipt** — apply the chosen path's independent safety,
   review, verification, and receipt requirements.

Existing-card entry is validate-first: when the first non-empty line is
`## 任务卡`, skip raw-request solution/classification and card generation. A
valid card proceeds to policy/runner; an invalid card-shaped payload stops.

**Hard rule**: the user's initial natural-language request is NOT an executable
task card and must NOT be used directly for Light / Medium / Heavy
classification. Always complete ambient preflight and solution formation first.
\"方案 OK\" alone is not mutation authority. Explicit same-session modification
authorization selects `direct-edit`; an explicit task-card/handoff instruction
selects task-card generation.

## Operating Model

- Cursor owns preflight, diagnosis, solution formation, task framing,
  implementation strategy, verification, and final review.
- Cursor may directly implement changes inside the repository when the task is
  light or medium and the risk is controlled.
- Cursor may delegate bounded execution to Claude Code CLI, but must provide a
  self-contained prompt and review the result before treating the task as
  complete.
- Skills provide procedural guardrails.
- Verification evidence is required before a task is treated as complete.

Do not jump to classification from raw user requests. First understand the
request, form a solution, and get user confirmation. Only after the solution is
confirmed should you classify the task and choose the smallest workflow that
still captures the risk.

## Preflight / Solution / Confirmation Flow

Before any classification or routing decision:

1. Run `ags project detect` (when available) or manually read `AGENTS.md`,
   `CLAUDE.md`, `WORKSPACE.md`, and `config/agent-project-profile.yaml` when
   present.
2. Read the context capsule and task memory when available (see
   `protocol/context-memory.md`).
3. Read relevant protocol files (`protocol/agent-task-protocol.md`,
   `protocol/runtime-adapters.md`).
4. Run `git status --short` to record current repository state.
5. Understand the user's request: clarify ambiguities, diagnose when the request
   describes a problem, and form a concrete solution.
6. Present the solution to the user and wait for explicit confirmation.
7. Once confirmed, use the solution as the stable input to routing.
8. Require an explicit execution decision: same-session modification authorization
   enters `direct-edit`; a task-card/handoff instruction enters task-card generation;
   otherwise remain read-only.

Only after steps 7 AND 8 should you proceed to an execution path below.

## Routing Phase (Solution-Confirmed)

After the user has confirmed a solution, classify it using the criteria below.
The classification determines risk, review, verification, and — for handoffs only —
task-card fields and delegation rules.

1. Identify the task type from the confirmed execution contract (not the raw
   user request).
2. Identify the blast radius.
3. Identify whether data, historical outputs, vector stores, migrations, or
   irreversible operations are involved.
4. Identify whether Cursor may directly edit files, should first return a short
   design note, or must wait for confirmation.
5. Decide whether to implement directly or delegate to Claude Code CLI.
6. Set `Review gate` to the canonical Review Gate rules in
   `agent-task-protocol.md`.
7. Select only the skill tags that directly apply.
8. Define the narrowest meaningful verification command before editing.

## Value Route (效价比路由)

Before routing, AGS surfaces a Value Route recommendation — the minimal
execution-path *form* that still covers the risk. This is the structured form of
"choose the smallest workflow that still captures the risk" above. It is advisory
and deterministic, and it shapes path form only: it does NOT replace or change the
Light / Medium / Heavy level, permission mode, Review gate, or Verification gate.
The canonical definition (path forms, exposed fields, evidence format) lives in
`protocol/agent-task-protocol.md` §3.9.

Path forms: `read-only-advisory`, `direct-edit`, `plan-first`,
`claude-code-route`, `stop-for-scope`. Pick the lightest form that still covers
the risk, and record why a lighter form would under-cover and a heavier form
would over-spend.

When a route becomes a task card, both `read-only-advisory` and `plan-first` map
to `Permission mode: plan-only`; direct or delegated implementation maps to
`Permission mode: execute-and-verify`. Route names do not create additional
permission modes.

AGS exposes the recommendation in `ags_solution_check` (MCP) and `ags gate
prompt-request` (CLI) as a `value_route` block. The planner owns the final path
and records it as evidence; Light / Medium / Heavy routing then proceeds
independently — Value Route is not a fourth task level.

## Capability Route (能力路由)

Parallel to Value Route, AGS also surfaces a `capability_route` block on the same
two entries (`ags_solution_check` and `ags gate prompt-request`). Where Value
Route answers "which execution-path *form* covers this risk?", Capability Route
answers "which managed capability should the host be *advised to wake up* for
this demand, and is it reachable?". It is deterministic, manifest-driven, and
advisory only: it never auto-invokes a skill/MCP/CLI, never blocks the request,
and never changes the Light / Medium / Heavy level, permission mode, Review gate,
or Verification gate. Reachability also depends on machine-local enrollment
(`ags setup --capability-route <off|suite-only|adopted|review-all>`); a
not-enrolled capability degrades to advisory and never blocks. The `auto-*` skills
are explicit-wakeup compatibility aliases, not implicit auto-triggers AGS depends
on. The canonical definition (fields, fail-closed availability, enrollment,
host/target resolution) lives in `protocol/agent-task-protocol.md` §3.10.

## Task Card Compiler v2

The compiler turns a confirmed execution contract into the fixed task-card
skeleton. This is the primary flexibility layer; do not create alternate full
templates to handle conversational variation.

The compiler's input is an **approved execution intent** (the confirmed
solution), not raw user chat. It may accept flexible intent files for
compatibility, but generators (Codex / Cursor) must only feed it confirmed
solutions.

Compiler rules:

- Keep task-card headings, field order, and baseline wording stable.
- Fill dynamic slots from the execution contract, repository evidence, project
  workflow docs, and `config/agent-project-profile.yaml` when present.
- Prefer short references to stable docs and the project profile over copying
  long repeated rules.
- Put runner history in `任务存档` references under local context memory; do not
  paste historical logs into a new task card.
- Put volatile facts such as command output, current diffs, or one-off evidence
  in `背景`, `验证`, or the delivery report, not in the `读取并遵守` list.
- If the profile suggests a default but live evidence disagrees, use live
  evidence and record the mismatch in `背景` or `实施要求`.
- If required slot values cannot be inferred safely, fill the slot with an
  explicit stop condition rather than inventing facts.
- **Do not feed raw user chat directly to the compiler.** The compiler accepts
  flexible intents for backward compatibility, but generators must only pass
  confirmed execution contracts.

## Light Task

Use the light template when all of the following are true:

- The change is small and local.
- One file or a narrow code path is likely affected.
- No data migration, vector store, database, or historical output is involved.
- No architecture boundary is being changed.
- Verification is straightforward.
- Cursor can execute directly after reading the relevant file.

Examples:

- Fix a typo or log message.
- Adjust a small condition.
- Add a small CLI option.
- Patch a focused bug with an obvious failing behavior.
- Add or update a narrow unit test.

Default execution mode:

- Use `Permission mode: execute-and-verify`.
- Read relevant files.
- Make the change.
- Run the smallest meaningful verification.
- Report modified files and results.

Delegation default:

- Do not delegate unless the user asks for Claude Code CLI or local execution
  would be slower than a bounded subtask prompt.

Common skill tags:

```text
[skill: verification-before-completion]
```

Add `[skill: test-driven-development]` only when the user explicitly wants
test-first work or the bug is best captured by a new regression test.

## Medium Task

Use the medium template when any of the following are true:

- Multiple files are likely affected.
- The task changes behavior across a module boundary.
- The task benefits from a brief, non-blocking design note during direct
  execution.
- The task touches configuration, tests, CLI behavior, API clients, or shared
  helpers.
- The change has rollback or compatibility concerns, but does not touch live
  data stores or historical baseline assets.

Examples:

- Add a feature to an existing pipeline.
- Refactor a shared helper with tests.
- Fix a bug whose root cause is not obvious.
- Update configuration loading behavior.
- Improve reliability of a script without changing the data model.

Default execution mode:

- Use `Permission mode: execute-and-verify`; a short root-cause or design note
  does not change the direct-execution state.
- Read code and explain current behavior.
- Give a concise root cause or design note.
- Implement after the plan is clear.
- Run targeted tests and smoke checks.
- Report changed files, verification, and residual risks.

Delegation default:

- Implement directly unless the task is large enough that a bounded Claude Code
  CLI prompt would improve throughput.
- If delegating, Cursor must still own task boundaries, constraints, and final
  review.

Common skill tags:

```text
[skill: diagnosing-bugs]
[skill: verification-before-completion]
```

Add `[skill: test-driven-development]` for test-first work.
Add `[skill: review]` when the task is mostly code review.
Do not add a commit-message skill tag; describe commit-message requirements in
the task body unless a current commit skill exists.

## Heavy Task

Use the heavy template when any of the following are true:

- The task touches historical data, cleaned outputs, vector stores, databases,
  indexes, manifests, or curated datasets.
- The task must preserve an old baseline while creating a new layer.
- The task includes migration, deduplication, quarantine, rollback, audit, or
  traceability requirements.
- The task changes architecture, ingestion rules, filtering standards, or
  long-lived quality gates.
- The task has a large blast radius across directories, scripts, configs, and
  tests.
- The task requires dry-run first, then staged implementation.
- The user explicitly says not to delete, overwrite, reinstall, re-clean, or
  mutate a baseline.
- The confirmed execution contract explicitly requests a planning-only pass
  before any later implementation card.

Examples:

- Curate an old vector library without breaking the baseline.
- Build a new shadow collection from existing cleaned docs.
- Migrate storage or indexing behavior.
- Redesign a pipeline stage.
- Audit and quarantine low-value historical outputs.
- Add traceable manifests and quality reports.

Default execution mode (task level never downgrades an explicitly declared
permission mode — it only sets the default when `Permission mode:` is omitted):

- Read existing code, docs, directories, and relevant data shape.
- **Heavy plan** (`plan-only`, declared or the default when `Permission mode:` is
  unspecified for this high-risk class): return root cause, design, implementation
  plan, and verification plan, then stop without making any code change. A later
  implementation requires an `execute-and-verify` task card.
- **Heavy execute** (`execute-and-verify`): run and verify directly per the task
  card. The level adds no planning round.
- First implementation pass must be dry-run or read-only audit when data safety
  matters.
- An independent Review gate (human / Codex / adversarial) is required for an
  executable Heavy card regardless of class.
- Generate audit evidence for automatic judgments.
- Keep old baselines untouched unless the user explicitly approves mutation.
- Produce a delivery report with verification evidence.

Delegation default:

- Cursor should frame the heavy task itself before delegation.
- Any Claude Code CLI prompt must include goals, non-goals, hard constraints,
  relevant paths, baseline preservation rules, staged execution flow,
  verification commands, delivery report format, and skill tags.
- Cursor must inspect the resulting diff, report, and verification evidence
  before marking the work complete.

Common skill tags:

```text
[skill: diagnosing-bugs]
[skill: codebase-design]
[skill: verification-before-completion]
```

Add `[skill: test-driven-development]` when tests should drive the implementation.
Add `[skill: database-migration]` if schema/data migration is involved.
Add `[skill: supply-chain-risk-auditor]` only for dependency or package risk
assessment.

## Escalation Rules

When in doubt, choose the heavier template if any of these risks exist:

- Data loss.
- Irreversible write.
- Baseline mutation.
- Cross-file protocol or task-card skeleton change.
- Runtime adapter, hook, permission, or review gate behavior change.
- Path migration, generated artifact synchronization, or cross-repository target
  mapping.
- Data writes, ledger writes, vector store writes, or persistent output changes.
- Ambiguous domain rules.
- Multiple plausible designs.
- Weak test coverage around the affected behavior.
- User wants auditability or traceability.

Do not escalate just because a prompt is long. Escalate because the task has
higher risk or broader blast radius.

## Review Gate Defaults

Every task card must include a `Review gate:` field. The single canonical
Light / Medium / Heavy mapping lives in
`protocol/agent-task-protocol.md`; task cards and fallback templates
should reference that rule instead of copying the full text.

## Skill Tag Rules

Use manual skill tags only for skills Cursor or a delegated Claude Code CLI run
should explicitly load.

Do not add automatic trigger skills as manual tags unless the project protocol
explicitly defines them as manual aliases.

For this project:

- The legacy automatic aliases are **not valid manual task-card tags**. AGS
  Capability Route never routes them as primary; their demands now route to
  canonical successors — debug → `diagnosing-bugs`, verify →
  `verification-before-completion`, brainstorm → `grill-with-docs`.
- Manual task-card tags are a positive allowlist: use only current
  `route_state: routable` skills whose registry `invoke_hint` is shaped as
  `[skill: ...]`.
- Use `[skill: verification-before-completion]` when deep verification should be forced.
- Use `[skill: diagnosing-bugs]` for complex root cause work.
- Use `[skill: codebase-design]` for architecture context, dependency mapping, or risk
  assessment.
- Use `[skill: test-driven-development]` for test-driven implementation.
- Use `[skill: review]` for actionable code review output.

## Task Handoff Protocol

Cursor / Codex / Claude Code 三方交接遵循 `protocol/agent-task-protocol.md`。
生成任务卡时使用 `protocol/task-card-template.md`。任务卡的输入必须是已确认的
方案或 execution contract，不能是原始用户自然语言请求。

## Prompt Generation Requirements

Every generated Claude Code CLI prompt must include:

- Task summary.
- Context and current evidence.
- Goals.
- Non-goals.
- Hard constraints.
- Relevant paths and modules.
- Expected output or artifacts.
- Verification standard.
- Delivery report format.
- Skill tags.

Heavy prompts must additionally include:

- Baseline preservation rules.
- Staged execution flow.
- Dry-run or audit-first requirement.
- Traceability and rollback requirements.
- Independent Review gate (human / Codex / adversarial).
- Resume / compression recovery rules: on "继续", context compression, or
  task-notification resume, reread the task card, run `git status --short`,
  reconfirm `review_targets`, and honor the card's permission mode before
  mutation (`plan-only` remains non-mutating; `execute-and-verify` resumes
  execution and verification). A resume token cannot rewrite the confirmed task
  card's authority.
