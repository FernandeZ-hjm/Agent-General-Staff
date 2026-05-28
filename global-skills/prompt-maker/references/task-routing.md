# Agent Task Routing

Use this document to classify a user request before generating an execution task card for Claude Code, Codex, Cursor, or another supported runtime.

## Operating Model

- Codex owns diagnosis, architecture, task framing, prompt generation, and final review unless `Executor: Codex` is explicitly selected.
- The task card declares the executor through `Executor` and `Runtime adapter`.
- Skills provide procedural guardrails.
- Verification evidence is required before a task is treated as complete.

## Classification Flow

1. Identify the task type.
2. Identify blast radius.
3. Identify whether data, historical outputs, vector stores, databases, migrations, deletion, overwrites, or irreversible operations are involved.
4. Select `Executor`, `Runtime adapter`, and `Execution surface`.
5. Identify whether the executor may directly edit files or must first return a plan.
6. Select `light`, `medium`, or `heavy`.
7. Select `Permission mode`, `Parallelism`, `Review gate`, and `Verification gate`.
8. Add only directly relevant skill tags.

## Light

Use `light` when all are true:

- The task is small and local.
- One file or a narrow code path is likely affected.
- No data migration, database, vector store, historical output, or architecture boundary is involved.
- Verification is straightforward.
- The executor can execute directly after reading relevant files.

Examples:

- Fix a typo or log message.
- Adjust a small condition.
- Add a narrow unit test.
- Patch an obvious focused bug.

Default tags:

```text
[skill: verify]
```

## Medium

Use `medium` when any are true:

- Multiple files are likely affected.
- Behavior changes across a module boundary.
- The task touches configuration, tests, CLI behavior, API clients, or shared helpers.
- A concise plan is useful before editing.
- Rollback or compatibility matters, but no live data store or historical baseline is touched.

Examples:

- Add a feature to an existing module.
- Refactor a shared helper with tests.
- Fix a bug whose root cause is not obvious.
- Update configuration loading behavior.

Default tags:

```text
[skill: diagnose]
[skill: verify]
```

## Heavy

Use `heavy` when any are true:

- The task touches historical data, cleaned outputs, vector stores, databases, indexes, manifests, curated datasets, or baseline assets.
- The task includes migration, deduplication, quarantine, rollback, audit, traceability, or staged rollout.
- The task changes architecture, ingestion rules, filtering standards, security boundaries, or long-lived quality gates.
- The task requires dry-run first.
- The task has a broad blast radius across directories, configs, scripts, tests, or runtime workflows.
- The user explicitly says not to delete, overwrite, reinstall, re-clean, mutate, or break a baseline.
- The executor must first return root cause, design, and implementation plan before editing.

Examples:

- Curate or migrate a historical vector library.
- Build a shadow collection from existing data.
- Redesign a pipeline stage.
- Audit and quarantine low-value outputs.
- Add traceable manifests and quality reports.

Default tags:

```text
[skill: diagnose]
[skill: zoom-out]
[skill: verify]
```

## Escalation Rules

When in doubt, choose the heavier template if any of these risks exist:

- Data loss.
- Irreversible write.
- Baseline mutation.
- Ambiguous domain rules.
- Multiple plausible designs.
- Weak test coverage.
- User wants auditability or traceability.

Do not escalate just because the prompt is long. Escalate because the task has higher risk or broader blast radius.

## Runtime Adapter Defaults

Default adapter selection:

- User asks "you execute" or "你直接做": `Executor: Codex`, `Runtime adapter: codex-local`, `Execution surface: local-workspace`.
- User asks for Claude Code: `Executor: Claude Code`, `Runtime adapter: claude-code`, `Execution surface: cli`.
- User asks for Cursor: `Executor: Cursor`, `Runtime adapter: cursor`, `Execution surface: ide`.
- Unknown or external executor: `Executor: Other`, `Runtime adapter: generic`.

Default permission by task level:

- `Light`: `Permission mode: execute-and-verify`, `Parallelism: none`.
- `Medium`: `Permission mode: edit-with-confirmation`, `Parallelism: none`.
- `Heavy`: `Permission mode: plan-only`, `Parallelism: none`.

For Heavy tasks, `plan-only` is binding until explicit human approval for the
current task. Only after that approval may the generated task card move to
`edit-with-confirmation` or `execute-and-verify`.

## Resume / Compression Recovery

When a request is a continuation, context-compression recovery,
task-notification follow-up, or says "继续":

- If the task level is Heavy, require the executor to reread the task card, run
  `git status --short`, and reconfirm `review_targets`.
- If the current context does not contain explicit human approval for mutation,
  keep the task card at `Permission mode: plan-only` and stop at the
  confirmation gate.
- Do not treat "继续", a resume notification, an earlier plan, or a compressed
  summary as approval for Heavy writes.

Always include `Verification gate` with commands, expected evidence, and stop condition.
Always include `Review gate` with the fixed Light / Medium / Heavy mapping:

- `Light`: 完成前自查 diff；提交前建议运行 `caveman-review`。
- `Medium`: 人工 Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 review”，并提醒操作者手动运行 `/codex:review` 后再放行。
- `Heavy`: 先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 adversarial review”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。

## Tag Rules

- Add `[skill: tdd]` when test-first development is requested or the task is best driven by a regression test.
- Add `[skill: commit]` only for commit-message or commit-ready tasks.
- Add `[skill: review]` for code review tasks.
- Add `[skill: database-migration]` only when schema/data migration is involved.
- Add `[skill: supply-chain-risk-auditor]` only for dependency or package risk assessment.
- Do not manually add automatic trigger skills unless the current project defines them as manual aliases.
