## 任务卡

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

任务：测试完整任务卡格式校验功能

背景：验证 full task card 的所有必填字段能被正确识别

项目画像：Rust workspace with task-card-validator crate

记忆胶囊：暂无相关记忆

任务存档：参考此前 compact card 校验通过记录

目标文件夹路径：
- /Volumes/AI Project/agent-governance-suite-private

相关路径：
- crates/

本次任务相关文件：
- Cargo.toml

目标：
- G-01: 确认 full task card 校验器正确接受合法输入

验收标准：
- AC-01 -> G-01: validator、policy、gate 与 runner 均接受同一合法输入

非目标：不涉及生产环境变更

验证：
cargo test --workspace

Verification gate:
- commands:
  - V-01 -> AC-01: cargo test --workspace
- expected evidence:
  - EV-01 -> AC-01: all tests pass
- stop condition:
  - 任一验证命令失败即停止并报告

交付：
- 按协议输出测试通过结果，并用 Contract ID 与 task-card-hash 回绑交付报告
