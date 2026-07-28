## 任务卡

AGENT_SUITE_COMPACT_TASK_CARD_V1

读取并遵守：
- AGENTS.md

Contract ID: tc-0123456789abcdef

Handoff source: existing-card

Executor: Codex

Runtime adapter: codex-local

Execution surface: local-workspace

Permission mode: execute-and-verify

Parallelism: none

Execution effort: normal

Workflow authority: none

任务级别：Light

Review gate:
- Light review

任务：测试 compact 格式的结构判别

背景：除第二个非空行外，本卡满足当前完整任务卡合同

项目画像：Rust workspace

记忆胶囊：无

任务存档：无

目标文件夹路径：
- .

相关路径：
- crates/

本次任务相关文件：
- Cargo.toml

目标：
- G-01: 确认 compact 结构标记被单独拒绝

验收标准：
- AC-01 -> G-01: validator 仅因第二个非空行的 compact 标记拒绝本卡

非目标：不修改任何文件

验证：
cargo test -p ags-task-contract

Verification gate:
- commands:
  - V-01 -> AC-01: cargo test -p ags-task-contract
- expected evidence:
  - EV-01 -> AC-01: compact discriminator rejects this otherwise-valid card
- stop condition:
  - 任一验证失败即停止

交付：
- 返回验证结果
