# Skill Sync Governance

本地 Agent 技能同步的阶段边界定义。本文件是 `protocol/skill-governance.md` 的
配套阶段文档，定义 scan、candidate check、proposal、adopt、ignore、manifest、
backup、rollback 各阶段的输入、输出、门禁和范围。

## 当前范围声明

本文件定义技能同步的完整阶段边界。公开版是 public-full-sanitized：包含确认式
技能治理 CLI 和空白审计骨架，但不包含预打包技能目录（`global-skills/`、
`skill-packs/`）、已安装第三方技能、用户本地技能或带真实历史的私有技能审计日志。
`governance/skill-adoption-log.yaml` 和 `governance/skill-ignore-list.yaml` 在公开版中
只能作为 `entries: []` 的空白骨架出现，用户运行一段时间后会沉淀自己的审计记录。

公开版用户如需安装第三方开发技能，可使用 `ags skill install --skill <name> --confirm`
或参考 `docs/skill-recommendations.md` 中的推荐列表和手动安装说明。所有技能治理的写入操作必须遵守
`protocol/skill-governance.md` 的硬门禁（dry-run 先行、diff before apply、
人工确认、禁止静默覆盖用户目录、不得接管外部 CLI）。

## 阶段概览

```
候选来源（GitHub / 插件 / CLI / 拖拽目录 / 手动路径）
        │
        ▼
┌─────────────────┐
│  1. Scan        │  只读盘点：发现本地技能、候选来源技能
│     (只读)      │  输出：inventory list
└─────────────────┘
        │
        ▼
┌─────────────────┐
│  2. Candidate    │  交叉比对：inventory vs adoption log vs ignore list vs manifest
│     Check        │  输出：candidate list（new / conflict / ignored / already-adopted）
│     (只读)      │
└─────────────────┘
        │
        ▼
┌─────────────────┐
│  3. Proposal     │  为每个 candidate 生成 adoption proposal
│     (只读)      │  输出：proposal list（含 scan findings、建议决策）
└─────────────────┘
        │
        ├──────────┐
        ▼          ▼
┌──────────┐ ┌──────────┐
│ 4. Adopt │ │ 5. Ignore│  人工确认后进入
│ (dry-run │ │ (dry-run │  必须先 dry-run 展示 diff
│  → apply)│ │  → apply)│
└──────────┘ └──────────┘
        │          │
        ▼          ▼
┌──────────────────────┐
│  6. Manifest         │  更新 manifests/suite.yaml
│     更新             │
└──────────────────────┘
        │
        ▼
┌──────────────────────┐
│  7. Backup           │  任何写入前先备份目标路径已有技能
│     (adopt 前)       │
└──────────────────────┘
        │
        ▼
┌──────────────────────┐
│  8. Rollback         │  按 adoption log 的 backup_ref 回滚
│     (按需)           │  追加 rollback entry 到 adoption log
└──────────────────────┘
```

## 阶段详细定义

### 1. Scan — 只读技能盘点

**触发条件**：用户请求扫描本地技能或检查候选来源。

**输入**：
- 目标扫描目录（默认 `$HOME/.agents/skills/`，可覆盖）
- 候选来源列表（URL、路径、技能名）
- 已有 adoption log（`governance/skill-adoption-log.yaml`，如存在）
- 已有 ignore list（`governance/skill-ignore-list.yaml`，如存在）
- 已有 suite manifest（`manifests/suite.yaml`）

**处理**：
1. 遍历目标目录，收集所有技能目录/文件的路径、名称、hash
2. 对每个候选来源，获取技能元数据（名称、版本、来源 URL、hash），不下载完整内容
3. 生成 inventory list（本地技能 + 候选技能元数据）

**输出**：结构化 inventory list（技能名、路径/来源、hash、状态标记）

**门禁**：只读，不修改任何文件，不下载外部内容。

### 2. Candidate Check — 交叉比对

**触发条件**：scan 完成后自动触发，或用户手动指定候选技能。

**输入**：
- inventory list（来自 scan）
- 已有 adoption log
- 已有 ignore list
- suite manifest

**处理**：
1. 将 inventory 与 adoption log 交叉比对 — 标记 already-adopted
2. 将 inventory 与 ignore list 交叉比对 — 标记 ignored（含 ignore reason 和 expires）
3. 将 inventory 与 manifest 交叉比对 — 标记 required/optional/personal 状态
4. 检测同名冲突（不同来源的同名技能）
5. 检测路径冲突（不同技能指向相同安装路径）

**输出**：
- candidate list：状态为 new / conflict / ignored / already-adopted 的技能列表
- 每个 candidate 附对比结果和冲突说明

**门禁**：只读。如果发现高危冲突（如同名不同源的 required 技能），停止并报告，不等 proposal 阶段。

### 3. Proposal — 采纳提案生成

**触发条件**：candidate check 完成后，用户请求生成 proposal。

**输入**：
- candidate list（来自 candidate check）
- 项目 profile（`config/agent-project-profile.yaml`，如存在）
- 已知风险模式库

