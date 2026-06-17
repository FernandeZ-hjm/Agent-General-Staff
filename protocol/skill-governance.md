# Skill Governance Protocol

Agent Governance Suite 技能治理总协议。定义本地 Agent 技能的 source of truth、候选来源层级、
治理生命周期和写入规则。本文件是技能同步、adoption log、ignore list 和 suite manifest 的
协议权威源。

**这是唯一 canonical 技能治理协议。不得为 Cursor、Codex、Claude Code 各自创建独立技能治理文档。**

## Source of Truth

本地仓内套件资产是技能治理的 **唯一 source of truth**：

- `governance/skill-adoption-log.yaml` — 已接纳技能的 append-only 审计日志
- `governance/skill-ignore-list.yaml` — 已拒绝/忽略技能的审计日志
- `manifests/suite.yaml` — 套件 manifest，声明 required/optional/personal 技能

以下来源仅作为 **候选来源**，不得直接作为 source of truth：

- GitHub 仓库（包括官方 skill 仓库、社区 skill 仓库）
- 插件市场 / CLI 工具输出（`npx skills add`、`lark-cli update` 等）
- 本地拖拽目录 / 手动拷贝的技能目录
- 外部 Agent 运行时自动下载的技能缓存

所有候选来源的技能必须经过完整治理生命周期后才能进入 adoption log 和 suite manifest。

## 治理生命周期

技能从候选来源进入套件 manifest 必须经过以下阶段，不得跳过：

### 1. Discover（发现）

识别候选技能来源。可以来自：

- 套件 manifest 中声明的 optional 技能引用
- 用户手动指定的路径、URL 或技能名
- 项目 profile 或 context capsule 中引用的技能

此阶段只做 inventory，不修改任何文件。

### 2. Scan（扫描）

对候选技能执行只读安全检查：

- 文件结构完整性（是否包含 SKILL.md 或等价入口）
- 已知风险模式匹配（`curl | bash`、`eval`、未审查网络请求等）
- 与现有 ignore list 交叉比对
- 与已接纳技能的冲突检测（同名、同路径、同功能）

Scan 结果写入 proposal，不做 adoption/ignore 决策。

### 3. Proposal（提案）

基于 scan 结果生成 adoption proposal。Proposal 包含：

- 技能名、来源、hash
- Scan 发现的风险项（无风险则注明 clean）
- 建议决策：adopt / ignore / defer
- 若建议 adopt：列出目标路径、与现有技能的交互影响
- 若建议 ignore：列出 risk category 和 review date

Proposal 是只读建议，不执行写入。必须等待人工确认后才能进入 adopt 或 ignore 阶段。

### 4. Dry-Run（干运行）

在人工确认 adopt 后、实际写入前，必须执行 dry-run：

- 展示将要写入的文件列表和路径
- 展示 diff（新增、修改、覆盖）
- 展示将要更新的 adoption log entry
- 如涉及用户目录（`$HOME/.agents/skills` 等），显式标红警告

Dry-run 不得产生任何文件系统副作用。只有 dry-run 通过且人工二次确认后，才能进入写入阶段。

### 5. Apply（写入）

按以下顺序写入：

1. Backup：如目标路径已有同名技能，先备份到 `governance/backups/`
2. Write：将技能文件写入目标路径
3. Log：在 `governance/skill-adoption-log.yaml` 追加 adoption entry
4. Manifest：更新 `manifests/suite.yaml` 的 required/optional 列表

写入阶段不得：

- 静默覆盖用户目录（`$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`）
- 在执行 `npx skills add/remove/update`、`lark-cli update` 或任何外部 CLI 安装命令
- 在未先 backup 的情况下覆盖已有技能
- 在 dry-run 未通过或人工未确认的情况下写入

### 6. Verify（验证）

写入后验证：

- 目标文件存在且内容与 dry-run diff 一致
- Adoption log entry 格式正确，可被 YAML parser 解析
- Suite manifest 更新正确
- 已有技能未受影响（hash 比对）

## 写入规则（硬门禁）

所有技能写入操作必须遵守以下规则。违反任何一条即停止，不得继续：

