# Skill Governance Protocol

Agent Governance Suite 技能治理协议。定义本地 Agent 技能的 source of truth、候选来源层级、
治理生命周期和写入规则。本文件是技能同步、adoption log、ignore list 和 suite manifest 的
协议权威源。

**这是唯一 canonical 技能治理协议。不得为 Cursor、Codex、Claude Code 各自创建独立技能治理文档。**

## Source of Truth

本地仓内套件资产是技能治理的 **唯一 source of truth**：

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
3. Log：在 adoption log 追加 adoption entry
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

## 当前公开版实现状态

公开版提供技能治理的**协议规范、推荐/说明和只读边界**。以下内容不在公开版范围内：

- 私有技能审计日志（`governance/skill-adoption-log.yaml`、`governance/skill-ignore-list.yaml`）
- 写入型技能 CLI 命令（`ags skill scan|check|propose|adopt|apply|rollback`）
- 预打包技能目录（`global-skills/`、`skill-packs/`）

公开版用户如需安装第三方开发技能，请参考 `docs/skill-recommendations.md`
中的推荐列表和手动安装说明。所有第三方技能必须由用户自行选择可信来源并手动安装。
AGS 公开版默认安装不会 clone、下载、curl 或写入任何用户技能目录。

## 协议引用

- 套件 manifest schema：`manifests/suite.yaml`
- 任务卡模板（技能治理补充）：`protocol/task-card-template.md`
- 套件级协议概述：`AGENT_SUITE_PROTOCOL.md`
