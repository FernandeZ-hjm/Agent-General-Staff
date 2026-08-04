# Skill Governance Protocol

> AGS 使用显式刷新、请求路径只读的静态能力模型；纯第三方 Skill 可通过机器私有维护面纳管。

## 权威数据

- `manifests/skills-registry.yaml`：AGS Skill 元数据与路由权威。
- `manifests/third-party-capabilities.yaml`：经审查的第三方能力版本清单。
- `<runtime_home>/skill-registry/private-skills.json`：机器私有第三方 Skill 来源、许可证、语义元数据与目标宿主记录。
- `<runtime_home>/skill-bodies/<skill-id>/<content-hash>/`：审计后发布的机器私有不可变 body。
- `<runtime_home>/capability-snapshot/<host>.json`：安装或更新时生成的一份宿主快照。

请求路径不维护 overlay、source registry、usage ledger、adoption plan 或历史 snapshot
bundle，AGS 路由只读取当前静态快照。显式 `ags skill adopt/remove` 是独立维护事务：
状态只写入 runtime home 与宿主薄索引，不写 AGS 仓库，不让 suite manifest 冒充第三方
正文权威。

## 生命周期

```text
官方能力：审查上游版本 → 更新仓库内权威清单/canonical body → setup/update
纯第三方：外部工具拉取 → adopt plan → 人工确认 plan hash → adopt apply
→ 写入不可变私有 body / registry / 宿主薄索引
→ 为 codex / claude-code / omp / cursor / codebuddy-code 各生成一份快照
→ preflight
→ read ags://capabilities/current-host
→ typed HostRouteProposal
→ exact route
→ optional leased apply
```

preflight、resource read、route 和 MCP apply 不联网、不抓取仓库、不比较上游、不写快照，
也没有 `bundle_epoch`。相同静态文件始终产生相同 `snapshot_hash`。显式替换快照后，
旧连接/lease fail closed，调用方重新 preflight。

`ags skill adopt` 本身也不联网、不执行第三方文件；它只接受本地 Skill 目录或
`SKILL.md`。审计拒绝 symlink、特殊文件、超限 body、缺失许可证与官方 skill id 冲突。
可选 `--metadata` 文件补充机器私有的 summary、intent tags、正反例、entrypoints、
invoke hint、auth 与版本，并与源码、许可证一起绑定进计划哈希。apply 必须提交完全相同
的 `--plan-hash`；任一输入漂移都 fail closed。

## 路由语义

安装可见不等于可路由。`SkillTarget` 必须同时满足：

1. 注册表 `route_state: routable`；
2. 当前宿主快照中 `Active + Ready`；
3. `skill_id` 与可选 entrypoint 精确存在；
4. proposal 的 `snapshot_hash` 与 preflight binding 相同。

`ags-setup`、`ags-init`、`ags-doctor`、`ags-agents` 等前台命令技能是
`not-routable`，由宿主直接调用 CLI。若宿主错误地把它们提交为 MCP
`SkillTarget`，路由器返回 `skill_target_kind_mismatch`，并给出对应 CLI
命令提示；这不是安装或快照错误。`ags-skill` 是这组命令技能中唯一允许作为
`SkillTarget` 路由的例外。不得通过刷新快照改变能力的路由类型。

MCP 与 Skill 共享同一份宿主静态能力快照和一次自然语言选择面，但执行面不同。
`McpTarget` 必须同时满足：

1. `mcp-registry.yaml` 中父服务器 `route_state: routable`；
2. 显式 snapshot 刷新时，当前宿主只读探针确认服务器已注册且 active；
3. `mcp_id` 精确存在；若给出 `tool`，它必须来自该父服务器的已登记 tool route target；
4. proposal 的 `snapshot_hash` 与 preflight binding 相同。

快照把可路由父服务器写入 `active_mcps`，把语义卡片写入 `mcp_catalog`。route
只返回 host-native MCP dispatch 元数据，不建立第三方 MCP 动作、不代理调用。
注册、启用、升级或移除 MCP 后，必须显式刷新快照并重新 preflight。

## OMP

OMP 使用独立 host id `omp` 和独立静态快照。setup/update 必须生成 OMP snapshot；
route、verify 和 lease 绑定都使用 `omp`，不得复用其他宿主的快照。

## 写入边界

`ags capability snapshot --host <host> --write` 只允许在显式 setup/update 工作中调用；
`ags skill adopt/remove` 可在其单次私有事务内重建所选宿主快照。registry、宿主薄索引与
快照都先捕获事务内备份，失败时恢复；新 body 只有完整复制并复算内容哈希后才原子发布。
remove 保留不可变 body 以便人工恢复，但从 registry、宿主索引和 active snapshot 中撤销。

## 私有纳管命令

```bash
ags skill adopt <local-skill-dir> --metadata <routing.yaml> --host codex --format json
ags skill adopt <local-skill-dir> --metadata <routing.yaml> --host codex --yes --plan-hash <hash>
ags skill status <skill-id>
ags skill remove <skill-id> --format json
ags skill remove <skill-id> --yes --plan-hash <hash>
```

宿主仍使用完整任务上下文做唯一自然语言判断。私有 registry 投影出的 SkillCard 提供
summary、intent tags 与正反例；Resolver 不做模糊匹配，只校验宿主提交的精确 skill id
是否在 preflight 绑定的 ActiveSkillTable 中。
