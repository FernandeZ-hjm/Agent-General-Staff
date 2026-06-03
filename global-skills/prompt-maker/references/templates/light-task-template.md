# Light Agent Task Card Template

Use this global fallback template only when no project task-card protocol is available, the task is cross-repo, an external agent will execute it, or the executor cannot read project files.

---

~~~~markdown
## 任务卡

读取并遵守：
- 本任务卡内的约束
- 当前工作目录中的 AGENTS.md / CLAUDE.md / README / CONTRIBUTING，如存在
- 当前任务相关文件

Executor: {executor}

Runtime adapter: {runtime_adapter}

Execution surface: {execution_surface}

Permission mode: execute-and-verify

Parallelism: none

任务级别：Light

Review gate:
- Light：完成验证后运行 `caveman-review` 或等价轻量 diff review；如发现风险高于 Light，升级 Medium。
- Medium：Codex 最终 Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待 Codex review”，由 Codex 审查通过后再放行。
- Heavy：先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 adversarial review”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。

任务：
{one_sentence_task_summary}

背景：
{brief_context}

项目画像：
- 无 / `config/agent-project-profile.yaml`

记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
- 若存在，同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

相关路径：
- `{path_or_module_1}`
- `{path_or_module_2}`

本次任务相关文件：
- `{task_file_or_doc_1}`
- `{task_file_or_doc_2}`

适用治理文档：
- 无 / `{governance_doc}`

目标：
1. {goal_1}
2. {goal_2}

非目标：
- {non_goal_1}
- {non_goal_2}

实施要求：
- 先阅读相关文件，确认当前实现。
- 只做完成目标所需的最小改动。
- 不顺手重构无关代码。
- 不安装新依赖。
- 如发现实际代码与任务描述不一致，先报告再继续。

验证：
Verification gate:
- commands:
  - `{verification_command_1}`
  - `{verification_command_2}`
- expected evidence:
  - command result + diff summary + delivery report
- stop condition:
  - risk higher than Light / destructive action / missing verification

交付：
完成后必须先读取并使用 `claude-delivery-report` skill 或项目交付报告协议，按其模板输出简洁交付报告。
如执行中发现任务应升级为 Medium 或 Heavy，交付报告必须提醒操作者等待 Codex review 或手动触发 `/codex:adversarial-review` 后再放行。

{skill_tags}
~~~~
