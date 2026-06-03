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

Permission mode: edit-with-confirmation

Parallelism: none

任务级别：Medium

Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

任务：
<一句话任务描述>

背景：
<只写本次任务差异，不重复长期协议>

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
- 无 / <governance doc>

目标：
1. <goal_1>
2. <goal_2>

非目标：
- <non-goal_1>
- <non-goal_2>

实施要求：
- 先给简短 root cause 或 design note
- 计划清楚后 execute → verify；如触发 stop condition，先报告并等待确认
- 跨文件改动注意边界影响
- 如果启动目录不是实际修改仓库，或任务会触碰 cwd 外仓库，先写入 `.claude/review_targets.json`，声明实际目标仓库和 Medium 级别；无法确认目标仓库时停止并报告。

验证：
Verification gate:
- commands:
  - <verification command>
- expected evidence:
  - <test result / diff summary / delivery report>
- stop condition:
  - risk higher than Medium / destructive action / unclear scope / missing verification

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。
Codex review 完成前，交付报告必须标为“部分完成 / 等待 Codex review”。

[skill: verify]
