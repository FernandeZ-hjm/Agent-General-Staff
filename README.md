# Agent Governance Suite — Private Development Suite

Agent Governance Suite 私有版主库（A）。本仓把 Codex / Cursor /
Claude Code 三方协作协议、任务卡治理、Rust 原生 CLI 门禁和分发边界
打包成可迁移的私有工程套件。

AGS 定位为开发工作中的**常驻工程中枢**：开发请求到达时自动接入治理流程
（ambient preflight → solution formation → user confirmation → user task-card
instruction → execution contract → task routing → gate / execution / receipt），
不需要用户单独唤出 CLI 工具箱。\"方案 OK\" 只结束方案阶段，用户任务卡指令是进入
路由的硬门槛；`ags task compile --task-card-requested` 强制执行此规则。

公开版必须从明确的 public/core-only 边界单独维护，不能直接发布本仓内容。

## 工作区身份

| Code | Role | Path |
|---|---|---|
| A | Development private suite | `/Volumes/AI Project/agent-governance-suite-private` |
| A1 | Private bare repo | `/Users/hujiaming/git-remotes/agent-governance-suite-private.git` |
| S | Stable private suite | `/Volumes/AI Project/agent-governance-suite-stable` |
| B | Public worktree | `/Volumes/AI Project/ai-dev-env-bootstrap` |
| B1 | Public bare repo | `/Users/hujiaming/git-remotes/ai-dev-env-bootstrap.git` |

## 当前状态

本仓是协议权威源和日常开发入口。Rust `ags` 工具链已经提供：

- `ags task validate`（旧别名：`task-card-validator`）— canonical 任务卡格式与语义门禁
- `ags policy resolve`（旧别名：`resolve-policy`）— 执行策略解析
- `ags sync check`（旧别名：`workflow-sync-check`）— private / stable / public-core 协议漂移与安全断言检查
- `ags doctor`（旧别名：`suite-doctor`）— 套件健康诊断
- `ags bootstrap --dry-run`（旧别名：`bootstrap-dry-run`）— 引导流程干运行检查
- `ags project detect` / `ags protocol status` / `ags agent instructions` — M2 Agent 感知能力（只读）
- `ags session preflight --for codex|claude-code|cursor` — 聚合 Agent 唤醒检查（kernel activation 入口，不依赖 skill governance）
- `ags verify --scope local|full|release` — 结构化验证入口，提供稳定 CheckItem 模型和机器可消费 JSON 报告

## 工作区结构

```
Cargo.toml                  # workspace root
Cargo.lock                  # lockfile (tracked)
AGENTS.md                   # agent entry point
CLAUDE.md                   # workspace protocol
WORKSPACE.md                # repository role map
AGENT_SUITE_PROTOCOL.md     # suite protocol overview
protocol/                   # canonical protocol files in A
governance/                 # skill governance audit logs and sync docs
manifests/                  # suite manifest and capability metadata
scripts/                    # verification and validation scripts
crates/
  ags-cli/                  # unified CLI entry point (binary `ags`)
  task-card-validator/      # task-card validation library
  execution-policy/         # execution policy resolver
  workflow-sync-check/      # protocol drift checker + safety assertions
  suite-doctor/             # suite health diagnostics
  bootstrap-dry-run/        # bootstrap dry-run simulation
tests/
  fixtures/                 # validator test fixtures
```

## 边界

- A 是协议权威源和日常开发入口。
- A 只能通过 `scripts/push-a1.sh` 推送到 A1。
- S 从 A1 fast-forward 拉取，是当前稳定运行基线。
- 未经明确任务授权，A 不直接修改 S。
- Rust task-card-validator 是唯一 canonical 任务卡格式门禁。
- 不安装 hook、runner adapter 或生产 wiring，除非用户明确批准。
- Private -> public promotion 必须经过发布清单和边界审查。

## Stable -> Public 发布边界

public/core-only 是公开可分发用户包，不是私有治理工作台。S 推 B/B1 时只允许
发布公开协议、任务卡模板、项目集成模板、公开安装/校验脚本、公开文档、
license/changelog、公开 manifest/capability 元数据，以及公开发布清单显式包含的
public-safe rules/skills。

Rust `ags` 工具链是私有治理工具，不是公开 payload。公开版不得携带
`Cargo.toml`、`Cargo.lock`、`crates/`、`target/`、release/debug `ags`
二进制，或私有诊断/同步工具实现。

## 安装

```bash
# 安装 ags 到 ~/.cargo/bin/ags（要求 ~/.cargo/bin 在 PATH 中）
cargo install --path crates/ags-cli

# 验证
command -v ags
ags --help
```

`cargo install --path crates/ags-cli` 是标准 Rust 安装方式，完全可审计：
不引入外部依赖，不静默修改 shell 配置。`~/.cargo/bin` 默认已在 Rust 用户的
PATH 中。

## 运行

```bash
# 构建
cargo build
cargo build --release

# 测试
cargo test
RUSTFLAGS="-D warnings" cargo test

# CLI（推荐：直接用 ags）
ags --help
ags task --help
ags sync --help
ags doctor --help
ags bootstrap --help
ags policy --help
ags session preflight --for codex
ags session preflight --for claude-code --format json

# 开发 fallback（cargo run，不要求 PATH 安装）
cargo run -p ags-cli -- --help
cargo run -p ags-cli -- task --help
cargo run -p ags-cli -- session preflight --for codex

# 校验任务卡
ags task validate path/to/task-card.md
ags task validate - < task-card.md

# 便捷包装 (委托 Rust validator)
bash scripts/validate.sh path/to/task-card.md
```

## 验证

```bash
# 结构化验证（推荐）—— Rust CheckItem 模型，text/json 双格式
ags verify --scope local             # 本地门禁（fmt、test、build、fixtures、YAML、preflight）
ags verify --scope local --format json
ags verify --scope full              # 完整门禁（local + stable/public drift）
ags verify --scope release           # 发布边界检查

# 兼容 wrapper（委托 ags verify + 剩余 shell-only smoke tests）
bash scripts/verify.sh

# 或手动基础检查:
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
```

## 推送

```bash
bash scripts/push-a1.sh
```

该脚本只允许从 A 推 A1，随后 fast-forward S，并重新运行 full verification。
