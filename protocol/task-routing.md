# Agent Task Routing

> AGS 0.3.0 的需求治理协议。自然语言语义判断属于宿主；AGS 只校验 typed proposal 和确定性准入。

## 唯一链路

```text
preflight
→ 读取 ags://capabilities/current-host（按 snapshot_hash 缓存薄目录）
→ 宿主结合完整对话做语义判断
→ HostRouteProposal
→ ags_route_request（严格只读）
→ RouteResolution
→ DirectResponse | 精确 SkillSelection | HostNativeDirectEdit | ServerHeldAction
→ 若且仅若存在 ServerHeldAction：ags_apply_action(lease_id, action_id)
```

AGS 0.3.0 没有原始文本关键词路由。`ags_route_request` 不接受 `{request: ...}`，不启动进程、不写文件，也不在失败时回退到 substring、BM25、embedding、`SkillDemand` 或第二套路由器。

## HostRouteProposal

宿主提交的 proposal 必须显式包含：

- `schema_version`
- `request_fingerprint`（非原始提示词）
- `phase`
- `solution_state`
- `execution_authority`
- `scope_hash`
- `targets`

`targets` 只有两种合法形态：

1. 独占的 `DirectResponse`；
2. 至多一个精确 `SkillTarget`，再加至多一个 `MachineCliTarget`。

`SkillTarget` 只含 `skill_id`、可选 `entrypoint` 与 `snapshot_hash`。`MachineCliTarget` 只含闭集 `CliCapabilityId` 与 `TypedCliInput`。两者都不接受自然语言。

## 阶段与授权

- `direct_response`：`solution_state=not_required`、`execution_authority=none`，直接交付并停止。
- `solution_formation`：`solution_state=open`、无执行授权；可以选精确方法技能，但不能申请机器动作。
- `execution + direct_edit`：必须是已确认方案与同会话明确修改授权；返回宿主原生动作，不编译任务卡、不经 MCP 代写仓库、不重复规划。
- `execution + task_card_handoff`：仅用于明确交接；编译仍要求 task-card request 与 confirmed handoff contract 双门槛。
- 首个非空行是 `## 任务卡`：先 validate；合法卡进入 policy/gate/LaunchPlan，非法卡停止，绝不回落到需求路由或任务卡生成。

“方案 OK”只确认设计，不单独授权修改或交接。已经确认的方案不得因为出现“架构”“实现”等词再次进入 brainstorming、writing-plans 或任务卡生成。

## 精确技能解析

宿主从 `HostCapabilitySnapshot.catalog` 读取薄 `SkillCard`，按完整语义选择精确技能。AGS Resolver 仅校验：

```text
skill_id + entrypoint + snapshot_hash → ActiveSkill
```

缺失、歧义、entrypoint 不允许或快照 stale 均 fail closed；不做关键词、相似度、候选 fallback 或自动替代。旧 `SkillDemand` / `demand_routes` 只作为 `intent_tags` 与旧序列化迁移信息，不再决定技能。

## DecisionLease 与显式 Apply

机器动作内容保存在当前 MCP 连接中。`ags_route_request` 只返回 `action_id` 与绑定证据；调用方只能提交 `lease_id`、`action_id` 和可选 outcome，不能重传 capability、input、argv 或路径。

租约绑定 host、target、proposal、scope、registry、snapshot 与 policy hash。新 preflight、新 route、连接重置、任一绑定变化，或一次成功/失败消费都会使旧租约失效。租约没有 TTL；重放、跨连接、篡改和 host/target 冲突一律 fail closed。

不与 MachineCli 共存的精确 SkillTarget 同时得到一个受控 outcome action。宿主只能通过 apply 写入 `succeeded|failed|abandoned`；若用户纠正了技能选择，先把旧 decision 记为 `abandoned`，再提交同一 `request_fingerprint` 的新 proposal。该关联只进入离线质量评估，不触发在线改路由。

## Machine CLI

`TaskPrepareExecution` 是 canonical capability；旧 `task_execute` 只作为反序列化兼容别名。`ags run` 仅执行 validate → policy → gate → LaunchPlan，并返回 `HOST_EXECUTION_REQUIRED`；真实执行、验证和收据由宿主完成。

## 任务级别与权限

需求路由不从原始文本直接推断 Light / Medium / Heavy。先确定或复用方案，再由结构化任务卡的显式字段进入 Policy/Gate：

- Light：默认 `execute-and-verify`；
- Medium：默认 `execute-and-verify`；
- Heavy：未声明时默认 `plan-only`；显式 `execute-and-verify` 可直接执行，但必须有独立 Heavy review。

技能不能改变 task level、permission mode、review gate、verification gate 或安全 stop condition。

## Handoff Compiler

新 typed `HandoffContract` 必须显式给出 `task_level`。旧 loose contract 缺失时仅兼容为 Medium 并产生 deprecation，不得靠关键词猜级别。Compiler 不重新解释原始聊天，也不选择技能。
