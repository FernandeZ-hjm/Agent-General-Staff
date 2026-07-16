# Agent Task Routing

> AGS 0.2.8 路由细则。权威生命周期见 `protocol/agent-task-protocol.md`。

## Lifecycle Order

```text
preflight
→ 宿主提供完整对话上下文
→ ags_route_request
→ RequestDecision
→ DirectResponse | SkillDemand | MachineCli
→ 对应消费端
```

自然语言只在 Request Router 解释一次。Skill Resolver、Compiler、Policy、Gate、Runner 和 CLI capability 均不得再次解析自然语言。

## RequestDecision

`DirectResponse` 是独占终止目标。一个决定也可以同时包含一个 `Skill` 与一个 `MachineCli`；两者是自然语言需求的平级目标，不存在“需求路由优先于技能路由”的第二套路由。

`InsufficientContext` 描述路由判断状态，不是 AGS 持有的会话状态。宿主保留上下文并在下一轮重新提交完整输入。

## DirectResponse

下列任务直接回复并停止：

- 按已确认结构压缩或改写内容；
- 翻译、摘要、重排、格式转换；
- 统一已批准 JSON 字段；
- 不需要机器状态的普通解释。

它们不触发 brainstorming、writing-plans、任务分级、task-card compiler 或技能快照读取。

## SkillDemand

`SkillDemand` 是闭集 enum，按领域分组：Engineering、Knowledge、Lark、Content、Personal。Router 只返回 demand，不返回技能名。

系统架构 demand 必须出现明确的大型设计信号。普通“实现”“创作”“压缩”“格式转换”“按已批准方案执行”不得仅凭主题词命中 brainstorming。

`manifests/skills-registry.yaml` 的 `demand_routes` 是 demand-to-skill 唯一映射表。每个 demand 恰好一条映射；Resolver 不做相似度搜索和 fallback。

Superpowers 是宿主可发现的父技能。内部 brainstorming、TDD、executing-plans、verification 等由明确 entrypoint 表达，但任务卡 tag 仍使用父技能 `[skill: superpowers]`。

## ActiveSkillTable

路由前由 MCP 读取机器本地快照。快照是持久化缓存，同时带 registry/runtime/hash 完整性校验；不一致即 `skill_snapshot_stale`，不得边路由边静默重建。

成功的以下写入动作刷新 Codex 快照：

- `ags setup --yes`
- `ags skill propose ... --apply`
- `ags skill sync --apply`
- `ags capability install ... --apply`
- `ags capability sync --apply`
- `ags update apply --apply`
- `ags update repair-local --apply`

人类也可显式刷新：

```bash
ags capability snapshot --host codex --write
```

## Machine CLI

`CliCapabilityId` 粒度是业务动作，不是子命令：例如 `TaskExecute` 表示执行任务卡完整链路，而不是让 Router 输出 validate、policy、run、verify 的命令序列。

MCP 调用固定 argv，输入经 stdin 或明确参数传递，禁止 shell。每次最多一个 Machine CLI 目标。

## Task Card Compiler v2

Compiler 只消费结构化、已确认 handoff contract。缺少 `task_card_requested` 或 `confirmed_handoff_contract` 时停止；它不读取原始聊天、不选择技能、不重新形成方案。

## Light Task

低风险、边界明确、验证快速。默认 `execute-and-verify`。

## Medium Task

跨多个文件或需要更完整验证，但边界仍明确。默认 `execute-and-verify`。

## Heavy Task

跨系统、核心协议、迁移、发布或高回滚成本。默认 `plan-only`；若结构化任务卡明确授权 `execute-and-verify`，可执行并增加独立 review gate。

## Escalation Rules

安全边界、任务级别和 permission mode 由结构化 Policy/Gate 决定，不由 Skill Resolver 决定。技能选择不能降级或升级执行权限。

## Review Gate Defaults

Light 按风险选择 review；Medium 通常需要 review；Heavy 必须独立 review。release/destructive 等边界优先于默认规则。

## Skill Tag Rules

任务卡 skill tag 必须同时满足 registry、invoke hint 和 ActiveSkillTable 三闸。快照 stale 或 demand 不可用时停止，不自动替代。

## Task Handoff Protocol

明确交接请求 + 已确认 handoff contract 才能编译任务卡。方案确认本身不等于交接授权。

## Prompt Generation Requirements

生成给执行器的提示词时只引用已确认 contract，不把原始对话重新解释成新方案，也不添加未经选择的技能或 CLI 工作流。
