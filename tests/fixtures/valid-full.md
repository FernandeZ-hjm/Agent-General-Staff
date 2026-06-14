## 任务卡

读取并遵守：
- AGENTS.md

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

任务存档：参考 full card 校验通过记录

相关路径：
- crates/

本次任务相关文件：
- Cargo.toml

目标：确认 full task card 校验器正确接受合法输入

非目标：不涉及生产环境变更

验证：
cargo test --workspace

Verification gate:
- commands: cargo test --workspace
- expected evidence: all tests pass

交付：
按协议输出测试通过结果
