## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/runtime-adapters.md
- docs/agent-workflow/cursor-skill-index.md

Executor: Cursor

Runtime adapter: cursor

Execution surface: ide

Permission mode: plan-only

Parallelism: none

任务级别：Heavy

Heavy 默认 `Permission mode: plan-only`。只有当前任务获得明确人工确认后，才可进入 `edit-with-confirmation` 或 `execute-and-verify`；“继续”、上下文压缩恢复或 task-notification 接续不算 Heavy 写入批准。

Review gate:
- Light：完成前自查 diff；提交前建议运行 `caveman-review`。
- Medium：人工 Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 review”，并提醒操作者手动运行 `/codex:review` 后再放行。
- Heavy：先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 adversarial review”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。

任务：
评估是否可以重构一条历史数据处理管线，并给出实施计划与验证计划。

背景：
这是一个 Cursor IDE workflow 样例。任务涉及历史产物和潜在基线变更，因此只能先计划，不得直接修改。

项目画像：
- 无 / `config/agent-project-profile.yaml`

记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
- 若存在，同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

相关路径：
- `scripts/example-pipeline.py`
- `data/example-baseline/`
- `docs/pipeline-notes.md`

本次任务相关文件：
- `docs/agent-workflow/agent-task-protocol.md`
- `docs/agent-workflow/task-routing.md`
- `docs/agent-workflow/runtime-adapters.md`

适用治理文档：
- 无 / `docs/agent-workflow/data-safety.md`

目标：
1. 说明当前管线结构和风险点。
2. 给出 root cause / design / implementation plan / verification plan。
3. 明确哪些文件或数据必须保持只读。

非目标：
- 不改代码。
- 不重跑历史数据处理。
- 不删除、覆盖、迁移 baseline。
- 不创建提交或 PR。

实施流程：
1. 阅读与诊断 → 输出 root cause / 设计 / 计划 → 等待确认。
2. 确认后才允许生成后续执行任务卡。

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 当前上下文没有明确人工批准 mutation 时，停在 plan / confirmation gate。
- 不得把“继续”理解为 Heavy 写入批准。

基线保护：
- 不修改、删除、覆盖: `data/example-baseline/`

实施要求：
- 使用 IDE 上下文辅助理解，但结论必须引用文件、diff、命令或文档证据。
- 如发现需要实际数据变更，停止并要求新任务卡授权。
- 不扩大到未列出的管线。

验证：
Verification gate:
- commands:
  - `git status --short`
  - `python -m py_compile scripts/example-pipeline.py`
- expected evidence:
  - 当前 dirty state 摘要
  - root cause / design / implementation plan / verification plan
  - baseline 保护清单
- stop condition:
  - 任何 mutation 需求 / baseline 风险不清 / 验证命令不可用 / 需要跨模块重构授权

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。报告必须包含：
- root cause / 设计摘要
- 是否触碰基线数据（预期必须为否）
- 验证结果
- 风险提示
- 下一步建议

[skill: diagnose]
[skill: zoom-out]
[skill: verify]
