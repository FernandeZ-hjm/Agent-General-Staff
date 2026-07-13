## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- protocol/agent-task-protocol.md
- protocol/task-routing.md
- protocol/runtime-adapters.md

Executor: Claude Code

Runtime adapter: claude-code

Execution surface: cli

Permission mode: execute-and-verify

Parallelism: none

任务级别：Light

Review gate:
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

任务：
在 demo-project 的 main.rs 中新增一个 greet 函数。

背景：
demo-project 是一个合成示例项目，位于 examples/demo-project/。当前 main.rs
只打印参数或默认消息，需要新增 greet 函数。

项目画像：
- 无

记忆胶囊：
- 无

任务存档：
- 无

相关路径：
- examples/demo-project/Cargo.toml
- examples/demo-project/src/main.rs

本次任务相关文件：
- examples/demo-project/src/main.rs
- examples/demo-project/tests/demo_test.rs

目标：
1. 在 src/main.rs 中新增 fn greet(name: &str) -> String
2. 在 main 函数中调用 greet
3. 运行 cargo test 确认未破坏已有测试

非目标：
- 不修改 Cargo.toml
- 不新增依赖
- 不修改已有测试文件 tests/demo_test.rs

验证：
Verification gate:
- commands:
  - cargo test --manifest-path examples/demo-project/Cargo.toml
- expected evidence:
  - cargo test 输出显示所有测试通过
  - git diff --stat 确认只修改了 main.rs
- stop condition:
  - 测试失败且无法在当前范围内修复
  - 需要修改 Cargo.toml 或新增依赖时停止并报告

交付：
按 protocol/agent-task-protocol.md 输出交付报告。
完成前通过 `superpowers` 执行 `verification-before-completion` playbook。

[skill: superpowers]
