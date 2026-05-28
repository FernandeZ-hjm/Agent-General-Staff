# Agent Task Routing

This document defines how Cursor should classify and execute development tasks.

Use this file before any development, debugging, review, architecture, migration, data, or handoff task. The goal is for Cursor to replace the old Codex orchestration role inside this development suite while still being able to delegate bounded implementation work to Claude Code CLI when useful.

## Operating Model

- Cursor owns diagnosis, architecture, task framing, implementation strategy, verification, and final review.
- Cursor may directly implement changes inside the repository when the task is light or medium and the risk is controlled.
- Cursor may delegate bounded execution to Claude Code CLI, but must provide a self-contained prompt and review the result before treating the task as complete.
- Skills provide procedural guardrails.
- Verification evidence is required before a task is treated as complete.

Do not use one generic workflow for every task. Classify the task first, then choose the smallest workflow that still captures the risk.

## Classification Flow

1. Read `config/agent-project-profile.yaml` when present, and use it only as a
   slot-filling source.
2. Identify the task type from the user's natural-language request.
3. Identify the blast radius.
4. Identify whether data, historical outputs, vector stores, migrations, or irreversible operations are involved.
5. Identify whether Cursor may directly edit files, should first return a short design note, or must wait for confirmation.
6. Decide whether to implement directly or delegate to Claude Code CLI.
7. Set `Review gate` to the canonical Review Gate rules in `agent-task-protocol.md`.
8. Select only the skill tags that directly apply.
9. Define the narrowest meaningful verification command before editing.

## Task Card Compiler v2

The compiler turns flexible user intent into the fixed task-card skeleton. This
is the primary flexibility layer; do not create alternate full templates to
handle conversational variation.

Compiler rules:

- Keep task-card headings, field order, and baseline wording stable.
- Fill dynamic slots from the user request, repository evidence, project
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

- Read relevant files.
- Make the change.
- Run the smallest meaningful verification.
- Report modified files and results.

Delegation default:

- Do not delegate unless the user asks for Claude Code CLI or local execution would be slower than a bounded subtask prompt.

Common skill tags:

```text
[skill: verify]
```

Add `[skill: tdd]` only when the user explicitly wants test-first work or the bug is best captured by a new regression test.

## Medium Task

Use the medium template when any of the following are true:

- Multiple files are likely affected.
- The task changes behavior across a module boundary.
- The task needs a brief implementation plan before editing.
- The task touches configuration, tests, CLI behavior, API clients, or shared helpers.
- The change has rollback or compatibility concerns, but does not touch live data stores or historical baseline assets.

Examples:

- Add a feature to an existing pipeline.
- Refactor a shared helper with tests.
- Fix a bug whose root cause is not obvious.
- Update configuration loading behavior.
- Improve reliability of a script without changing the data model.

Default execution mode:

- Read code and explain current behavior.
- Give a concise root cause or design note.
- Implement after the plan is clear.
- Run targeted tests and smoke checks.
- Report changed files, verification, and residual risks.

Delegation default:

- Implement directly unless the task is large enough that a bounded Claude Code CLI prompt would improve throughput.
- If delegating, Cursor must still own task boundaries, constraints, and final review.

Common skill tags:

```text
[skill: diagnose]
[skill: verify]
```

Add `[skill: tdd]` for test-first work.
Add `[skill: review]` when the task is mostly code review.
Add `[skill: commit]` only when the user asks for a commit message or commit-ready output.

## Heavy Task

Use the heavy template when any of the following are true:

- The task touches historical data, cleaned outputs, vector stores, databases, indexes, manifests, or curated datasets.
- The task must preserve an old baseline while creating a new layer.
- The task includes migration, deduplication, quarantine, rollback, audit, or traceability requirements.
- The task changes architecture, ingestion rules, filtering standards, or long-lived quality gates.
- The task has a large blast radius across directories, scripts, configs, and tests.
- The task requires dry-run first, then staged implementation.
- The user explicitly says not to delete, overwrite, reinstall, re-clean, or mutate a baseline.
- Cursor or any delegated Claude Code CLI run must first return root cause, design, and implementation plan before editing.

Examples:

- Curate an old vector library without breaking the baseline.
- Build a new shadow collection from existing cleaned docs.
- Migrate storage or indexing behavior.
- Redesign a pipeline stage.
- Audit and quarantine low-value historical outputs.
- Add traceable manifests and quality reports.

Default execution mode:

- Read existing code, docs, directories, and relevant data shape.
- Return root cause, design, and implementation plan first.
- Default to `Permission mode: plan-only`.
- Wait for user confirmation before code changes.
- First implementation pass must be dry-run or read-only audit when data safety matters.
- Generate audit evidence for automatic judgments.
- Keep old baselines untouched unless the user explicitly approves mutation.
- Produce a delivery report with verification evidence.

Delegation default:

- Cursor should frame the heavy task itself before delegation.
- Any Claude Code CLI prompt must include goals, non-goals, hard constraints, relevant paths, baseline preservation rules, staged execution flow, verification commands, delivery report format, and skill tags.
- Cursor must inspect the resulting diff, report, and verification evidence before marking the work complete.

Common skill tags:

```text
[skill: diagnose]
[skill: zoom-out]
[skill: verify]
```

Add `[skill: tdd]` when tests should drive the implementation.
Add `[skill: database-migration]` if schema/data migration is involved.
Add `[skill: supply-chain-risk-auditor]` only for dependency or package risk assessment.

## Escalation Rules

When in doubt, choose the heavier template if any of these risks exist:

- Data loss.
- Irreversible write.
- Baseline mutation.
- Ambiguous domain rules.
- Multiple plausible designs.
- Weak test coverage around the affected behavior.
- User wants auditability or traceability.

Do not escalate just because a prompt is long. Escalate because the task has higher risk or broader blast radius.

## Review Gate Defaults

Every task card must include a `Review gate:` field. The single canonical
Light / Medium / Heavy mapping lives in
`docs/agent-workflow/agent-task-protocol.md`; task cards and fallback templates
should reference that rule instead of copying the full text.

## Skill Tag Rules

Use manual skill tags only for skills Cursor or a delegated Claude Code CLI run should explicitly load.

Do not add automatic trigger skills as manual tags unless the project protocol explicitly defines them as manual aliases.

For this project:

- `auto-debug` triggers on errors, failures, broken behavior, or test failures.
- `auto-verify` triggers when work is claimed complete.
- Use `[skill: verify]` when deep verification should be forced.
- Use `[skill: diagnose]` for complex root cause work.
- Use `[skill: zoom-out]` for architecture context, dependency mapping, or risk assessment.
- Use `[skill: tdd]` for test-driven implementation.
- Use `[skill: commit]` only for commit-message or commit-ready tasks.
- Use `[skill: review]` for actionable code review output.

## Task Handoff Protocol

Cursor / Codex / Claude Code 三方交接遵循 `docs/agent-workflow/agent-task-protocol.md`。生成任务卡时使用 `docs/agent-workflow/task-card-template.md`。

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
- Explicit confirmation gate before mutation.
- Resume / compression recovery rules: on "继续", context compression, or task-notification resume, reread the task card, run `git status --short`, reconfirm `review_targets`, and stop at the confirmation gate unless mutation approval is explicit in the current context.
