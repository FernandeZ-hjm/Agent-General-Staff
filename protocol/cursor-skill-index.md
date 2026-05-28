# Cursor Skill Index

Cursor cannot directly inherit every external agent runtime tool, but it can reuse project rules, local skill files, and task protocols. In this suite, Cursor is a primary development agent and should choose skills proactively.

When a task references a skill, open only the relevant SKILL.md. When a task does not reference a skill, choose the smallest relevant set from this index after classifying the task with `docs/agent-workflow/task-routing.md`.

## Operating Rules

- Paths in this document that contain `$HOME` are template paths. Before opening any SKILL.md, expand `$HOME` to the actual home directory of the current machine.

- Use `auto-brainstorm` for open-ended feature, design, architecture, or recommendation work before implementing.
- Use `auto-debug` for errors, failing tests, broken behavior, performance issues, or unexpected runtime output.
- Use `auto-verify` before claiming work is complete, fixed, passing, or ready to hand off.
- Use manual skills only when they directly reduce risk for the current task.
- Do not stack unrelated skills. Prefer one or two strong procedural guardrails over a long list.
- If a skill file is missing or unreadable, report that briefly and continue with the nearest safe workflow.
- For delegated Claude Code CLI work, follow `docs/agent-workflow/agent-task-protocol.md` and use `docs/agent-workflow/task-card-template.md`.
- Include the required `[skill: xxx]` tags in the task card and require a delivery report.

## Auto-trigger skills

- auto-brainstorm: $HOME/.agents/skills/auto-brainstorm/SKILL.md
- auto-debug: $HOME/.agents/skills/auto-debug/SKILL.md
- auto-verify: $HOME/.agents/skills/auto-verify/SKILL.md

## Manual skills

- tdd: $HOME/.agents/skills/tdd/SKILL.md
- diagnose: $HOME/.agents/skills/diagnose/SKILL.md
- grill-with-docs: $HOME/.agents/skills/grill-with-docs/SKILL.md
- improve-codebase-architecture: $HOME/.agents/skills/improve-codebase-architecture/SKILL.md
- prototype: $HOME/.agents/skills/prototype/SKILL.md
- zoom-out: $HOME/.agents/skills/zoom-out/SKILL.md
- caveman-commit: $HOME/.agents/skills/caveman-commit/SKILL.md
- caveman-review: $HOME/.agents/skills/caveman-review/SKILL.md
- finishing-a-development-branch: $HOME/.agents/skills/finishing-a-development-branch/SKILL.md
- using-git-worktrees: $HOME/.agents/skills/using-git-worktrees/SKILL.md
- webapp-testing: $HOME/.agents/skills/webapp-testing/SKILL.md
- database-migration: $HOME/.agents/skills/database-migration/SKILL.md
- supply-chain-risk-auditor: $HOME/.agents/skills/supply-chain-risk-auditor/SKILL.md
- skill-creator: $HOME/.agents/skills/skill-creator/SKILL.md
- graphify: $HOME/.agents/skills/graphify/SKILL.md — use when the user asks for Graphify, `/graphify`, 项目图谱, 项目知识图谱, 代码图谱, 架构图谱, or a graph-based project map.

## Continuation prompt template

**This is a state-carrying handoff wrapper, not a task execution card.** It carries session context across boundaries (agent handoffs, truncated sessions, continuation requests). When actual task execution is required, the continuation context must be rewrapped into either a project task card or a global fallback task card — never executed directly from this template.

Use this when Cursor needs to hand work to Claude Code CLI, another agent, or a later session:

```md
请接续开发当前任务。

必须先读取：
- $HOME/.agents/rules/SOUL.md
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/cursor-skill-index.md

当前状态：
- 分支：
- 已改文件：
- 任务分级：light / medium / heavy
- 未完成目标：
- 已验证命令：
- 失败/风险：
- 下一步建议：

执行要求：
- 先 git status --short
- 不读取或打印密钥
- 保持现有架构约定
- 按 light / medium / heavy 分级执行
- heavy 任务先给 root cause / 设计 / 计划，等确认再修改
- 完成后运行相关验证

[skill: verify]
```
