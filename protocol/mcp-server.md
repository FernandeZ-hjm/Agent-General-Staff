# AGS MCP: Host Initialization Adapter

> AGS 0.2.8 MCP 是宿主初始化与结构化消费适配器，不是另一套业务实现。

## Architecture

```text
Human request
  → Host conversation context
  → AGS MCP
  → Request Router (the only natural-language node)
  → RequestDecision
      ├─ DirectResponse → host replies
      ├─ SkillDemand → Skill Resolver → SkillSelection
      └─ MachineCli → fixed argv → real ags executable
```

Compiler、Policy、Gate、Runner 只消费结构化 contract。MCP 不复制它们的实现。

## AGS Initialization Gate

任何 AGS 场景的第一调用必须是：

```text
ags_preflight(agent, target?)
```

MCP 不可用时才使用 `ags session preflight --for <agent> --target <path>`。失败不得自动放行。

## Installation

```bash
cargo build -p ags-mcp
ags mcp serve --transport stdio
```

## Transport

当前支持 stdio JSON-RPC 2.0，MCP protocol version `2024-11-05`。

## MCP Capabilities

### Tools (7)

| Tool | 作用 |
|---|---|
| `ags_preflight` | 强制第一调用，建立宿主/项目预检状态 |
| `ags_protocol_status` | 读取项目协议状态 |
| `ags_agent_instructions` | 读取宿主特定指令 |
| `ags_task_validate` | 验证现有结构化任务卡 |
| `ags_policy_resolve` | 对已验证任务卡解析策略 |
| `ags_verify_local` | 执行固定 local verification scope |
| `ags_route_request` | 唯一自然语言需求路由入口 |

旧的 phase classifier 与独立 capability classifier 已删除；不存在并行调试入口。

### `ags_route_request`

输入：

```json
{
  "request": "...",
  "approved_contract": false,
  "confirmed_handoff_contract": false,
  "active_host": "codex",
  "target": "."
}
```

宿主负责把对话上下文整理进 `request` 和两个结构化证据字段。AGS 路由器无状态。

输出包含 canonical `RequestDecision`，以及按 target 类型产生的消费结果：

- `DirectResponse`：不读取技能快照，不调用 CLI；
- `Skill`：加载并校验 active host 的快照，确定性解析 skill；
- `MachineCli`：调用真实 `ags` 二进制。

### Machine CLI 固定映射

| Capability | 固定 CLI 入口 |
|---|---|
| `TaskCompile` | `ags task compile - --format json --output report --task-card-requested --confirmed-handoff-contract` |
| `TaskExecute` | `ags run - --format json` |
| `TaskValidate` | `ags task validate -` |
| `PolicyResolve` | `ags policy resolve - --format json` |
| `ProjectVerify` | `ags verify --scope local --format json --target <target>` |
| `ReceiptVerify` | `ags receipt verify - --format json` |
| `SkillTagsVerify` | 需要结构化任务卡，不能从原始文本推导 |

实现使用 `std::process::Command`、固定 argv 和 stdin pipe；禁止 shell 和任意命令字符串。

### Skill snapshot

Skill 目标要求 `<runtime_home>/capability-snapshot/capability-snapshot.json` 与当前 registry/runtime hash 一致。缺失或 stale 返回治理前置条件失败：

```text
code: skill_snapshot_stale
next: ags capability snapshot --host <host> --write
```

不自动重建、不自动替代技能。DirectResponse 与纯 MachineCli 不依赖该快照。

### Resources (5)

AGS 暴露 global kernel、agent task protocol、task routing、task-card template 与 runtime adapters 五项只读协议资源。全局内核资源 URI 是 `ags://global-kernel`。

### Prompts (4)

Prompts 用于加载治理说明，不承担自然语言路由。宿主会话启动时可加载 `ags_global_kernel`；Prompt 不能绕过 preflight、RequestDecision 或 task-card 双门槛。

## AGS vs Governed MCPs

AGS MCP 位于 `manifests/mcp-registry.yaml` 的 `suite_interfaces`；第三方 MCP 位于 `mcps`。AGS 自身不是被治理的第三方 MCP。

## JSON-RPC Protocol

Server info：

```json
{
  "name": "ags-mcp",
  "version": "0.2.8"
}
```

Wire example: `serverInfo: {"name":"ags-mcp","version":"0.2.8"}`.

## Tencent Agent Registration (WorkBuddy / CodeBuddy-Code)

这些宿主注册同一个 `ags mcp serve --transport stdio` 入口，并遵守相同 preflight 与 RequestDecision contract。标准宿主 ID 是 `tencent-agent`，客户端 ID 是 `workbuddy` 和 `codebuddy-code`。AGS 不从本命令面写宿主配置。

## Verification

```bash
cargo test -p request-router
cargo test -p skill-resolver
cargo test -p ags-mcp
cargo test --workspace
ags verify --scope full
```

## Stop Conditions

- preflight 失败；
- Skill snapshot stale；
- task-card 双门槛不完整；
- fixed CLI mapping 缺少结构化输入；
- destructive / credential / external-write / release 边界触发。

## Version History

| Version | Date | Change |
|---|---|---|
| 0.2.8 | 2026-07-16 | 唯一 Request Router、平级 RouteTarget、确定性 Skill Resolver、单 host hash 快照、MCP 固定 argv 调用真实 CLI；删除旧的双路由和机器加入层。 |