1. **Dry-run 先行**：任何写入操作前必须完成 dry-run 并展示 diff
2. **Diff before apply**：人工必须在看到 diff 后才能确认 apply
3. **人工确认**：adopt / ignore / rollback 决策不得由 Agent 自动做出
4. **禁止静默覆盖用户目录**：任何对 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache` 的写入必须显式标出并要求独立确认
5. **Backup before mutate**：覆盖已有技能前必须先备份
6. **Append-only log**：adoption log 和 ignore list 只能追加或显式 supersede，不得删除历史记录
7. **不得接管外部 CLI**：不得运行 `lark-cli update`、`npx skills add/remove/update` 或等价命令
8. **不得自动安装未审查技能**：所有新技能必须走完整 scan → proposal → dry-run → confirm → apply 链路

## 与 AGS 任务卡系统的关系

技能治理任务使用 AGS 标准任务卡骨架（`protocol/task-card-template.md`），并补充以下规则：

- `适用治理文档` 填写本文件路径
- `非目标` 明确禁止写入用户目录、运行外部 CLI、自动应用 patch
- `实施要求` 说明默认先 scan / dry-run，人工确认后才能 adopt / ignore
- 涉及 `notebooklm`、Hermes 输出层技能、TempoFlow 输出层业务契约时，必须注明只可引用不可 adopt

详细填槽规则见 `protocol/task-card-template.md` 的 "Skill Governance 治理任务补充" 章节。

## 技能边界

以下技能类型不在套件治理范围内，只能被引用，不能被 adopt / update / 打包：

- **外部官方 CLI 技能**：`notebooklm`、`lark-*` 系列（飞书开放平台技能）等由外部 CLI 或服务管理的技能
- **输出层业务技能**：Hermes 输出层技能、TempoFlow 输出层业务契约为业务运行时契约，不得改写为开发套件任务卡或技能治理对象
- **项目自管输出层技能**：项目内 `output/`、`dist/` 等自管输出目录下的技能，治理权归项目自身

套件分发的 public-safe 技能清单由 `manifests/suite.yaml` 的 allowlist 规则（见
`AGENT_SUITE_PROTOCOL.md` public-full sanitized 发布边界）和 `governance/skill-ignore-list.yaml`
共同决定。

## 第三方技能与 MCP 纳管控制台（`ags skill` console）

`ags skill` 不只是静态账本检查，而是本机第三方技能、MCP、CLI-backed
capability 的统一纳管控制台。控制台模型与写入边界由
`crates/skill-governance/src/console.rs` 实现，CLI 入口在 `crates/ags-cli`。

### Canonical 本体 + per-host thin index（核心心智模型）

AGS 电脑上每个能力**只保留一套 canonical 本体**：技能本体（含 `references/`、
`scripts/` 等依赖文件）、MCP 定义、hook 本体由 AGS 统一管理一份。各宿主
（Claude Code、Codex、Cursor）**只拥有薄索引（thin index）**——一个指回 canonical
本体的可发现入口，宿主重启后即可识别。

- **canonical 本体**：AGS 唯一来源（仓内 `global-skills/` / `skill-packs/` 或等价
  canonical 目录），由 `canonical_present` 建模。
- **thin index**：宿主入口 `<host>/skills/<name>`，实现为 **symlink → canonical 目录**
  （本机实测 Claude Code 用相对 symlink、Codex 用绝对 symlink，均指向
  `~/.agents/skills/`）。symlink 让 `references/` 等依赖文件随本体一起可达，**绝不
  逐文件复制 SKILL.md**（否则 Lark 类技能会丢失 `references/` 而运行时不可用）。
- **角色边界（goal 7）**：做方案的当前 agent 可使用**完整工具本体**（canonical 全部
  文件）；其他 agent / 其他宿主**只需要 thin-index 可见性**即可发现并调用能力，不需要
  也不应各自复制一套本体。

### 统一能力模型

每个被纳管对象建模为一个 `ManagedCapability`，至少包含以下结构化字段：

| 字段 | 含义 |
|---|---|
| `kind` | `skill` / `mcp` / `suite-interface` / `cli-backed` |
| `name` | 能力名 |
| `source` | canonical 来源路径或注册表引用 |
| `canonical_present` | AGS 是否持有 canonical 本体（与 host visibility 分开建模） |
| `managed_status` | `suite-managed` / `governed` / `suite-interface` / `discovered` / `ignored` / `unmanaged` |
| `registry_status` | `registered` / `not-registered`（是否在 AGS 注册表内） |
| `host_visibility` | 每个宿主一条 thin-index 可见性：`visible` / `not-visible` / `degraded` / `unsupported` / `deferred` |
| `health_status` | 运行时健康：`healthy` / `degraded` / `unknown` / `unhealthy`（与 host visibility 分开建模） |
| `actions` | 控制台对该能力开放的动作 |
| `risk_notes` | 风险/边界提示 |

默认 `ags skill` inventory 必须同时展示 canonical 本体状态与各宿主 thin-index
可见性（claude-code + codex），不得只探测单一宿主。

inventory 来源：suite manifest 技能、本机 skill 目录（`global-skills/` /
`skill-packs/`）、`manifests/mcp-registry.yaml` 的 `mcps:`（被治理 MCP）与
`suite_interfaces:`（AGS 自身，host initialization adapter，**不是**被治理第三方
MCP），以及 CLI-backed 家族（如 `lark-cli`）。

### Host visibility 与 runtime health 分层

以下是**不同**证据，不得混为一谈（参见 `tests/fixtures/skill-console-lark.md`）：

1. skill 源文件在仓内存在；
2. 宿主 skill 可加载（`~/.claude/skills/<name>/SKILL.md` 可解析，symlink 目标存在）；
3. MCP server 已在宿主注册（`claude mcp list/get` 可发现）；
4. MCP 已连接（runtime health）；
5. 外部 endpoint doctor 通过（如 Feishu/Lark，只能作为 degraded observation，
   不得依赖真实网络或真实账号）。

Claude Code 与 Codex 宿主检查均已实现（skill path：`~/.claude/skills` /
`~/.codex/skills`，symlink-aware；MCP 注册：`claude mcp list` / `codex mcp list`）。
Cursor 为预留接口，返回 `deferred`，但模型与 JSON 字段保持稳定。宿主 CLI 不可用时
返回 `degraded`，不得 panic。expected host 由 required 技能 + 注册表
`installed_clients` 判定，逐宿主独立。

### 纳管链路（硬性顺序）

```
discover → scan → propose → dry-run → confirm/apply → host restart → verify → audit evidence
```

- **discover / scan**：只读盘点本机技能、MCP、CLI-backed capability（`ags skill`、
  `ags skill scan`、`ags skill check`）。
- **propose**：对 discovered capability 生成 adopt / update / remove / uninstall /
  repair / verify 的 dry-run 提案（`ags skill propose --action <verb> --skill <name>`）。
- **dry-run**：默认行为。无确认参数时只输出计划，**不写用户目录、不写宿主配置、
  不运行外部安装器**。
- **confirm/apply**：仅当显式传入 `--apply` 时执行受保护写入。所有写入集中经过
  一个 guard，且**只写 thin index**——对每个支持的宿主在 `<host>/skills/<name>`
  创建 symlink → canonical 本体（覆盖前把旧入口整体 rename 到 `.bak`）。技能 adopt /
  update / repair 不复制本体；remove / uninstall 只移走 thin index，canonical 本体
  原样保留。MCP 仍只 advise（`claude mcp add/remove` / `codex mcp add/remove`），AGS
  永不运行 `npx skills add/remove`、`lark-cli update` 或任何外部安装/注册命令。写入
  目标必经 containment 断言，限定在各宿主 skills 根之内。
- **host restart**：apply 后提示重启 Claude Code / Codex / Cursor 以重扫 thin index。
- **verify**：`ags skill verify --host <host>` 重启后复核宿主可见性。
- **audit evidence**：在 delivery report / receipt 中记录 dry-run 计划、apply 写入
  清单、host visibility 证据；adoption log 仍按既有 append-only 规则记录采纳决策。

### 写入边界（与上文硬门禁一致）

- 默认命令（`ags skill` / `scan` / `check` / `propose` / `verify`）只读。
- 任何真实写入或外部命令必须由显式确认参数（`--apply`）保护。
- 写入型代码路径必须支持测试注入 root / home / config 目录；测试只能使用临时目录和
  mock command，不得触碰真实 `$HOME`。
- 第三方能力默认 opt-in；控制台不得 silently bundle 或 silently install。
- AGS 自身（`suite_interfaces.ags`）是治理权威，不能通过控制台 adopt / remove。

### 写入与验证的失败语义（硬化要求）

- **apply 失败必须上抛，advised-only 不算 applied**：`applied=true` 仅当**至少有一
  个 AGS 自有写入被规划且全部成功**；任一失败进 `apply_errors`、`applied=false`、CLI
  退出非零。MCP/CLI-backed 动作 AGS 不写任何文件、只 advise——`apply_status` 标为
  `advised-only`、`applied=false`，`--apply` 时 CLI 退出非零（提示用户自行执行 advised
  命令），不得报告为已执行。`apply_status` 取值：`dry-run` / `applied` / `failed` /
  `advised-only` / `nothing-to-do` / `blocked`。
- **写入必须事务化 + 多宿主预检**：每个宿主的 relink 走 stage→backup→atomic swap，
  任一步失败自动回滚，绝不把现有入口删一半。多宿主 apply 先**预检所有目标**（containment
  + skills 目录可创建），任一宿主预检失败则**零变更中止**，后面的宿主失败不会让前面的
  宿主处于半改状态。
- **路径必须收敛**：capability 名落盘前先校验为安全单段路径（拒绝 `/`、`\`、`..`、
  绝对路径、多段路径）；guard 再断言写入目标位于各宿主 skills 根之内。
- **canonical 源必须收敛**：symlink 目标必须 canonicalize 后落在已批准的 canonical
  store（`global-skills/` / `skill-packs/`）之内，并且 canonical `SKILL.md` 的
  front-matter `name` 必须与 capability name 一致；坏的/陈旧的 manifest source（绝对
  路径、`..` 逃逸、store 外目录、名字不符）一律 blocked，不得把任意本地目录暴露为宿主
  可加载技能本体。
- **verify 必须反映不可见**：`host_visibility` 区分 `expected`（按 required 技能 +
  注册表 `installed_clients` 判定）。存在 expected 但不可见的能力时，verify
  `status=incomplete`、`all_visible=false`；`ags skill verify --strict` 在
  `status!=ok` 时退出非零，作为 apply 后的门禁；默认无 `--strict` 为只读信息模式。
- **verify 必须反映不可见**：`host_visibility` 区分 `expected`（按 required 技能 +
  注册表 `installed_clients` 判定）。存在 expected 但不可见的能力时，verify
  `status=incomplete`、`all_visible=false`；`ags skill verify --strict` 在
  `status!=ok` 时退出非零，作为 apply 后的门禁；默认无 `--strict` 为只读信息模式。

## 当前实现状态

当前 2.5 私有主库已从旧套件备份迁入技能治理分类账，并将 `ags skill`
实现为第三方技能与 MCP 纳管控制台的第一版可执行闭环：

- **已落地**：本协议、`governance/skill-sync.md`、`governance/skill-adoption-log.yaml`、`governance/skill-ignore-list.yaml`、`manifests/suite.yaml`、`manifests/skills-registry.yaml`、`manifests/mcp-registry.yaml`、`global-skills/`、`skill-packs/optional/`、`skill-packs/personal/`
- **已实现只读入口**：`ags skill`（统一 inventory）、`ags skill scan`、`ags skill check`、`ags skill propose`（dry-run）、`ags skill verify --host <host>`、`ags skill inventory`
- **已实现受确认保护的 apply 路径**：`ags skill propose --action <verb> --skill <name> --apply` 经单一 guard 执行 AGS 自有宿主入口写入（覆盖前 backup）；无 `--apply` 时只 dry-run
- **已迁移分类**：required 核心开发技能、optional 集成/第三方技能包、personal 用户风格技能、ignored 外部/不纳管技能
- **仍不接管外部 CLI**：`npx skills add/remove/update`、`lark-cli update`、`claude mcp add/remove` 等外部安装/注册命令一律只 advise、永不执行；AGS 只做纳管、提案、确认保护、入口分发和验证

## 协议引用

- 同步阶段边界：`governance/skill-sync.md`
- 采纳日志 schema：`governance/skill-adoption-log.yaml`
- 忽略列表 schema：`governance/skill-ignore-list.yaml`
- 套件 manifest schema：`manifests/suite.yaml`
- 任务卡模板（技能治理补充）：`protocol/task-card-template.md`
- 套件级协议概述（public-full sanitized 边界）：`AGENT_SUITE_PROTOCOL.md`