**处理**：
1. 对每个 candidate 执行安全扫描（结构完整性、已知风险模式）
2. 生成建议决策：adopt / ignore / defer
3. 若建议 adopt：列出目标路径、与已有技能的交互影响
4. 若建议 ignore：列出 risk category、建议 review date
5. 生成 proposal 文档（不写入 adoption log）

**输出**：
- proposal list：每个 candidate 的建议决策、scan findings、影响分析
- 汇总统计：new / conflict / clean / risky / deferred

**门禁**：只读，不执行 adopt/ignore。Proposal 必须等待人工确认。

### 4. Adopt — 采纳写入

**前置条件**：人工已确认 proposal 中的 adopt 决定。

**处理流程**：

1. **Pre-adopt check**：再次确认目标路径、hash、冲突状态（距 proposal 可能有变更）
2. **Backup**（阶段 7）：如目标路径已有同名技能，先备份到 `governance/backups/<skill-name>-<timestamp>/`
3. **Dry-run**：展示将要写入的文件列表、路径、diff（新增/修改/覆盖）；如涉及用户目录，显式标红
4. **二次确认**：人工必须确认 dry-run 结果
5. **Write**：将技能文件写入目标路径
6. **Log**：在 adoption log 追加 adoption entry（含 backup_ref）
7. **Manifest**：更新 `manifests/suite.yaml`

**adoption log entry 字段**：
- `id`：唯一 entry ID
- `skill_name`：技能名
- `profile`：来源 profile（user / project / suite）
- `source`：来源 URL 或路径
- `source_hash`：来源内容 hash
- `safety_findings`：安全扫描发现
- `decision`：adopted
- `actor`：确认采纳的人或 Agent（需人工确认）
- `timestamp`：ISO 8601
- `rollback_ref`：如为 rollback 恢复，指向旧 entry
- `backup_ref`：备份路径

**门禁**：
- Dry-run 未通过 → 停止
- 人工未二次确认 → 停止
- 涉及用户目录但未独立确认 → 停止
- 备份失败 → 停止

### 5. Ignore — 忽略/拒绝写入

**前置条件**：人工已确认 proposal 中的 ignore 决定。

**处理流程**：

1. **Dry-run**：展示将要追加到 ignore list 的 entry
2. **确认**：人工确认 ignore 决定和 risk category
3. **Write**：在 ignore list 追加 ignore entry
4. **不修改已有 entry**：即使新 entry supersedes 旧 entry，旧 entry 保留（状态改为 superseded）

**ignore list entry 字段**：
- `pattern`：技能名或来源 pattern（glob 兼容）
- `reason`：忽略原因
- `risk_category`：security / stability / license / compatibility / policy / other
- `actor`：做出决定的人
- `timestamp`：ISO 8601
- `expires`：过期时间或 "never"
- `review_date`：下次复审日期
- `supersedes`：如替换旧 entry，指向旧 entry id
- `status`：active / superseded / expired

**门禁**：
- 不允许静默删除历史 entry
- 状态变更必须通过新增 entry + `supersedes` 字段
- 过期 entry 保留原记录，状态变为 expired

### 6. Manifest — 套件清单更新

**触发条件**：adopt 或 ignore 完成后自动触发。

**处理**：
- adopt 后：将技能加入 `manifests/suite.yaml` 的 `required` 或 `optional` 列表
- ignore 后：通常不更新 manifest（技能不在套件中）
- 区分 required（套件运行必需）/ optional（识别但不强制）/ personal（用户个人，不同步到公开版）

**manifest entry 字段**（由 `manifests/suite.yaml` schema 定义）：
- `name`：技能名
- `version`：版本
- `source`：来源引用
- `hash`：内容 hash
- `profile`：required / optional / personal

**门禁**：
- required 技能只有在人工确认后才能进入 manifest
- personal profile 技能不得同步到 public 发行版

### 7. Backup — 写入前备份

**触发条件**：adopt 写入前自动触发，或用户手动请求。

**处理**：
1. 检查目标路径是否已有同名技能
2. 如已有，将现有技能整体复制到 `governance/backups/<skill-name>-<ISO8601-timestamp>/`
3. 记录 backup 路径，供 adoption log entry 的 `backup_ref` 字段引用

**门禁**：
- 备份失败 → 停止 adopt
- 备份路径冲突 → 停止并报告

### 8. Rollback — 回滚

**触发条件**：用户请求回滚某次 adoption。

**处理**：
1. 查找目标技能在 adoption log 中的最新 entry
2. 读取 `backup_ref` 指向的备份路径
3. Dry-run：展示将要恢复的备份内容和将要移除的当前版本
4. 人工确认
5. 从备份恢复
6. 在 adoption log 追加 rollback entry（`rollback_ref` 指向被回滚的 entry）

**门禁**：
- 无 backup_ref → 无法自动回滚，报告并停止
- 备份路径不可读 → 停止
- Dry-run 未确认 → 停止

## 协议引用

- 技能治理总协议：`protocol/skill-governance.md`
- 套件 manifest schema：`manifests/suite.yaml`
