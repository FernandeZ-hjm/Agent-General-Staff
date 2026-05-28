## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/runtime-adapters.md
- docs/agent-workflow/cursor-skill-index.md

Executor: <Codex / Claude Code / Cursor / Human / Other>

Runtime adapter: <codex-local / claude-code / cursor / generic>

Execution surface: <local-workspace / cli / ide / web / remote-control / background-agent>

Permission mode: plan-only

Parallelism: none

任务级别：Heavy

Heavy 写入批准规则按 docs/agent-workflow/agent-task-protocol.md 执行；“继续”、上下文压缩恢复或 task-notification 接续不算 Heavy 写入批准。

Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

任务：
<一句话任务描述>

背景：
<只写本次任务差异，不重复长期协议。说明涉及的数据、历史产物和风险范围。>

项目画像：
- 无 / `config/agent-project-profile.yaml`

记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
- 若存在，同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

相关路径：
- <path_1>
- <path_2>

本次任务相关文件：
- <file_1>
- <file_2>

适用治理文档：
- <governance doc>

目标：
1. <goal_1>
2. <goal_2>

非目标：
- <non-goal_1>
- <non-goal_2>

实施流程：
1. 阅读与诊断 → 输出 root cause / 设计 / 计划 → 等待确认
2. 确认后执行
3. 验证与交付

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 当前上下文没有明确人工批准 mutation 时，停在 plan / confirmation gate。
- 不得把“继续”理解为 Heavy 写入批准。

基线保护：
- 不修改、删除、覆盖: <受保护数据/目录>

实施要求：
- 先输出 root cause / design / implementation plan / verification plan
- 等待用户确认后再改代码
- 数据操作必须 dry-run 先行
- 保持旧基线不动
- 如果启动目录不是实际修改仓库，或任务会触碰 cwd 外仓库，先写入 `.claude/review_targets.json`，声明所有实际目标仓库和 Heavy 级别；无法确认目标仓库时停止并报告。

验证：
Verification gate:
- commands:
  - <verification command>
- expected evidence:
  - root cause / design / implementation plan / verification plan
  - <dry-run or test result after approved execution>
- stop condition:
  - before mutation, wait for user confirmation / baseline mutation / destructive action / missing verification

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。报告必须包含：
- root cause / 设计摘要
- 改动内容
- 验证结果
- 是否触碰基线数据
- 风险提示
- 下一步建议
Review gate 状态按 docs/agent-workflow/agent-task-protocol.md 报告。

[skill: diagnose]
[skill: zoom-out]
[skill: verify]
