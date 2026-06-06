## 任务卡

路径：
- /Volumes/AI Project/agent-governance-suite-private-rust

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: cli

Permission mode: execute-and-verify

Parallelism: none

Execution effort: normal

Workflow authority: none

任务级别：Medium

读取：
- 本任务卡

任务：运行 cargo test 验证所有测试通过

目标：验证 task-card-validator 能正确识别合法的 compact 任务卡

非目标：不修改任何文件

关键路径：
- crates/

验证：
cargo test --workspace

停止条件：
cargo test 失败时停止并报告

交付：
返回测试通过结果
