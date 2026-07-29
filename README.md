# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)

[中文](README.md) | [English](README_EN.md)

Agent Governance Suite（AGS）是一个**多 Agent 开发治理控制面**。它负责准入、
授权、策略、能力快照、验证、回执和记忆闭环，但不负责调度 Agent 团队，也不是任务
队列、并行执行器或多 Agent 协商运行时。

本仓是 AGS 的公开发行版，采用 **GPL-3.0-only**。当前产品版本和 latest release
是 **v0.3.8**。

## v0.3.8 核心链路

```text
用户请求
  -> 宿主理解完整对话
  -> typed HostRouteProposal
  -> AGS 校验封闭字段
  -> 只读 ags_route_request
  -> 宿主原生动作或显式 ags_apply_action
  -> evidence / delivery closure
```

- 自然语言只由 Codex、Claude Code、Cursor、OMP 等宿主解释。
- `ags_route_request` 严格只读，不接收 raw request，也不做关键词或相似度回退。
- `ags_apply_action` 是唯一 effectful MCP 工具，只消费服务端保存的一次性固定动作。
- DirectResponse 与受治理目标互斥；否则最多一个精确 SkillTarget 和一个闭集
  MachineCliTarget。
- Skill Resolver、Compiler、Policy、Gate、Runner 只消费结构化契约。
- 任务卡生成必须同时满足明确交接指令和 confirmed handoff contract。

## 一工作区一个 AGS

每个 canonical workspace 只有一个长期 AGS daemon：

```text
canonical workspace
  -> AGS daemon
       -> Codex client session
       -> Claude Code client session
       -> Cursor client session
       -> OMP client session
```

stdio 进程只是 `connect-or-start` 薄转发。工作区 daemon 为每个宿主加载一次静态
snapshot，而 `session_id`、preflight binding 和 DecisionLease 始终按客户端会话隔离。断开某个
客户端不会停止 daemon；空闲回收和二进制 stop-before-restart 升级由服务内部处理。

这项架构**不增加任何用户命令**。宿主仍然启动：

```bash
ags mcp serve --transport stdio
```

## 十二个主要 module

v0.3.8 的 runtime workspace 只暴露十二个权威 Cargo package：

| Module | 职责 |
|---|---|
| `ags-platform` | 跨平台路径、文件、进程、哈希和原子写入 |
| `ags-workspace-facts` | canonical workspace、项目发现、协议审计和 preflight facts |
| `ags-host-integration` | Codex、Claude Code、Cursor、OMP 宿主集成事实 |
| `ags-capability-governance` | capability catalog、skill-body governance、精确解析和 snapshot |
| `ags-task-contract` | 任务卡编译/校验、handoff contract、非执行型 launch preparation |
| `ags-governance-decision` | typed proposal、policy、route 和 decision contracts |
| `ags-session` | workspace daemon、client session、binding 和一次性 action store |
| `ags-evidence` | receipt、delivery closure 和证据完整性 |
| `ags-verification` | bootstrap readiness、doctor、projection、local/promotion/release 验证 |
| `ags-lifecycle` | setup、init、onboarding、update |
| `ags-cli` | 当前人类 CLI 和 Machine CLI adapter |
| `ags-mcp` | MCP wire、session connection 和错误映射薄 adapter |

原 `bootstrap-dry-run`、`capability-registry`、
`delivery-report-validator`、`execution-policy`、`runner`、
`skill-governance`、`suite-doctor`、`task-card-validator`、
`workflow-sync-check` 的实现已收口到对应权威 module。0.3.8 只保留当前实际调用的
命令、wire/schema 和必要 re-export，不再保留旧命令或第二套 package authority。详见
[WORKSPACE.md](WORKSPACE.md) 和 [docs/architecture.md](docs/architecture.md)。

## 宿主支持矩阵

| 宿主 | MCP / daemon | Skill/命令入口 | 原生记忆闭环 | 当前验证 |
|---|---|---|---|---|
| Codex | 支持 | 全局/项目 skills | SessionStart / SessionEnd adapter | 原生 MCP 登记探针 + MCP 进程 E2E |
| Claude Code | 支持 | `/ags` 与 skills | SessionStart / Stop adapter | 原生 MCP 连接探针 + MCP 进程 E2E |
| OMP | 支持；可复用 Codex 配置 | native/shared skills | OMP lifecycle extension | 原生 RPC 可发现性探针 + MCP 进程 E2E |
| Cursor | 支持 | host/project skill projection | 原生 sessionStart / sessionEnd / stop hooks | `cursor-agent mcp list` 原生只读探针 + lifecycle/MCP 进程 E2E |
| CodeBuddy-Code / WorkBuddy | MCP 接入 | setup 生成配置片段 | 尚无完整原生记忆闭环声明 | 初始化与静态/可见性验证 |

