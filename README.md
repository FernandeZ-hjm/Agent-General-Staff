# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)]()

[中文](README.md) | [English](README.en.md)

AGS 是一个面向多 Agent 开发的治理控制面。它负责准入、授权、策略、验证、回执、
能力快照和记忆闭环；它不负责组建 Agent 团队，也不提供任务队列、调度器、并行执行器
或 Agent 间协商。

当前 latest 产品版本是 **v0.3.1**，许可证为
**GPL-3.0-only**。

## 它解决什么

Codex、Claude Code、Cursor 和 OMP 都可以在同一个代码库工作，但宿主能力强并不等于
工程边界清楚。AGS 把容易漂移的自然语言执行过程压缩成可验证的治理链路：

```text
项目预检
  → 宿主提交类型化提案
  → AGS 校验权限、策略与精确能力
  → 宿主执行
  → 验证、回执与交付闭环
```

AGS 的边界是：

- 宿主理解用户语义；AGS 不解析原始自然语言来猜技能或权限。
- AGS 校验封闭的 `HostRouteProposal`，只接受精确能力标识和固定机器动作。
- `ags_route_request` 只读；唯一 effectful MCP 工具是
  `ags_apply_action`，并且只消费一次性、session-bound lease。
- Runner 只准备结构化 LaunchPlan，不伪装成已经执行或验证。
- 缺少任务卡、能力、认证、快照或验证证据时 fail closed。

## 快速开始

### 从源码安装

```bash
git clone https://github.com/FernandeZ-hjm/Agent-General-Staff.git
cd Agent-General-Staff
bash scripts/install.sh
```

脚本会构建 `ags` 并执行公开安全的本机 setup。也可以手动构建：

```bash
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
ags --version
```

### 作为 MCP launcher 使用

正式 Release 的预编译资产可通过 npm launcher 启动：

```bash
npx -y @agent-governance-suite/mcp
```

launcher 根据 OS/架构下载同版本二进制，校验 `SHA256SUMS` 后以无 shell 子进程启动
MCP stdio adapter。npm 包通过 GitHub OIDC trusted publishing 发布，不保存长期 npm
token。

### 推荐流程

```bash
ags setup --yes --force
ags agents govern --agent codex --apply
ags agents govern --agent claude-code --apply
ags agents govern --agent omp --apply
ags init --target .
ags doctor --target .
ags verify --scope local
```

所有会写入的生命周期命令仍然遵守 dry-run 或显式 `--apply` / `--yes` 约束。

## 人类命令面

v0.3.1 保持 v0.3.0 的完整 Clap 命令树、参数、别名、默认值、stdout/stderr、
退出码和 JSON schema。唯一预期变化是：

```text
ags --version
0.3.0 → 0.3.1
```

顶层人类命令保持不变：

| 命令 | 作用 |
|---|---|
| `ags setup` | 安装或升级本机治理内核 |
| `ags onboarding` | 评估并逐项确认公开 onboarding |
| `ags init` | 将项目接入 AGS 协议与入口 |
| `ags doctor` | 诊断 runtime、宿主、项目与能力链路 |
| `ags agents` | 扫描、纳管并验证 Agent 宿主 |
| `ags capability` | 能力 inventory、snapshot 与宿主可见性 |
| `ags skill` | 技能本体、入口、更新与回滚治理 |
| `ags update` | 更新内核、runtime、宿主、技能与项目投影 |

内部 MCP/Machine CLI 没有被提升为新的人类命令。现有脚本和项目入口无需迁移。

## v0.3.1 Workspace Service

MCP stdio 进程现在是薄适配器：

```text
MCP stdio adapter
        ↓ connect-or-start
canonical workspace path → 唯一 AGS workspace daemon
        ├── Codex session
        ├── Claude Code session
        ├── Cursor session
        └── OMP session
```

- daemon 实例键只有 workspace canonical path，不包含 host。
- 同一工作区共享一份原子发布的能力 bundle，避免重复扫描。
- 每个对话保留独立 `session_id`、preflight binding 和 DecisionLease。
- 技能刷新采用候选 bundle 校验、原子替换、发布新 hash；旧绑定得到明确失效结果，
  重新 preflight 后立即接受新 hash。
- lease 跨宿主、跨 session、跨 workspace 或重放都会拒绝。
- 客户端断开不会立刻终止 daemon；无会话且超过 idle TTL 后回收。
- 二进制升级严格 stop-before-restart，不允许不同版本共同服务同一工作区。

这部分是内部运行架构变化，不要求用户学习新命令。

## 12 个主要边界

