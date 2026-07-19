# Skill Lifecycle Governance

> AGS 0.3.0 机器私有技能生命周期的实现边界。本文件补充
> `protocol/skill-governance.md`，不另建路由权威。

## 权威与数据边界

技能身份与机器状态分开保存：

- `manifests/skills-registry.yaml` 是官方/团队技能的 tracked 权威；
- `manifests/suite.yaml` 描述套件 required、optional 与 personal 集合；
- `<runtime_home>/skill-registry/user-overlay.yaml` 只保存本机对外部、用户、项目及
  enabled-plugin 候选的 adopt/ignore/rollback 决定；
- `<runtime_home>/skill-registry/user-overlay-events.ndjson` 是 append-only mutation
  receipt；
- `<runtime_home>/capability-snapshot/<host>.json` 是宿主绑定的统一候选目录；
- `<runtime_home>/skill-usage/<host>.ndjson` 是非敏感 outcome ledger。

overlay、snapshot、usage、auth、lease 与 runtime receipt 都是机器私有数据，不得进入
tracked diff、stable/public fixture 或 release payload。旧
`governance/skill-adoption-log.yaml` 与 `governance/skill-ignore-list.yaml` 仅作为迁移和
历史审计资料，不再决定 0.3.0 的运行时可路由状态。

## 单一生命周期链路

```text
本机来源发现（只读、零宿主进程）
  → HostCapabilitySnapshot.catalog
  → 宿主基于完整对话选择精确 skill_id / entrypoint
  → Skill Resolver 按 snapshot_hash 精确校验
  → 宿主执行技能
  → ags_apply_action 追加受控 outcome
  → 离线 activity / quality 评估
```

发现范围只有：suite roots、宿主 `.system`、用户安装目录、project-local，以及宿主配置
明确证明 enabled 的插件 roots。不得遍历整棵 disabled plugin cache。候选可进入
`catalog`，但只有 `Active + Ready` 进入 `active_skills`。

宿主是唯一自然语言语义节点。AGS 不按名称、summary、tag 或旧 `SkillDemand` 猜技能，
也不做相似度、关键词、fallback 或自动替代。

## 前台命令

```bash
ags skill adopt <skill-id> [--host <host>] [--apply]
ags skill ignore <skill-id> [--host <host>] [--apply]
ags skill rollback <skill-id> --to <revision> [--host <host>] [--apply]
```

三条命令默认 dry-run。`--apply` 只修改机器私有 overlay，追加 mutation receipt，并刷新
对应宿主 snapshot；不会复制、下载或删除技能本体，也不会运行外部 installer、registrar
或任意 shell。隐藏的 `ags skill propose` 是 0.2 兼容 wrapper，生命周期动作必须委托给
同一 overlay 服务，不得保留第二套写入实现。

`ags skill`、`ags skill inventory`、`ags skill verify` 仍提供只读盘点和可见性诊断。
`scan/check` 是只读兼容面。它们可以显示旧治理资料，但不能把旧日志重新提升为路由或
overlay 权威。

## Adopt

`adopt` 只接受当前统一目录中的精确候选。允许的来源是 external、user、project 与
enabled-plugin；以下情况 fail closed：

- skill id 已由 tracked registry 定义；
- 官方条目为 retired；
- 候选 metadata 不完整或 source hash 不可确定；
- 候选来源不在允许集合；
- 当前 snapshot 或候选证据已经漂移。

dry-run 返回拟写 entry、下一 revision 与相对 runtime 路径。`--apply` 后，该 entry 才能
参与下一份 snapshot 的治理状态派生。

## Ignore

`ignore` 为候选或已 adopt 的本机技能写入 `Ignored` overlay entry。即使技能本体随后从
发现目录消失，只要 overlay 中已有受控 entry，仍可基于原 metadata 执行 ignore；这样
不会因下载目录变化而丢失治理决定。

Ignore 不删除技能本体、不修改官方 registry，也不自动触发 thin-index 清理。

## Rollback

`rollback --to <revision>` 从 append-only mutation receipt 恢复指定技能的历史 metadata
和状态；`--to 0` 删除该技能的 overlay entry。回滚本身产生新的全局 overlay revision 和
mutation receipt，不覆盖历史记录。

receipt 历史缺失、目标 revision 不存在或 source identity 不一致时必须停止。写 overlay
成功但 receipt 追加失败时，原子恢复先前文件。

## 写入保证

所有 overlay 写入都必须满足：

- 同目录临时文件 + 原子替换；
- 文件权限 0600；
- revision 单调递增；
- entry 保存 `source_hash` 与 `metadata_version`；
- mutation receipt 保存 before/after，但不保存 prompt、凭据或机器绝对路径；
- tracked registry 对同名官方/团队 ID 永远优先，overlay 不能 shadow 或改写其
  routing/auth metadata。

测试必须使用临时 HOME、runtime_home 与 host roots。不得把当前机器真实
`~/.agents/skills`、`~/.codex/skills`、插件 cache 或 runtime 当作写入夹具。

## 与 thin-index 分发的关系

生命周期决定“这个本机候选是否可进入 ActiveSkill”；thin-index 分发决定“已有技能本体
如何被其他宿主看见”。两者不是同一个写入面：

- `ags skill adopt/ignore/rollback`：只写私有 overlay、receipt、snapshot；
- `ags skill sync` / `ags capability sync`：单独规划或应用 AGS-owned host thin-index；
- 外部 MCP、CLI 与 installer 注册：始终 advise-only，AGS 不代为执行。

因此 adopt 不等于安装或跨宿主同步，sync 也不能绕过 overlay/registry 让候选变成可路由
技能。

## Usage 与冷技能

技能 outcome 只可由连接内 `ags_apply_action` 追加。ledger 只含 event、request
fingerprint、proposal/decision/lease、skill/entrypoint、outcome 与非敏感质量字段。

- 从未有 outcome：`Unobserved`；
- Active 连续 30 天无 outcome，或最后 outcome 超过 90 天：`Cold`；
- 其他：`Warm`。

Cold 只提示，不影响 snapshot/lease hash，不自动 ignore、retire、更新 registry 或改变
生产路由。route correction 与 outcome 只供离线评估。

## 验证重点

- 全来源发现且 disabled plugin cache 被排除；
- 官方 registry 优先，overlay 无法 shadow 或改写 retired/routing/auth；
- adopt/ignore/rollback dry-run 零写入，apply 为 0600 原子写和 append-only receipt；
- snapshot/hash 确定，activity 与时间戳不参与 hash；
- auth state 不含 secret，未满足时不可 Ready；
- usage 不含 raw prompt、credential 或绝对路径；
- public-safe 投影排除全部机器私有 lifecycle 数据。

## 协议引用

- `protocol/skill-governance.md`
- `protocol/task-routing.md`
- `protocol/mcp-server.md`
- `manifests/skills-registry.yaml`
- `manifests/suite.yaml`
