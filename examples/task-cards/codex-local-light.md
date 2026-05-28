# Example Task Card: Codex Local Light

```markdown
## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/runtime-adapters.md
- docs/agent-workflow/cursor-skill-index.md

Executor: Codex

Runtime adapter: codex-local

Execution surface: local-workspace

Permission mode: execute-and-verify

Parallelism: none

任务级别：Light

任务：
修正文档中的一个错别字，并验证 diff 只包含该文档改动。

背景：
这是一个低风险本地执行样例，用于展示用户说“你直接做”时的 Codex-local 路由。

相关路径：
- `docs/example.md`

本次任务相关文件：
- `docs/example.md`

适用治理文档：
- 无

目标：
1. 只修正文档错别字。
2. 保持任务范围外文件不变。

非目标：
- 不修改代码。
- 不运行依赖安装。
- 不提交或推送。

实施要求：
- 修改前运行 `git status --short`。
- 只做最小文本改动。
- 不处理无关 dirty changes。

验证：
Verification gate:
- commands:
  - `git diff -- docs/example.md`
  - `git diff --check`
- expected evidence:
  - diff 只包含目标文档的错别字修正
  - whitespace 检查通过
- stop condition:
  - 目标文件存在无关用户改动 / diff 超出文档范围 / 验证命令不可用

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。

[skill: verify]
```
