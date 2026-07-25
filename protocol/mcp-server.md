# AGS MCP: Workspace Service and Host Adapter

> AGS 0.3.0 MCP 是工作区治理服务与薄宿主适配器，不是自然语言 Agent。

## Architecture

```text
Human request
  → Host keeps full conversation context
  → MCP stdio thin adapter
  → connect-or-start unique workspace AGS daemon
  → per-client session + shared workspace capability state
  → read ags://capabilities/current-host
  → HostRouteProposal
  → ags_route_request (read-only resolve)
  → DirectResponse | exact Skill | host-native edit | server-held action
  → ags_apply_action(lease_id, action_id) only for a held action
```

自然语言语义选择只在宿主发生。Compiler、Policy、Gate、Runner、Skill Resolver 和 MCP server 都不重新解释原始文本。

daemon 的唯一实例键只包含工作区 canonical path，不包含 host。Codex、Claude Code、
Cursor、OMP 等宿主只是同一工作区服务的客户端；每个客户端仍拥有独立
`session_id`、preflight binding、route generation 与 DecisionLease。stdio 进程仅转发
JSON-RPC，不保存治理状态。客户端断开不会终止 daemon；无活动会话超过 idle window
后才回收。新 adapter 发现 executable hash 变化时，必须先停止旧 daemon，再启动新
版本，禁止同一工作区的新旧 daemon 并存。daemon 只监听 loopback ephemeral
endpoint；registry/token、诊断日志与 capability bundle 位于用户私有 runtime 目录，
使用私有权限、token handshake、非符号链接目录和原子替换。

## Initialization Gate

任何 AGS 场景的第一调用必须是 `ags_preflight(agent, target?)`；MCP 不可用时才使用 `ags session preflight --for <agent> --target <path>`。preflight 绑定当前 daemon client session 的 host/target，并返回 current-host resource URI、`snapshot_hash` 与 `workspace_service` 身份。preflight target 必须属于 daemon 的 canonical workspace；跨工作区 target fail closed。新 preflight 会清空该 session 的所有 held actions。若动态目录已变化而持久化快照失效，preflight 必须返回 `overall_status=warning`、`governance_status=NEEDS_USER_DECISION`、`refresh_required=true` 与结构化刷新 argv；不得继续显示 “All clear”。该 warning 不阻断 `DirectResponse`，但 `SkillTarget` / `MachineCliTarget` 在用户明确刷新并重新 preflight 前继续 fail closed。preflight 本身不自动写快照。

尚未初始化的项目不会进入普通治理态，而会建立受限
`bootstrap_required` 绑定。该绑定只允许 `ags_onboarding_plan` 与
`ags_apply_action`；route、task、policy、verify 和 phase prompts 继续 fail closed。
任一 onboarding apply 后旧绑定和全部 lease 失效，宿主必须重新 preflight。

## MCP Capabilities

### Tools (9)

| Tool | 副作用 | 作用 |
|---|---|---|
| `ags_preflight` | 只读 | 建立 session 的宿主/项目绑定 |
| `ags_protocol_status` | 只读 | 读取协议状态 |
| `ags_agent_instructions` | 只读 | 读取宿主指令 |
| `ags_onboarding_plan` | 严格只读 | 评估 public profile，并为可逐项确认的动作建立 session 内引用 |
| `ags_task_validate` | 只读 | 验证现有任务卡 |
| `ags_policy_resolve` | 只读 | 解析已验证任务卡策略 |
| `ags_verify_local` | 只读兼容说明 | 返回固定 `ProjectVerify` 动作说明；不启动验证进程 |
| `ags_route_request` | 严格只读 | 校验 typed proposal，解析精确技能并持有动作引用 |
| `ags_apply_action` | effectful | 一次性消费当前 daemon client session 内的固定动作 |

`ags_apply_action` 是 AGS MCP 内唯一 effectful 工具。所有资源均只读。
真正的 local verification 必须作为 `MachineCliTarget(ProjectVerify)` 经
`ags_route_request → DecisionLease → ags_apply_action` 执行；兼容工具
`ags_verify_local` 本身只返回这一迁移说明。

### `ags_onboarding_plan`