这里的 E2E 会启动真实 `ags` stdio adapter 和 workspace daemon，覆盖同工作区共享、
跨项目隔离、重连、foreign lease 拒绝、snapshot rebind、idle recycle 和升级；它不是
对各宿主 GUI 的自动化操作。
Cursor 的记忆闭环与外部 MCP 注册是两个独立事实：AGS 可以安装并验证原生 lifecycle
hooks，也可通过 `cursor-agent mcp list` 只读验证注册；写入 Cursor MCP 配置仍由操作者控制。

## 真实限制

- AGS 只能证明 typed proposal 与治理状态一致，不能证明宿主正确理解了用户原意。
- Runner 只返回验证后的 LaunchPlan / host handoff，不 dispatch Agent，不声称执行或
  验证已经发生。
- 没有任务队列、Agent scheduler、资源配额或多 Agent 协商。
- MCP/CLI 等外部注册通常是 advice-only；AGS 不替用户执行第三方安装命令。
- Codex、Claude Code、Cursor 与 OMP 共享同一 Rust lifecycle contract；宿主侧只做事件映射。
- 公开版不携带私人 skill bodies、真实 memory/receipt/archive 或机器私有 runtime。

## 安装

普通 MCP 用户无需 Rust 或 Cargo：

```bash
npx -y @agent-governance-suite/mcp
```

npm launcher 会按 OS/架构下载同版本预编译 `ags`，校验 `SHA256SUMS`，缓存验证后的
二进制，并无 shell 地启动 `ags mcp serve --transport stdio`。

从源码安装：

```bash
cargo install --path crates/ags-cli --locked --force
ags setup --yes --force
```

## 首次接入

```bash
ags onboarding plan --host codex
ags onboarding apply --item project-init --plan-hash <HASH_FROM_PLAN> --host codex --yes
ags onboarding verify --host codex
```

`apply` 一次只接受一个 plan item。第三方 capability manifest 随发布包固定；普通
setup、preflight、resource read、route 和 apply 都不联网刷新。

## 稳定命令面

v0.3.8 只承诺下列当前命令面。已删除的旧命令、alias 和 plan-only 假动作不再作为兼容合同。

```bash
ags setup --help
ags onboarding --help
ags init --help
ags doctor --help
ags agents --help
ags capability --help
ags skill --help
ags update --help
ags mcp --help
ags memory --help
ags host lifecycle --help
ags task close --help
```

写入型动作仍需显式 `--apply` 或 `--yes`，并受既有确认、policy 和 lease 门禁约束。

## 宿主内部契约与执行边界

`policy`、`project`、`session` 与 `run` 主要是宿主/MCP Machine CLI 合同。
`task validate/close`、`memory`、`mcp` 与 `verify` 同时是明确的运维接口。

没有明确交接指令或 confirmed handoff contract 时，compiler 只能返回诊断，不能输出
可执行卡。`ags run` 仍是非执行型 preparation surface：它验证 task card、解析 policy、
执行 gate，并返回 `host_execution_required` 的结构化计划。

## 验证

```bash
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
ags verify --scope release
git diff --check
```

公开版完成判定不能只看精确 release manifest。还必须验证十二 module 源码
结构、双语文档、真实 MCP E2E、performance benchmark 合同、旧 authority 缺失、
发布资产和 exact public commit 的远端 CI。

## 许可证与发布

- 许可证：**GPL-3.0-only**
- latest：**v0.3.8**
- 当前合同：v0.3.8 human/Machine CLI
- 历史：v0.3.1 release notes 保留，不作为 current version

发布顺序不可倒置：

1. public-safe 源码进入 GitHub `main`，等待 exact commit CI 全绿；
2. Cargo、npm、manifest、文档和 release notes 统一为 `0.3.8`；
3. 维护者显式推送 annotated `v0.3.8` tag；
4. tag workflow 构建五个平台资产、`SHA256SUMS` 和 provenance；
5. Release 资产齐全后，手动 dispatch npm OIDC trusted-publisher workflow，
   发布 `@agent-governance-suite/mcp@0.3.8` 为 latest。

日常 CI、同步 guard 和 npm workflow 都不会替维护者创建 tag。

## 公开边界

[WORKSPACE.md](WORKSPACE.md) 说明十二个 module 的 ownership 和 support package
迁移状态。公开版是完整公共 Rust 版，但不包含维护者本机的 skill bodies、真实
记忆/回执/归档、凭据、本机配置或 `workspace-services/` 状态。
