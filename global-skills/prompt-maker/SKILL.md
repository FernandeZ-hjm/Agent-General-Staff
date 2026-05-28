---
name: prompt-maker
description: >
  Generate execution-ready task cards for Claude Code or another supported agent runtime.
  Use this whenever the user asks Codex to write a Claude Code prompt, hand off a
  diagnosis to an executor, produce an executable task brief, or choose a prompt
  template by task complexity. Always classify the task as light, medium, or heavy
  before generating the final prompt.
---

# Prompt Maker

Prompt Maker v2 is a task-card compiler. It turns flexible user intent into the
cache-stable task-card skeleton, using project profile data only to fill fixed
dynamic slots.

Use this skill to turn a user request, Codex diagnosis, or implementation idea into a clear execution task card for Claude Code or another supported agent runtime.

This skill is for prompt or task-card handoff. Codex still owns diagnosis, architecture framing, and final review unless the task card explicitly sets `Executor: Codex`.

## Workflow

1. Read the user's request and any Codex diagnosis already produced.
2. Inspect local project rules when available:
   - `AGENTS.md`
   - `CLAUDE.md`
   - `config/agent-project-profile.yaml`
   - `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
   - nested `CLAUDE.md` files near the target module
   - project docs that define commands, data safety, runtime adapters, or workflow rules
3. Build a slot plan before writing the final card:
   - intent and task type,
   - task level,
   - executor/runtime/permission/parallelism,
   - profile-derived defaults,
   - memory-capsule references,
   - task-archive references,
   - paths and docs,
   - verification commands,
   - stop conditions.
4. Choose the output format in this order:
   - If the current project has `docs/agent-workflow/task-card-template.md`, prefer the project task-card format.
   - Use the global full templates only when the project has no task-card protocol, the task is cross-repo, an external agent will execute it, or the executor cannot read the project files.
5. Read project routing docs when present, otherwise read `references/task-routing.md`.
6. Read project runtime adapter docs when present, otherwise use the runtime defaults in this skill.
7. Select `Executor`, `Runtime adapter`, `Execution surface`, `Permission mode`, `Parallelism`, and `Review gate`.
8. Classify the task as `light`, `medium`, or `heavy`.
9. Fill the selected project task card or global template with concrete project facts.
10. Ensure the prompt requires the executor to use `claude-delivery-report` or the project's delivery-report protocol before its final response.
11. Add only directly relevant skill tags.
12. Output a Claude Code executable compact task card by default, unless the
    user explicitly asks for a human-only preview.

## Classification Standard

Use the smallest template that still captures the risk.

Escalate to `heavy` when the task touches data stores, historical outputs, vector stores, migrations, baseline preservation, deletion, overwrites, irreversible operations, staged rollout, audit evidence, or broad architecture changes.

Use `medium` for multi-file behavior changes, shared helpers, configuration changes, nontrivial bug fixes, or local refactors that need a short plan.

Use `light` for narrow, low-risk, directly executable changes.

## Runtime Adapter Standard

Select runtime fields before filling task-specific details.

Default mapping:

- User asks "you execute", "你直接做", or equivalent: `Executor: Codex`, `Runtime adapter: codex-local`, `Execution surface: local-workspace`.
- User asks for a Claude Code prompt or task card: `Executor: Claude Code`, `Runtime adapter: claude-code`, `Execution surface: cli`.
- User asks for Cursor execution or a Cursor task card: `Executor: Cursor`, `Runtime adapter: cursor`, `Execution surface: ide`.
- Unknown external executor: `Executor: Other`, `Runtime adapter: generic`.

Default permission by task level:

- `Light` -> `Permission mode: execute-and-verify`
- `Medium` -> `Permission mode: edit-with-confirmation`
- `Heavy` -> `Permission mode: plan-only`

For Heavy tasks, `plan-only` is binding until explicit human approval for the
current task. Only after that approval may a task card move to
`edit-with-confirmation` or `execute-and-verify`.

When a request is a continuation, context-compression recovery,
task-notification follow-up, or says "继续", Heavy task cards must require the
executor to reread the task card, run `git status --short`, reconfirm
`review_targets`, and stop at the confirmation gate unless mutation approval is
explicit in the current context.

Default `Parallelism` is `none`. Use `subagent`, `worktree`, `multi-session`, or `agent-team` only when the user or task card explicitly allows it and the work scopes do not overlap.

## Project Overrides

When a project contains its own agent workflow docs, treat them as project-specific overrides. Common locations:

- `config/agent-project-profile.yaml`
- `docs/agent-workflow/agent-task-protocol.md`
- `docs/agent-workflow/task-routing.md`
- `docs/agent-workflow/runtime-adapters.md`
- `docs/agent-workflow/project-profile.md`
- `docs/agent-workflow/context-memory.md`
- `docs/agent-workflow/task-card-template.md`
- `docs/agent-workflow/templates/*.md`
- `docs/agent-workflow/delivery-report-standard.md`

Project overrides can add paths, commands, domain constraints, and safety rules. They should not remove global safety requirements unless the user explicitly says so.

## Project Profile

Use `config/agent-project-profile.yaml` as the preferred source for stable
project defaults. It may provide verification commands, high-risk paths,
protected paths, default executor preferences, review strictness, and user
preferences.

Profile rules:

- Use profile facts only to fill existing task-card slots.
- Do not paste the whole profile into the task card.
- Prefer `项目画像：- config/agent-project-profile.yaml` plus short derived facts
  under `背景`, `实施要求`, or `验证`.
- If the profile is absent, continue from repo evidence and the user's request.
- If the profile conflicts with current repo evidence, use current evidence and
  mention the mismatch.
- Never create a new task-card template because of profile content.

## Context Memory

Use local context memory only as a continuity source. The preferred capsule is:

```text
$HOME/.agents/memory/projects/<project-slug>/context-capsule.md
```

Memory rules:

- Put the capsule path in `记忆胶囊` when it exists or when the task expects it.
- Use `无` when no capsule exists or the executor cannot access it.
- Do not paste long memory into the task card.
- When a capsule is used, require the executor to read sibling
  `task-memory.md` if present before starting.
- Treat `context-capsule.md` as the manual project charter. Its `## 项目设计目的`
  and human-maintained boundaries must not be overwritten by runner / hook /
  capture / automatic summaries.
- If the requested task conflicts with `## 项目设计目的`, the executor must stop
  and report.
- If memory conflicts with the current request, task card, or live repository
  evidence, current evidence wins.
- Memory is never approval for Heavy mutation.
- Reusable ideas from memory should become proposals first, not automatic rule
  or skill changes.

## Task Archives

Task-archive rules:

- Fill `任务存档` with the local `task-memory.md` path when it exists, otherwise
  use `无`.
- When handing off through the runner, prefer `scripts/run-task-card.sh --memory`
  so the receipt is archived under
  `$HOME/.agents/memory/projects/<project-slug>/task-archive/` and
  `$HOME/.agents/memory/projects/<project-slug>/task-memory.md` is refreshed.
- Do not store task archives in the project repository.
- Do not paste archived logs into new task cards; cite the archive path and
  summarize only the task-relevant fact.

## Template Selection

The output must always be one of exactly two task-card formats. Never create a third format.

1. **Project task card** — use the fixed skeleton from `docs/agent-workflow/task-card-template.md` when it exists.
2. **Global fallback task card** — use `templates/fallback-task-cards/{light,medium,heavy}.md` when no project protocol exists, the task is cross-repo, or the executor cannot access project files.

Prohibited: free-form runbooks, machine-specific full templates, phase-specific templates, tool-specific formats, or any format that is neither a project task card nor a global fallback task card.

Prefer a project task card over a global full prompt when the executor can read the repository files. The task card should reference the project's protocol files instead of repeating all fixed rules.

Do not create or select a third category of specialized full task cards.

For cache stability, keep the final task-card skeleton stable:

- use the same section titles,
- keep sections in the same order,
- keep baseline wording stable,
- put dynamic facts only in the fixed slots,
- put domain-specific additions under `实施要求`, `适用治理文档`, `相关路径`, `验证`, `Review gate`, or `Verification gate`.
- put profile references in `项目画像`; do not change the skeleton when the
  profile is absent.
- put memory capsule references in `记忆胶囊`; do not paste long memory.
- put the `task-memory.md` reference in `任务存档`; do not paste historical logs.

Use the global full templates only as a fallback:

- no project task-card protocol exists,
- the executor cannot access the project files,
- the execution target is outside the repository,
- the prompt must be self-contained for an external agent.

## Review Target Rules

Do not assume Claude Code's startup `cwd` is the repository being modified. In SSH, remote-control, mounted-volume, or cross-repo workflows, the hook cwd may be an entrypoint repo while the actual edits happen elsewhere.

When the actual modified repo may differ from the startup cwd, the generated task card must require Claude Code to rewrite `.claude/review_targets.json` in the startup cwd before making changes:

```json
{
  "task_level": "Light / Medium / Heavy",
  "targets": [
    {
      "name": "<repo-name>",
      "path": "<absolute path to actual repo>",
      "level": "Light / Medium / Heavy"
    }
  ]
}
```

Use one target entry per git repository that may be modified. The file is per-task state, so each task card must require rewriting it instead of relying on a stale file. Explicit review scope depends on these targets. If the target repository path is unknown, the task card must tell Claude Code to stop and report instead of continuing.

## Output Rules

The suite's design goal is **frontstage natural language, backstage
cache-stable task card**.

Default to a **compact executable Claude Code task card** when the user asks for
"任务卡", "Claude Code 任务卡", "给 CC 确认", "确认一下这个任务卡", or equivalent.
The user usually intends to paste this into Claude Code so Claude can confirm
or execute. It must be executable by Claude Code, not a human-only preview.

Compact executable card rules:

- It is still a real task card for Claude Code.
- Keep it short enough for frontstage review, usually 20-40 lines.
- Start with the fixed cache anchor exactly as shown. Do not put path, date,
  user request, or other dynamic content before it.
- Use the compact skeleton below exactly: keep headings, field order, and
  baseline wording stable; fill only the dynamic slots.
- Preserve the required executable fields: execution path, Executor,
  Runtime adapter, Permission mode, Parallelism, task level, goal, non-goals,
  key paths/context, verification gate, delivery requirement, and skill tags.
- Reference protocol files instead of pasting boilerplate review-gate text,
  runtime explanations, long JSON examples, or repeated policy paragraphs.
- Use concise bullets and path references. Do not expand the entire fixed
  skeleton unless needed.

Compact skeleton:

```markdown
AGENT_SUITE_COMPACT_TASK_CARD_V1
遵循固定字段顺序，只填动态 slot。

路径：<absolute project path>

Executor: Claude Code
Runtime adapter: claude-code
Execution surface: cli
Permission mode: <read-only / plan-only / edit-with-confirmation / execute-and-verify>
Parallelism: none
任务级别：<Light / Medium / Heavy>

读取：
- <protocol_or_project_doc>
- <memory_capsule_or_context_path>

任务：
<one sentence>

目标：
1. <goal>
2. <goal>

非目标：
- <non-goal>

关键路径：
- <path>
- <path>

验证：
- <command or evidence check>

停止条件：
- <when Claude Code must stop and report>

交付：
按 claude-delivery-report 输出：状态、证据、风险、下一步。

[skill: verify]
```

For `Medium` add `[skill: review]` only when the project explicitly routes
manual review through that tag. For `Heavy`, keep `Permission mode: plan-only`
unless the current user request explicitly approves mutation.

Generate the long/full fixed skeleton only when one of these is true:

- the user explicitly asks for a "完整任务卡", "可复制给 Claude Code", "直接发给 CC 执行",
  "完整骨架", "full prompt", or equivalent;
- a runner or file artifact needs the full skeleton;
- the executor cannot access project protocol files and needs a self-contained
  fallback card.

Generate a human-only preview only when the user explicitly says "只给我摘要",
"先不要给 Claude Code 执行卡", "只给确认版不要执行", or equivalent.

Do not make compact cards a third protocol family. They are the project/fallback
task-card format with boilerplate collapsed into protocol references.

For compact executable task-card delivery, the final answer should contain:

1. The execution path or repository root.
2. One compact fenced `markdown` task card that Claude Code can run or confirm.
3. No long explanation before the card.

For long/full execution delivery, the final answer should contain:

1. A one-line classification:
   - `任务级别：Light / Medium / Heavy`
   - `Review gate`
   - `使用模板：...`
   - `Executor / Runtime adapter / Permission mode：...`
2. A short reason for the classification.
3. The final execution prompt or task card in one uninterrupted fenced `markdown` block.

Dialogue delivery is the default. Do not create a task-card `.md` file unless
the user explicitly asks for a file artifact. When generating a compact or full
execution card, keep it contiguous and free of explanatory text inside the
fenced block.

When the task card contains any inner fenced code block such as ` ```json ` for `.claude/review_targets.json` or ` ```bash ` for command examples, wrap the whole task card with `~~~~markdown` and closing `~~~~`. Do not use outer triple backticks for these task cards, because the first inner triple-backtick block will prematurely close the outer block.

Do not split the task card by workflow stage, partial section, or separate code blocks. Put all required sections, fields, verification gates, and skill tags in the same fenced `markdown` block.

Do not include a long explanation before the prompt.

## Skill Tags

Use manual tags only for skills the executor should explicitly load.

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

Before returning a compact executable task card, check:

- It is executable by Claude Code.
- It names the execution path, executor/runtime/permission/task level, goal,
  non-goals, key paths, and verification evidence.
- It starts with the exact compact cache anchor.
- It follows the compact skeleton heading order exactly.
- It is short enough for a human to approve without reading boilerplate.
- It references protocol files instead of pasting long fixed rules.
- It includes delivery/report requirements and relevant skill tags.

Before returning a human-only preview, check:

- The user explicitly asked for a non-executable preview.
- It does not pretend to be a Claude Code execution card.

Before returning the full final prompt, check:

- The output is exactly one of the two allowed task-card formats (project or global fallback). If it matches neither, redo from Template Selection.
- The task card includes `Executor`, `Runtime adapter`, `Execution surface`, `Permission mode`, `Parallelism`, `Review gate`, and `Verification gate`.
- The task card includes `项目画像`, with `无` or `config/agent-project-profile.yaml`.
- The task card includes `记忆胶囊`, with `无` or a local capsule path.
- If `记忆胶囊` is a capsule path, the card reminds the executor to read sibling
  `task-memory.md` when present.
- The task card includes `任务存档`, with `无` or a local `task-memory.md` path.
- The prompt has clear goals and non-goals.
- Hard constraints are explicit.
- Relevant paths and modules are named when known.
- Remote-control or cross-repo prompts include `.claude/review_targets.json` requirements with absolute target repo paths.
- Verification commands are included or the prompt asks the executor to identify them.
- The prompt requires `claude-delivery-report` or the project's delivery-report protocol for the final task report.
- Destructive actions are blocked unless explicitly approved.
- Heavy tasks include a confirmation gate before mutation.