该工具只消费 preflight 绑定，不接受路径、命令或 profile 重传。它固定读取
public onboarding profile和统一第三方能力清单，返回 `absent`、
`installed-not-visible`、`visible-not-ready`、`active-ready`、
`blocked-untrusted-source`、`blocked-missing-integrity` 或
`unsupported-host`。

可执行项目只返回 `item_id + action_id`，不返回可篡改 argv。用户选择一个项目后，
`ags_apply_action(lease_id, action_id)` 才运行既定的项目 init、官方宿主 registrar、
受审计 Skill adoption 或固定 npm MCP 注册。一次 lease 只能选择一个项目。

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

调用方不得重传 capability、input、argv 或 action payload。服务器只执行 route 时已固定的动作。成功或失败尝试均消费租约；重放、跨 session、hash 漂移、host/target 冲突或篡改都拒绝。

SkillTarget 在不与 MachineCli 共存时返回受控 outcome action。`outcome=abandoned` 加相同 `request_fingerprint` 的后续 decision 构成 route-correction evidence；它只供离线评估，不修改 overlay/registry 或生产路由。

### Fixed Machine CLI mappings

| Capability | 固定入口 |
|---|---|
| `TaskCompile` | `confirmed_handoff_contract.handoff_source=explicit_handoff` → `ags task compile - --format json --output report --task-card-requested --confirmed-handoff-contract`; `host_plan_mode` → 同一固定入口改用 `--host-plan-mode-final` |
| `TaskPrepareExecution` | `ags run - --format json` |
| `TaskValidate` | `ags task validate -` |
| `PolicyResolve` | `ags policy resolve - --format json` |
| `ProjectVerify` | `ags verify --scope local --format json --target <preflight-target>` |
| `SkillTagsVerify` | `ags gate skill-tags - --target <preflight-target> --for <preflight-host> --format json` |
| `ReceiptVerify` | `ags receipt verify - --format json` |

每个 capability 只接受与其匹配的 `TypedCliInput`；route 在持有动作前校验，apply 在生成 argv 前再次校验。实现使用固定 argv 与 stdin，禁止 shell 和任意命令字符串。旧 `task_execute` 仅可反序列化，序列化输出永远是 `task_prepare_execution`。

### Resources (6)

新增 `ags://capabilities/current-host`：preflight-bound、只读的 `HostCapabilitySnapshot`。经显式刷新写出的 validated snapshot 由工作区 daemon 单点读取、重新校验、缓存并原子吸收到 workspace capability bundle；后续 route/apply 不再绕回可能被其他工作区覆盖的全机活动文件。宿主按 session 与 `snapshot_hash` 缓存薄目录，并提交精确 `skill_id` / `entrypoint` / `snapshot_hash`。第三方能力只有同时满足 `route_state = routable` 与 `availability.state = ready` 才能进入自然语言候选；hook 永远只走宿主事件面。host-native MCP 还必须以宿主当前连接的实时工具可见性为准，不能把仅注册或健康未知当作 ready。快照失效时，preflight 的 `capability_catalog.refresh.argv` 给出显式机器本地刷新参数；宿主须先取得用户确认，执行后重新 preflight 并读取新的 `snapshot_hash`。其他公开资源包括 `ags://global-kernel`、任务协议、路由、模板与 runtime adapter。

### Prompts and hosts

`ags_global_kernel` 是全局初始化 prompt。公开宿主 ID 包括 `codex`、`claude-code`、`cursor`、`tencent-agent`、`workbuddy` 与 `codebuddy-code`；它们都遵守同一个 preflight → current-host → typed proposal 入口。

## DecisionLease

Lease 只存在于当前 daemon client session，绑定 `session_id`、preflight host/target、proposal/scope/registry/snapshot/policy hash。它不能跨 Codex/OMP/Claude Code/Cursor session 使用。没有任意 TTL；生命周期由 session 与事实绑定决定。新 route、新 preflight、session 重置、绑定变化或消费都会使旧 lease 失效。

Onboarding lease 绑定 public profile 的完整 `plan_hash`、item、host 与 target；
不借用尚未存在的 capability snapshot。apply 后强制重新 preflight。

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
| 0.3.0 | 2026-07-19 | 宿主语义 typed proposal、只读 route、session 内 DecisionLease、显式 apply、workspace daemon 与 current-host 技能目录。 |
| 0.2.8 | 2026-07-16 | 关键词 Request Router、闭集 SkillDemand、固定 argv MachineCli。 |
