---
name: claude-execution-prompt-maker
description: >
  Generate execution-ready prompts for Claude Code. Use this whenever the user asks Codex to write a Claude Code prompt, hand off a diagnosis to Claude Code, produce an executable task brief, or choose a prompt template by task complexity. Always classify the task as light, medium, or heavy before generating the final prompt.
---

# Claude Execution Prompt Maker

Use this skill to turn a user request, Codex diagnosis, or implementation idea into a clear Claude Code execution prompt.

This skill is for prompt handoff only. Codex still owns diagnosis, architecture framing, and final review. Claude Code owns repository execution.

## Workflow

1. Read the user's request and any Codex diagnosis already produced.
2. Inspect local project rules when available:
   - `AGENTS.md`
   - `CLAUDE.md`
   - nested `CLAUDE.md` files near the target module
   - project docs that define commands, data safety, or workflow rules
3. Choose the output format in this order:
   - If the current project has `docs/agent-workflow/task-card-template.md`, prefer the project task-card format.
   - If the task matches a project-specific file under `docs/agent-workflow/task-cards/`, use that file only as a slot-filling module, not as a separate full task-card template.
   - Use the global full templates only when the project has no task-card protocol, the task is cross-repo, an external agent will execute it, or Claude Code cannot read the project files.
4. Read project routing docs when present, otherwise read `references/task-routing.md`.
5. Classify the task as `light`, `medium`, or `heavy`.
6. Fill the selected project task card or global template with concrete project facts.
7. Ensure the prompt requires Claude Code to use `claude-delivery-report` or the project's delivery-report protocol before its final response.
8. Add only directly relevant skill tags.
9. Output the final Claude Code prompt or task card.

## Classification Standard

Use the smallest template that still captures the risk.

Escalate to `heavy` when the task touches data stores, historical outputs, vector stores, migrations, baseline preservation, deletion, overwrites, irreversible operations, staged rollout, audit evidence, or broad architecture changes.

Use `medium` for multi-file behavior changes, shared helpers, configuration changes, nontrivial bug fixes, or local refactors that need a short plan.

Use `light` for narrow, low-risk, directly executable changes.

## Project Overrides

When a project contains its own agent workflow docs, treat them as project-specific overrides. Common locations:

- `docs/agent-workflow/agent-task-protocol.md`
- `docs/agent-workflow/task-routing.md`
- `docs/agent-workflow/task-card-template.md`
- `docs/agent-workflow/task-cards/*.md` as optional slot-filling modules
- `docs/agent-workflow/templates/*.md`
- `docs/agent-workflow/delivery-report-standard.md`

Project overrides can add paths, commands, domain constraints, and safety rules. They should not remove global safety requirements unless the user explicitly says so.

## Template Selection

The output must always be one of exactly two task-card formats. Never create a third format.

1. **Project task card** — use the fixed skeleton from `docs/agent-workflow/task-card-template.md` when it exists.
2. **Global fallback task card** — use `templates/fallback-task-cards/{light,medium,heavy}.md` when no project protocol exists, the task is cross-repo, or Claude Code cannot access project files.

Prohibited: free-form runbooks, machine-specific full templates, phase-specific templates, tool-specific formats, or any format that is neither a project task card nor a global fallback task card.

Prefer a project task card over a global full prompt when Claude Code can read the repository files. The task card should reference the project's protocol files instead of repeating all fixed rules.

Do not create or select a third category of specialized full task cards. When the task domain has a project-specific helper under `docs/agent-workflow/task-cards/`, read it as a module and fill its constraints into the fixed project task-card skeleton.

For cache stability, keep the final task-card skeleton stable:

- use the same section titles,
- keep sections in the same order,
- keep baseline wording stable,
- put dynamic facts only in the fixed slots,
- put domain-specific additions under `实施要求`, `适用治理文档`, `相关路径`, or `验证`.

Use the global full templates only as a fallback:

- no project task-card protocol exists,
- Claude Code cannot access the project files,
- the execution target is outside the repository,
- the prompt must be self-contained for an external agent.

## Output Rules

The final answer should contain:

1. A one-line classification:
   - `任务级别：Light / Medium / Heavy`
   - `使用模板：...`
2. A short reason for the classification.
3. The final Claude Code prompt or task card in one fenced `markdown` block.

Do not include a long explanation before the prompt. The user usually wants to copy the generated prompt into Claude Code.

## Skill Tags

Use manual tags only for skills Claude Code should explicitly load.

Default examples:

```text
[skill: verify]
```

```text
[skill: diagnose]
[skill: verify]
```

```text
[skill: diagnose]
[skill: zoom-out]
[skill: verify]
```

Add `[skill: tdd]` when test-first work is requested or appropriate.
Add `[skill: commit]` only when the user asks for commit-ready output.
Add `[skill: review]` for code review tasks.
Add domain-specific skill tags only when directly relevant.

Avoid adding automatic trigger skills as manual tags unless the current project explicitly defines them as manual aliases.

## Quality Gate

Before returning the final prompt, check:

- The output is exactly one of the two allowed task-card formats (project or global fallback). If it matches neither, redo from Template Selection.
- The prompt has clear goals and non-goals.
- Hard constraints are explicit.
- Relevant paths and modules are named when known.
- Verification commands are included or the prompt asks Claude Code to identify them.
- The prompt requires `claude-delivery-report` or the project's delivery-report protocol for the final task report.
- Destructive actions are blocked unless explicitly approved.
- Heavy tasks include a confirmation gate before mutation.
