# AGS MCP: Host Initialization Adapter

> AGS 0.3.0 MCP 是宿主初始化、只读治理解析和显式 apply 的连接适配器，不是自然语言 Agent。

## Architecture

```text
Human request
  → Host keeps full conversation context
  → read ags://capabilities/current-host
  → HostRouteProposal
  → ags_route_request (read-only resolve)
  → DirectResponse | exact Skill | host-native edit | server-held action
  → ags_apply_action(lease_id, action_id) only for a held action
```

自然语言语义选择只在宿主发生。Compiler、Policy、Gate、Runner、Skill Resolver 和 MCP server 都不重新解释原始文本。

## Initialization Gate

任何 AGS 场景的第一调用必须是 `ags_preflight(agent, target?)`；MCP 不可用时才使用 `ags session preflight --for <agent> --target <path>`。preflight 绑定当前连接的 host/target，并返回 current-host resource URI 与 `snapshot_hash`。新 preflight 会清空所有 held actions。

## MCP Capabilities

### Tools (8)

| Tool | 副作用 | 作用 |
|---|---|---|
| `ags_preflight` | 只读 | 建立连接的宿主/项目绑定 |
| `ags_protocol_status` | 只读 | 读取协议状态 |
| `ags_agent_instructions` | 只读 | 读取宿主指令 |
| `ags_task_validate` | 只读 | 验证现有任务卡 |
| `ags_policy_resolve` | 只读 | 解析已验证任务卡策略 |
| `ags_verify_local` | 只读兼容说明 | 返回固定 `ProjectVerify` 动作说明；不启动验证进程 |
| `ags_route_request` | 严格只读 | 校验 typed proposal，解析精确技能并持有动作引用 |
| `ags_apply_action` | effectful | 一次性消费当前连接内的固定动作 |

`ags_apply_action` 是 AGS MCP 内唯一 effectful 工具。所有资源均只读。
真正的 local verification 必须作为 `MachineCliTarget(ProjectVerify)` 经
`ags_route_request → DecisionLease → ags_apply_action` 执行；兼容工具
`ags_verify_local` 本身只返回这一迁移说明。

### `ags_route_request`

输入只有 typed proposal：

```json
{
  "proposal": {
    "schema_version": "0.3.0-host-route-proposal",
    "request_fingerprint": "sha256:...",
    "phase": "execution",
    "solution_state": "confirmed",
    "execution_authority": "task_card_handoff",
    "scope_hash": "sha256:...",
    "targets": [
      {
        "kind": "machine_cli",
        "capability": "task_prepare_execution",
        "input": {"kind": "task_card", "content": "## 任务卡\n..."}
      }
    ]
  }
}
```

旧 `{ "request": "..." }` 稳定返回 `legacy_raw_request_unsupported`。字段缺失返回结构化错误；绝不回退关键词分类。调用前后文件树与进程计数必须不变。

输出 `RouteResolution`，包含 `governance_status`、`proposal_hash`、preflight host/target、精确 skill selection 或阻断理由，以及可选 `DecisionLease` 证据。direct-edit 只返回 host-native action；MCP 不代写项目。

### `ags_apply_action`

```json
{
  "lease_id": "lease-...",
  "action_id": "action-...",
  "outcome": {"status": "succeeded", "quality": 90}
}
```

调用方不得重传 capability、input、argv 或 action payload。服务器只执行 route 时已固定的动作。成功或失败尝试均消费租约；重放、跨连接、hash 漂移、host/target 冲突或篡改都拒绝。

SkillTarget 在不与 MachineCli 共存时返回受控 outcome action。`outcome=abandoned` 加相同 `request_fingerprint` 的后续 decision 构成 route-correction evidence；它只供离线评估，不修改 overlay/registry 或生产路由。

### Fixed Machine CLI mappings

| Capability | 固定入口 |
|---|---|
| `TaskCompile` | `ags task compile - --format json --output report --task-card-requested --confirmed-handoff-contract` |
| `TaskPrepareExecution` | `ags run - --format json` |
| `TaskValidate` | `ags task validate -` |
| `PolicyResolve` | `ags policy resolve - --format json` |
| `ProjectVerify` | `ags verify --scope local --format json --target <preflight-target>` |
| `SkillTagsVerify` | `ags gate skill-tags - --target <preflight-target> --for <preflight-host> --format json` |
| `ReceiptVerify` | `ags receipt verify - --format json` |

每个 capability 只接受与其匹配的 `TypedCliInput`；route 在持有动作前校验，apply 在生成 argv 前再次校验。实现使用固定 argv 与 stdin，禁止 shell 和任意命令字符串。旧 `task_execute` 仅可反序列化，序列化输出永远是 `task_prepare_execution`。

### Resources (6)

新增 `ags://capabilities/current-host`：preflight-bound、只读的 `HostCapabilitySnapshot`。宿主按 session 与 `snapshot_hash` 缓存薄目录，并提交精确 `skill_id` / `entrypoint` / `snapshot_hash`。其他公开资源包括 `ags://global-kernel`、任务协议、路由、模板与 runtime adapter。

### Prompts and hosts

`ags_global_kernel` 是全局初始化 prompt。公开宿主 ID 包括 `codex`、`claude-code`、`cursor`、`tencent-agent`、`workbuddy` 与 `codebuddy-code`；它们都遵守同一个 preflight → current-host → typed proposal 入口。

## DecisionLease

Lease 只存在于当前 MCP 连接，绑定 preflight host/target、proposal/scope/registry/snapshot/policy hash。没有任意 TTL；生命周期由连接与事实绑定决定。新 route、新 preflight、连接重置、绑定变化或消费都会使旧 lease 失效。

## Runner Boundary

`TaskPrepareExecution` 只返回 LaunchPlan。Runner 不启动宿主、不验证任务执行结果、不写最终 receipt，也不声称任务完成；允许状态必须是 `HOST_EXECUTION_REQUIRED`。

## Server Info

`serverInfo` example: `{"name":"ags-mcp","version":"0.3.0"}`

## Verification

```bash
cargo test -p request-governance
cargo test -p skill-resolver
cargo test -p ags-mcp
cargo test --workspace
ags verify --scope full --format json
```

## Version History

| Version | Date | Change |
|---|---|---|
| 0.3.0 | 2026-07-19 | 宿主语义 typed proposal、只读 route、连接内 DecisionLease、显式 apply、current-host 技能目录。 |
| 0.2.8 | 2026-07-16 | 关键词 Request Router、闭集 SkillDemand、固定 argv MachineCli。 |
