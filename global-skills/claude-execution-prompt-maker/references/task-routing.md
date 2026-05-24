# Claude Code Prompt Task Routing

Use this document to classify a user request before generating a Claude Code execution prompt.

## Operating Model

- Codex owns diagnosis, architecture, task framing, prompt generation, and final review.
- Claude Code owns repository execution.
- Skills provide procedural guardrails.
- Verification evidence is required before a task is treated as complete.

## Classification Flow

1. Identify the task type.
2. Identify blast radius.
3. Identify whether data, historical outputs, vector stores, databases, migrations, deletion, overwrites, or irreversible operations are involved.
4. Identify whether Claude Code may directly edit files or must first return a plan.
5. Select `light`, `medium`, or `heavy`.
6. Add only directly relevant skill tags.

## Light

Use `light` when all are true:

- The task is small and local.
- One file or a narrow code path is likely affected.
- No data migration, database, vector store, historical output, or architecture boundary is involved.
- Verification is straightforward.
- Claude Code can execute directly after reading relevant files.

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
- Claude Code must first return root cause, design, and implementation plan before editing.

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

## Tag Rules

- Add `[skill: tdd]` when test-first development is requested or the task is best driven by a regression test.
- Add `[skill: commit]` only for commit-message or commit-ready tasks.
- Add `[skill: review]` for code review tasks.
- Add `[skill: database-migration]` only when schema/data migration is involved.
- Add `[skill: supply-chain-risk-auditor]` only for dependency or package risk assessment.
- Do not manually add automatic trigger skills unless the current project defines them as manual aliases.
