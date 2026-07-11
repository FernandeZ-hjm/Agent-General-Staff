## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- protocol/agent-task-protocol.md
- protocol/task-routing.md
- protocol/runtime-adapters.md
- protocol/project-profile.md
- protocol/context-memory.md
- protocol/cursor-skill-index.md

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: cli

Permission mode: execute-and-verify

Parallelism: none

任务级别：Medium

Review gate:
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

任务：
补齐 AGS public edition 的文档传播层——在 docs/ 下新增 docs/public-distribution-checklist.md。

背景：
AGS 公开版在 protocol/ 和 crates/ 方面已经完整，但缺少面向公开分发维护者的操作文档。
本次任务在 docs/ 下新增一份简洁的分发前检查清单，列出发布新版本时需要检查的项目。

项目画像：
- 无

记忆胶囊：
- 无

任务存档：
- 无

相关路径：
- docs/
- AGENT_SUITE_PROTOCOL.md
- protocol/
- scripts/verify.sh
- LICENSE
- README.md

本次任务相关文件：
- docs/
- README.md
- LICENSE

目标：
1. 新增 docs/public-distribution-checklist.md，包含以下检查项：
   - Rust 构建通过（cargo build --release）
   - 测试全部通过（cargo test）
   - 验证通过（bash scripts/verify.sh）
   - protocol 文件全部存在且无漂移
   - examples/ 样例可独立校验
   - LICENSE 和 NOTICE.md 完整
   - 无私有路径、密钥、token、真实任务记忆泄露
2. 文档使用中文，格式为 markdown checklist
3. 不修改 crates/ 下的 Rust 内核代码

非目标：
- 不修改 protocol/ 下的 canonical 协议规则
- 不修改 LICENSE 正文
- 不运行安装命令，不安装第三方技能或依赖
- 不生成或提交真实任务记忆、真实交付报告、真实 receipt

实施要求：
- 在执行记录中先给简短设计说明（文档结构 + 检查项列表），随后直接写入并验证，不追加确认轮次
- 检查项约 10-15 条，覆盖构建、测试、协议、边界、样例

验证：
Verification gate:
- commands:
  - bash scripts/verify.sh
  - git diff --stat
- expected evidence:
  - 新增文件 docs/public-distribution-checklist.md
  - verify.sh 通过
- stop condition:
  - 需要修改 protocol/ 或 LICENSE 时停止并报告
  - verify.sh 失败且无法在当前范围内修复时停止

交付：
按 protocol/agent-task-protocol.md 输出交付报告。
Medium Review gate：实现和验证完成后，任务状态写为"部分完成 / 等待 Codex review"。

[skill: codebase-design]
[skill: verification-before-completion]
