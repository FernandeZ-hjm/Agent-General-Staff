## 任务卡

路径：
- .

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: cli

Permission mode: autonomous-low-risk

Parallelism: none

Execution effort: normal

Workflow authority: none

任务级别：Light

读取：
- .

任务：自动运行测试

目标：验证功能正确性

非目标：不修改文件

关键路径：
- .

验证：
cargo test

停止条件：
test 失败时停止

交付：
返回测试结果
