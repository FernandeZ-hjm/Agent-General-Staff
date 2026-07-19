# Skill Governance Protocol

> AGS 0.3.0 的机器私有技能生命周期、统一目录与确定性解析协议。

## Source of Truth

- `manifests/skills-registry.yaml`：官方/团队技能身份、canonical source、routing/auth metadata；优先级最高。
- `manifests/suite.yaml`：suite required / optional / personal 集合。
- `<runtime_home>/skill-registry/user-overlay.yaml`：机器私有 adopt/ignore/rollback overlay，不入库。
- `<runtime_home>/capability-snapshot/<host>.json`：每宿主单一能力快照，不入库。
- `<runtime_home>/skill-usage/<host>.ndjson`：非敏感、append-only outcome ledger，不入库。

## 统一候选目录

`HostCapabilitySnapshot` 统一发现：

- suite roots；
- 宿主 `.system` 技能；
- `~/.agents/skills` 等用户安装技能；
- project-local 技能；
- 宿主配置明确证明 enabled 的 plugin roots。

不得遍历整棵 disabled plugin cache。候选技能也进入 `catalog`，但只有 `governance=Active` 且 `availability=Ready` 的技能进入 `active_skills`。

## SkillCard 与 ActiveSkill

薄 `SkillCard` 至少包含：`skill_id`、展示名、summary、`intent_tags`、entrypoints、source kind、governance、availability/reason codes、requires_auth/AuthState、version/source hash 和 activity。

`ActiveSkill` 以 `skill_id + allowed entrypoints + invoke_hint` 精确索引。Resolver 只接受精确 `SkillTarget`，不读取自然语言、不做关键词/相似度/fallback。旧 `SkillDemand` 与 `demand_routes` 只保留为 intent metadata 与旧序列化兼容输入。

## 正交状态

以下事实保持正交，不压成一个互斥大状态机：ManagedStatus、RegistryStatus、RouteState、HealthStatus、HostVisibility、AuthState。

派生状态：

- governance：`Discovered | Candidate | ManagedInactive | Active | Ignored | Retired`
- availability：`Ready | Degraded(reason_codes) | Unavailable(reason_codes)`
- activity：`Unobserved | Warm | Cold`

reason codes 至少覆盖：`candidate_requires_adoption`、`registry_not_routable`、`retired`、`canonical_missing`、`host_not_visible`、`health_degraded`、`auth_required`、`metadata_incomplete`、`snapshot_stale`。

requires_auth 的技能只有在不含 secret 的 runtime `AuthState=satisfied` 时才可 Ready。tracked registry 不得保存凭据或伪造认证完成状态。

## Snapshot determinism

每个宿主只有一个 `HostCapabilitySnapshot`：

```text
schema_version + host
+ registry_hash + overlay_hash + runtime_hash
+ catalog_hash + active_table_hash
→ snapshot_hash
```

时间戳和 activity 不参与 catalog/snapshot/lease hash。快照缺失、篡改或任一绑定 hash 漂移时 fail closed；显式刷新使用 `ags capability snapshot --host <host> --write`。

## 私有 Overlay 生命周期

前台命令：

```bash
ags skill adopt <skill-id> [--apply]
ags skill ignore <skill-id> [--apply]
ags skill rollback <skill-id> --to <revision> [--apply]
```

默认 dry-run。`--apply` 使用 0600、同目录原子替换，并追加 mutation receipt；记录 revision、source_hash、metadata_version 与 before/after。隐藏 `propose` 只作 deprecated wrapper，并调用同一服务。

硬规则：

- overlay 只能纳管 external/user/project/enabled-plugin 候选；
- suite tracked registry 对官方/团队 ID 永远优先；
- overlay 不能 shadow 官方 ID、覆盖 retired 条目或改写官方 routing/auth；
- 写失败必须保持或恢复到前一 revision；
- 所有测试使用临时 HOME/runtime/host roots。

## Usage 与冷技能闭环

只有 `ags_apply_action` 可以为受控技能动作追加 outcome。ledger 仅记录：event id、timestamp、request fingerprint、proposal/decision/lease id、skill id、entrypoint、`succeeded|failed|abandoned` 和非敏感质量字段；禁止 raw prompt、凭据和绝对路径。

activity：

- 从未有 outcome：`Unobserved`；
- Active 连续 30 天无 outcome，或最后 outcome 超过 90 天：`Cold`；
- 其他：`Warm`。

Cold 只提示，不影响 snapshot hash/lease，不自动 ignore/retire。每个不与 MachineCli 共存的精确 SkillTarget 都会得到一个连接内受控 outcome action，solution formation 与 direct-edit 使用同一记录路径。route correction 用旧 decision 的 `outcome=abandoned` 表示，宿主须在提交新 route 前消费旧 outcome action；离线评估按相同 `request_fingerprint` 关联前后 decision。correction 与 outcome 都不得自动改 registry、overlay 或生产路由。

## Task Card Gate

任务卡 `[skill: name]` 仍必须同时满足：registry routable、合法 invoke_hint、机器快照 Active+Ready。任一失败即停止；不得替换成职责相似技能。技能永远不能改变 task level、permission、review、verification、protected path 或 release boundary。

## Public / Stable Projection

只投影协议、schema 与官方 registry。真实 catalog snapshot、user overlay、usage ledger、lease、auth state、runtime receipt 和机器绝对路径必须排除在 tracked diff、stable/public fixture 与 release payload 之外。
