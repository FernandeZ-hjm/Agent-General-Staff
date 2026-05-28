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

Permission mode: execute-and-verify

Parallelism: none

任务级别：Light

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
- <path>

本次任务相关文件：
- <file>

适用治理文档：
- 无

目标：
1. <goal>

非目标：
- <non-goal>

实施要求：
- 直接 read → execute → verify
- 不改数据、向量库、历史产物
- 如果启动目录不是实际修改仓库，或任务会触碰 cwd 外仓库，先写入 `.claude/review_targets.json`，声明实际目标仓库和 Light 级别；无法确认目标仓库时停止并报告。

验证：
Verification gate:
- commands:
  - <verification command>
- expected evidence:
  - <test result / diff summary / delivery report>
- stop condition:
  - risk higher than Light / destructive action / missing verification

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。
如执行中发现任务应升级为 Medium 或 Heavy，交付报告必须提醒操作者手动触发对应 `/codex:review` 或 `/codex:adversarial-review` 后再放行。

[skill: verify]
