# Cursor Skill Index

Cursor cannot directly inherit every external agent runtime tool, but it can reuse project rules, local skill files, and task protocols. In this suite, Cursor is a primary development agent and should choose skills proactively.

When a task references a skill, open only the relevant SKILL.md. Otherwise read the preflight-bound SkillCard catalog and select the smallest exact capability from the complete conversation context; do not classify by keywords.

## Operating Rules

- Paths in this document that contain `$HOME` are template paths. Before opening any SKILL.md, expand `$HOME` to the actual home directory of the current machine.

- Bounded content compression, summary, translation, formatting, approved-field
  normalization, and approved-template conversion use `direct-response`; do not
  load an engineering workflow skill.
- Use the `superpowers` parent with the `brainstorming` entrypoint only for an
  explicit brainstorm or unresolved system/cross-module architecture boundary.
- Use one precise specialist when its task characteristic is present:
  `codebase-design` (module interface/testability), `domain-modeling` (domain
  terms/model), `improve-codebase-architecture` (architecture debt), `prototype`
  (throwaway experiment), or `grilling` (pressure-test an existing plan).
- Use `grill-with-docs` only when the user explicitly requests a
  documentation/ADR/glossary-grounded alignment interview; it is manual-only.
- Use `diagnosing-bugs` for errors, failing tests, broken behavior, performance issues, or unexpected runtime output.
- Use the `superpowers` parent with the `verification-before-completion`
  entrypoint before claiming work is complete, fixed, passing, or ready to hand off.
- Use manual skills only when they directly reduce risk for the current task.
- Do not stack unrelated skills. Prefer one or two strong procedural guardrails over a long list.
- If a skill file is missing or unreadable, report that briefly and continue with the nearest safe workflow.
- For delegated Claude Code CLI work, follow `protocol/agent-task-protocol.md` and use `protocol/task-card-template.md`.
- Include only optional `[skill: xxx]` tags precisely selected by Skill Resolver
  / the confirmed contract; no skill tag is required by task level. Require
  a delivery report independently.
- Task-card tags name host-visible capability bodies. For Superpowers workflows,
  use `[skill: superpowers]` once and name required internal playbooks such as
  `verification-before-completion` or `test-driven-development` in the task body;
  child playbook names are not standalone task-card tags.

## Exact capability routing

The host emits an exact `SkillTarget` with `skill_id`, optional entrypoint, and
snapshot hash. Skill Resolver validates it against the `ActiveSkillTable`.
Missing skills return unavailable; alternatives are never auto-selected.
Superpowers playbooks remain internal entrypoints of one host-visible parent.

## Manual skills

- diagnosing-bugs: $HOME/.agents/skills/diagnosing-bugs/SKILL.md
- codebase-design: $HOME/.agents/skills/codebase-design/SKILL.md
- domain-modeling: $HOME/.agents/skills/domain-modeling/SKILL.md
- grill-with-docs: $HOME/.agents/skills/grill-with-docs/SKILL.md
- grilling: $HOME/.agents/skills/grilling/SKILL.md
- improve-codebase-architecture: $HOME/.agents/skills/improve-codebase-architecture/SKILL.md
- prototype: $HOME/.agents/skills/prototype/SKILL.md
- superpowers: $HOME/.agents/skills/superpowers/SKILL.md — the only host-visible
  Superpowers body; internal `PLAYBOOK.md` resources are selected through registry
  route targets and are not independently host-discoverable skills.
- webapp-testing: $HOME/.agents/skills/webapp-testing/SKILL.md
- database-migration: $HOME/.agents/skills/database-migration/SKILL.md
- supply-chain-risk-auditor: $HOME/.agents/skills/supply-chain-risk-auditor/SKILL.md
- graphify: $HOME/.agents/skills/graphify/SKILL.md — use when the user asks for Graphify, `/graphify`, 项目图谱, 项目知识图谱, 代码图谱, 架构图谱, or a graph-based project map.

## Continuation prompt template

**This is a state-carrying handoff wrapper, not a task execution card.** It carries session context across boundaries (agent handoffs, truncated sessions, continuation requests). When actual task execution is required, the continuation context must be rewrapped into the single canonical task-card skeleton (`protocol/task-card-template.md`) — never executed directly from this template.

Use this when Cursor needs to hand work to Claude Code CLI, another agent, or a later session:

```md
请接续开发当前任务。

必须先读取：
- $HOME/.agents/rules/SOUL.md
- AGENTS.md
- CLAUDE.md
- protocol/task-routing.md
- protocol/cursor-skill-index.md

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
- 按任务卡 Permission mode 执行：`plan-only` 只诊断和出计划，
  `execute-and-verify` 直接执行并验证；Heavy 不另行强制计划
- 完成后运行相关验证
- 接续后由宿主用完整上下文与 current-host catalog 生成 typed proposal，再调用只读
  `ags_route_request`；验证是协议 gate，仅当 Skill Resolver / 原任务卡精确选择 `superpowers` 时才读取其
  `verification-before-completion` playbook。Continuation 自由文本不得携带
  `[skill: ...]` 元数据。
```
