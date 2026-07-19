# Agent Task Protocol

> AGS 0.3.0 权威任务协议。宿主解释语义；AGS 校验结构化提案、权限与确定性能力准入。

## 完整生命周期

### 1. Ambient Preflight（环境预检）

宿主第一步调用 MCP `ags_preflight`；MCP 不可用时才使用：

```bash
ags session preflight --for <agent> --target <path>
```

预检只建立 host/target、项目身份、协议状态、能力资源 URI 与 stop conditions。它不选择技能，也不从原始需求推断任务级别。新 preflight 会使当前连接旧租约失效。

### 2. Host Semantic Proposal（宿主语义提案）

```text
人类需求
→ 宿主保留完整对话上下文
→ 读取/缓存 ags://capabilities/current-host
→ HostRouteProposal
→ ags_route_request（只读）
→ RouteResolution
```

宿主负责语义理解与候选选择。AGS 不接收原始自然语言，不运行 substring/关键词、BM25、embedding 或另一套分类器。旧 `{request: ...}` 必须返回 `legacy_raw_request_unsupported`。

Proposal 必须提供 `schema_version`、`request_fingerprint`、`phase`、`solution_state`、`execution_authority`、`scope_hash` 和 `targets`。`DirectResponse` 独占；否则至多一个精确 `SkillTarget` 加至多一个闭集 `MachineCliTarget`。

### 3. Phase / Authority（阶段与授权）

- `DirectResponse`：有界内容加工或普通解释，直接交付，不读技能快照、不规划、不分级。
- `SolutionFormation`：只有关键设计仍开放时才进入；公开版可参考项目内公开资料形成方案。
- `DirectEdit`：方案已确认，且同会话收到明确修改授权；宿主按已确认 scope 直接执行，不编译任务卡、不重复方案形成、不通过 MCP 代写仓库。
- `TaskCardHandoff`：明确要求交接并且 handoff contract 已确认后才可编译。
- 已有 `## 任务卡`：先 validate；合法卡直接进入 policy/gate/LaunchPlan，非法卡停止，不得落回任务卡生成。

“方案 OK”仅确认设计，不独立授权 mutation 或 handoff。新问题若真正重开方案，才回到 SolutionFormation。

### 4. Exact Skill Resolution（精确技能解析）

```text
skill_id + optional entrypoint + snapshot_hash
→ validated ActiveSkillTable
→ exact SkillSelection | blocked reason
```

Skill Resolver 不读自然语言、不相似匹配、不 fallback。`SkillDemand` / `demand_routes` 仅是 0.2.x 迁移 metadata，不再拥有路由权威。候选目录与 overlay 规则见 `protocol/skill-governance.md`。

### 5. Read-only Resolve / Explicit Apply

`ags_route_request` 必须零进程启动、零文件写入。需要 AGS 机器动作时，服务器只保存当前连接内的固定 action，返回 `action_id` 与 `DecisionLease`；宿主随后显式调用：

```text
ags_apply_action(lease_id, action_id, optional outcome)
```

调用方不能重传 capability、input 或 argv。租约绑定 preflight host/target 与 proposal/scope/registry/snapshot/policy hash。新 preflight、新 route、连接重置、绑定变化或一次消费均使旧 lease 失效；没有任意 TTL。重放、跨连接与篡改 fail closed。

### 6. Task-Card Handoff Gate（任务卡交接门槛）

任务卡生成同时要求：明确 task-card/handoff 指令，以及已确认且封闭的 handoff contract。

```bash
ags task compile <contract> \
  --task-card-requested \
  --confirmed-handoff-contract
```

新 typed `HandoffContract` 必须显式声明 `task_level`。旧 loose contract 缺失时仅兼容默认 Medium，并发出 deprecation；禁止关键词推断。Compiler 不重新解释原始聊天、不选择技能。

### 7. Validate / Policy / Gate / LaunchPlan

`TaskPrepareExecution`（旧 `task_execute` 仅反序列化 alias）执行：

```text
validate → policy → gate → LaunchPlan → HOST_EXECUTION_REQUIRED
```

Runner 不启动宿主、不执行项目任务、不运行事后验证、不写最终 receipt，也不声称完成。宿主消费 LaunchPlan 后按任务卡执行、验证、review 和交付。direct-edit 不需要 route lease 或任务卡。

### 8. Receipt

新收据 writer 输出 `2.1-m6`，reader/verifier 同时接受 `2.0-m6` 与 `2.1-m6`。`governance_evidence` 只保存 decision/lease/proposal/scope/snapshot/policy hash、skill selection 与 outcome event id，不保存原始请求。

## 任务级别与权限

任务级别是风险/review 层，不是需求路由结果，也不能覆盖显式 permission：

- Light：缺省 `execute-and-verify`；边界小、验证快。
- Medium：缺省 `execute-and-verify`；跨文件或需要完整集成验证。
- Heavy：缺省 `plan-only`；若卡片显式为 `execute-and-verify`，可直接执行，但必须完成独立 Heavy review。

destructive、external-write、credential、migration、release 与 protected-path 是独立 stop conditions。

## Review Gate

- Light：按风险决定，可由主执行器完成针对性 diff 检查。
- Medium：交付前必须做完整 diff review；高影响模块应由非作者 reviewer 复核。
- Heavy：必须有独立 reviewer，对完整 diff、边界、失败语义和验证证据给出结论。作者自审、测试通过或计划确认都不能替代独立 review。
- reviewer 发现 blocking finding 时停止交付；修复后必须重跑受影响验证并重新获得 review 结论。
- 若任务卡禁止并行/外部 workflow，且当前执行面无法取得独立 reviewer，必须如实报告 review gate 未满足，不得宣称完整完成。

## GovernanceStatus

preflight、route、apply、CLI、Runner 与 receipt 共享 `GovernanceStatus`：`OK`、`NEEDS_USER_DECISION`、`BLOCKED_BY_POLICY`、`RISK_ESCALATED`、`DONE_WITH_RECEIPT`、`ADVISORY_NO_MUTATION`、`HOST_EXECUTION_REQUIRED`。它不折叠 task level、permission、review 或 stop condition。

## 角色

- 人类：提供需求、确认方案与授予明确 mutation/handoff 权限。
- 宿主 Agent：保留上下文、做语义提案、执行 host-native 工作。
- AGS Request Governance：验证 typed proposal，不解释自然语言。
- Skill Resolver：验证精确 skill/entrypoint/snapshot。
- AGS MCP：preflight、只读 resolve、连接内租约与显式 apply。
- Compiler / Policy / Gate / Runner：只消费结构化输入。

## 验证规则

声称完成前至少提供与风险匹配的目标测试、workspace 测试、local/full verify、路由/租约/技能/兼容/projection 矩阵和 diff 证据。无法证明 route 零副作用、lease 不可篡改、overlay 不覆盖官方 registry、旧收据可读或 public-safe 排除有效时，停止交付。

## 交付报告

报告必须列出：状态、改动边界、接口迁移、验证命令与结果、review 证据、git diff 摘要、保留的外部/未跟踪状态、剩余风险，以及未执行的跨工作区、版本控制和发布动作。不得把 LaunchPlan、dry-run 或“准备执行”写成已经执行完成。

权威任务卡骨架为 `protocol/task-card-template.md`。