| crate | 职责 |
|---|---|
| `ags-platform` | 路径、文件系统、进程、哈希与原子写入 |
| `ags-workspace-facts` | canonical workspace、发现和配置事实 |
| `ags-host-integration` | Codex、Claude Code、Cursor、OMP 宿主适配 |
| `ags-capability-governance` | 能力清单、精确解析、技能生命周期和快照 |
| `ags-task-contract` | 任务卡、编译、验证与交接契约 |
| `ags-governance-decision` | 类型化提案、策略、授权和 route decision |
| `ags-session` | workspace service、session、preflight 和 lease |
| `ags-evidence` | receipt、delivery report 和证据模型 |
| `ags-verification` | doctor、local/release verify 和同步检查 |
| `ags-lifecycle` | setup、init、onboarding、update 和 rollback |
| `ags-cli` | 保持兼容的人类 CLI 薄适配层 |
| `ags-mcp` | MCP 协议转换、连接和错误映射 |

治理规则不放在 CLI 或 MCP 适配层中，也不保留新旧两套路由实现。

## 真实支持范围

| 能力 | Codex | Claude Code | Cursor | OMP |
|---|---:|---:|---:|---:|
| 同一 workspace daemon | 是 | 是 | 是 | 是 |
| 独立 session/preflight/lease | 是 | 是 | 是 | 是 |
| 能力快照刷新与重连 | 是 | 是 | 是 | 是 |
| 技能入口探测 | 是 | 是 | 是 | 是 |
| 宿主原生记忆生命周期 | 是 | 是 | 受宿主能力限制 | 是 |

四宿主 E2E 覆盖同工作区单 daemon、跨项目隔离、snapshot refresh、stdio 重连、
lease 重放/越界拒绝、idle recycle、升级重启和损坏 bundle 错误。

源码 CI 覆盖 Linux、macOS、Windows。tag 发布构建 Apple Silicon、Intel macOS、
x86_64 Linux、ARM64 Linux 和 x86_64 Windows 资产。

## 安全与供应链

- 固定 argv，不使用 shell 字符串拼接执行治理动作。
- 任务卡、策略、路径、符号链接、内容 hash 和一次性 lease 均有 fail-closed 校验。
- onboarding 清单固定到不可变 Git commit，并校验预期 SHA-256；网络失败时才使用
  当前版本内置的已审阅清单。
- Release 包含 `SHA256SUMS` 和 provenance；npm 使用 trusted publisher。
- `cargo-deny`、Clippy strict mode、release boundary 和 public-private guard 都是发布门禁。
- 公开 tracked payload 不包含本机路径、runtime 状态、第三方技能本体或私有实现。

## 明确限制

AGS 目前不是：

- AutoGen、LangGraph 一类执行编排框架；
- Agent 任务队列、资源配额或并行调度器；
- 自动理解用户意图的自然语言安全证明器；
- 真实 Agent 执行结果的替代品。

类型化提案可以证明提案内部是否合法，不能证明宿主一定正确理解了用户意图。真实交付
仍然需要宿主执行、项目验证和证据闭环。

## 验证

日常使用：

```bash
ags doctor --target .
ags verify --scope local
```

源码贡献：

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --release --locked
bash scripts/verify.sh
git diff --check
```

发布门禁还会检查：

- v0.3.0 → v0.3.1 人类命令面和 Machine CLI fixture；
- workspace daemon/session/lease/snapshot E2E；
- 产品版本、协议/schema 版本和历史版本分组；
- public release boundary、npm launcher 和私有标记扫描。

### 0.3.1 发布顺序

1. public-safe `main` 的精确提交通过全部 GitHub Actions。
2. Cargo、npm、suite manifest、MCP `serverInfo`、README 和 Release Notes 都是
   `0.3.1`，wire/schema 兼容标识保持原值。
3. 在该提交创建 annotated `v0.3.1` tag。
4. tag workflow 产出五平台资产、`SHA256SUMS` 和 provenance，并创建 GitHub Release。
5. 手动触发 OIDC npm workflow，确认 registry `0.3.1` 且
   `latest` 指向 `0.3.1`。

推送 `main` 不等于完成 tag、Release 或 npm 发布。

## 文档

- [架构](docs/architecture.md)
- [MCP 协议](protocol/mcp-server.md)
- [任务协议](protocol/agent-task-protocol.md)
- [技能治理](protocol/skill-governance.md)
- [Release Notes](RELEASE_NOTES.md)
- [安全策略](SECURITY.md)
- [商业与 GPL 说明](COMMERCIAL.md)

## 许可证

AGS 使用 **GNU General Public License v3.0 only (GPL-3.0-only)**。

可以使用、研究、修改和分发；分发 AGS 或其衍生作品时，必须按 GPL-3.0-only 向接收方
提供相应完整源代码。纯内部使用本身不触发分发义务。第三方材料的许可证与署名见
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
