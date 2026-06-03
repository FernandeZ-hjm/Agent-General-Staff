# Heavy Agent Task Card Template

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

Permission mode: plan-only

Parallelism: none

任务级别：Heavy

Heavy 默认 `Permission mode: plan-only`。只有当前任务获得明确人工确认后，才可进入 `edit-with-confirmation` 或 `execute-and-verify`；"继续"、上下文压缩恢复或 task-notification 接续不算 Heavy 写入批准。

Review gate:
- Light：完成验证后运行 `caveman-review` 或等价轻量 diff review；如发现风险高于 Light，升级 Medium。
- Medium：Codex 最终 Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待 Codex review”，由 Codex 审查通过后再放行。
- Heavy：先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 adversarial review”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。

任务：
{one_sentence_task_summary}

背景：
{why_this_task_exists}

项目画像：
- 无 / `config/agent-project-profile.yaml`

记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
- 若存在，同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

相关路径：
- `{path_1}`
- `{path_2}`
- `{path_3}`

本次任务相关文件：
- `{task_file_or_doc_1}`
- `{task_file_or_doc_2}`
- `{task_file_or_doc_3}`

适用治理文档：
- 无 / `{governance_doc}`

目标：
1. {goal_1}
2. {goal_2}
3. {goal_3}
4. {goal_4}

非目标：
- {non_goal_1}
- {non_goal_2}
- {non_goal_3}

实施要求：
- 先阅读现有代码、目录结构、配置、现有测试和文档。
- 先输出 root cause / 当前结构理解 / 风险点 / 设计方案 / 实施计划 / 验证计划。
- 等待确认后再改代码或生成新产物。
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`；当前上下文没有明确人工批准 mutation 时，停在 plan / confirmation gate。
- 不安装新依赖，除非先说明必要性并等待确认。
- 不做破坏性删除；所有删除类动作必须先实现为 quarantine / disable / exclude / dry-run。
- 涉及数据、向量库、历史产物、索引、collection 或 baseline 时，必须保证可回滚、可审计、可对比。
- 所有自动判断必须留下审计证据。
- 如发现需求与现有代码事实不一致，先报告，不要自行扩大范围。

验证：
Verification gate:
- commands:
  - `{verification_command_1}`
  - `{verification_command_2}`
  - `{verification_command_3}`
- expected evidence:
  - root cause / design / implementation plan / verification plan
  - dry-run or test result after approved execution
- stop condition:
  - before mutation, wait for user confirmation / baseline mutation / destructive action / missing verification

交付：
完成后必须先读取并使用 `claude-delivery-report` skill 或项目交付报告协议，按其模板输出简洁交付报告。
Heavy 任务在人工 adversarial review 完成前，交付报告必须标为“部分完成”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。

如果本任务生成了详细审计报告、统计结果、manifest 或长日志，不要把全部内容塞进主报告；在主报告的“新增文件 / 输出物”或“风险提示”中引用对应文件即可。

{skill_tags}
~~~~
