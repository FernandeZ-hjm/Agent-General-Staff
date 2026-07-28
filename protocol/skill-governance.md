# Skill Governance Protocol

> AGS 0.3.5 使用显式刷新、运行时只读的静态能力模型。

## 权威数据

- `manifests/skills-registry.yaml`：AGS Skill 元数据与路由权威。
- `manifests/third-party-capabilities.yaml`：经审查的第三方能力版本清单。
- `<runtime_home>/capability-snapshot/<host>.json`：安装或更新时生成的一份宿主快照。

运行时不维护 user overlay、source registry、usage ledger、adoption plan 或历史
snapshot bundle。canonical body 仍由仓库或明确的外部 manager 持有，但 AGS 路由
只读取当前静态快照。

## 生命周期

```text
审查上游版本
→ 更新仓库内权威清单/canonical body
→ 运行显式 setup 或 update
→ 为 codex / claude-code / omp / cursor / codebuddy-code 各生成一份快照
→ preflight
→ read ags://capabilities/current-host
→ typed HostRouteProposal
→ exact route
→ optional leased apply
```

preflight、resource read、route 和 apply 不联网、不抓取仓库、不比较上游、不写快照，
也没有 `bundle_epoch`。相同静态文件始终产生相同 `snapshot_hash`。显式替换快照后，
旧连接/lease fail closed，调用方重新 preflight。

## 路由语义

安装可见不等于可路由。`SkillTarget` 必须同时满足：

1. 注册表 `route_state: routable`；
2. 当前宿主快照中 `Active + Ready`；
3. `skill_id` 与可选 entrypoint 精确存在；
4. proposal 的 `snapshot_hash` 与 preflight binding 相同。

`ags-setup`、`ags-init`、`ags-doctor`、`ags-agents` 是宿主前台命令技能，由宿主按
冻结的 CLI hint 直接调用。把它们提交为 MCP `SkillTarget` 必须返回
`skill_target_kind_mismatch` 和直接命令提示；刷新快照不会改变它们的类型。
`ags-skill` 是这组命令技能中唯一同时允许进入 `ActiveSkillTable` 的治理目标。

## OMP

OMP 使用独立 host id `omp` 和独立静态快照。setup/update 必须生成 OMP snapshot；
route、verify 和 lease 绑定都使用 `omp`，不得复用其他宿主的快照。

## 写入边界

`ags capability snapshot --host <host> --write` 只允许在显式 setup/update 工作中调用。
快照通过同目录临时文件原子替换，不生成持久副本、quarantine 或回滚计划。
进程内临时状态可以用于一次原子事务失败恢复，事务结束即销毁。
