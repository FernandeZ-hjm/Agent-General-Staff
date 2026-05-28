# Example Task Card: Claude Code Medium

```markdown
## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/runtime-adapters.md
- docs/agent-workflow/cursor-skill-index.md

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: cli

Permission mode: edit-with-confirmation

Parallelism: none

任务级别：Medium

任务：
为一个已有脚本增加只读 `--dry-run` 输出摘要，并补充对应验证命令。

背景：
这是一个 Claude Code handoff 样例。任务跨脚本和测试说明，要求先说明设计，再在范围内实现。

相关路径：
- `scripts/example-tool.sh`
- `tests/example-tool.bats`

本次任务相关文件：
- `docs/agent-workflow/agent-task-protocol.md`
- `docs/agent-workflow/runtime-adapters.md`

适用治理文档：
- 无

目标：
1. 增加 `--dry-run` 参数，输出将要执行的动作，不写文件。
2. 补充或更新最窄相关测试。
3. 交付报告列出实际验证命令和结果。

非目标：
- 不改变默认执行行为。
- 不安装新依赖。
- 不修改无关脚本。

实施要求：
- 先阅读相关脚本和测试。
- 先给简短 root cause / 当前行为 / 修改方案，再开始改代码。
- 如发现脚本不存在、测试框架不可用或风险超过 Medium，停止并报告。

验证：
Verification gate:
- commands:
  - `bash -n scripts/example-tool.sh`
  - `bats tests/example-tool.bats`
- expected evidence:
  - shell syntax check 通过
  - dry-run 行为测试通过
  - delivery report 包含 diff 摘要
- stop condition:
  - 需要写外部状态 / 需要安装依赖 / 风险高于 Medium / 缺少可验证路径

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。

[skill: diagnose]
[skill: verify]
```
