# Agent Task Protocol

Agent Governance Suite 三方代理协作协议。定义 Cursor / Codex / Claude Code 如何交接任务，并通过 runtime adapter 支持不同执行器。

**这是唯一 canonical 协议文件。不得为 Cursor、Codex、Claude Code 各自创建独立协议。**

## 完整生命周期

AGS 治理下的开发任务必须按以下阶段依次推进。不得跳过前置阶段直接进入分类或执行：

### 1. Ambient Preflight（环境预检）

Agent 收到开发相关请求时，自动执行：

- 检测项目身份（`ags project detect` 或手动读取 `AGENTS.md` / `CLAUDE.md` / `WORKSPACE.md`）
- 读取 context capsule（`context-capsule.md`）和 task memory（`task-memory.md`）
- 读取相关 protocol 文件
- 运行 `git status --short` 记录当前仓库状态
- 若任务目标与 capsule 的 `## 项目设计目的` 冲突，停止并报告

此阶段不涉及任何任务分级、gate、policy 或 runner。它只做只读上下文收集。

### 2. Solution Phase（方案形成）

- 理解用户请求：澄清歧义，诊断问题
- **Evolver / GEP recall（非简单任务必须）**：生成方案或任务卡之前，非简单任务
  （Medium / Heavy / 开发 / 架构 / 修复 / 重构 / 发布 / 治理规则调整）必须先执行
  Evolver/GEP recall。recall 状态（`status/search/fetch`）必须显式记录在方案文本
  中。recall 结果（召回路径、输入信号、命中信号、参考 Gene/Capsule、采纳、拒绝、
  影响、置信度/限制）必须进入方案。Evolver 不可用时必须明说原因并向用户确认继续。
  Evolver recall 不改变 AGS 生命周期门禁——方案确认、任务卡指令、分级路由仍按本
  协议执行。
- 形成具体方案或实现路径
- 评估影响范围、风险和替代方案
- 向用户呈现方案，等待确认

此阶段仍不涉及 Light / Medium / Heavy 分级。方案只是方案，不是可执行任务卡。

### 3. Execution Contract Phase（执行契约）

- 用户确认方案后，方案正式化为 execution contract
- Execution contract 是任务卡的输入来源
- **但此时仍不自动进入 Routing Phase**

**硬规则：用户的原始自然语言请求 ≠ 可执行任务卡。** 不得把初始聊天消息直接当作 Light / Medium / Heavy 分级的依据。只有经过 preflight → solution → confirmation 形成的 execution contract 才能进入 Routing Phase。

### 3.5. Task-Card Instruction Gate（任务卡指令门槛）

**这是 execution contract 与 routing 之间的硬门禁。**

- 用户确认方案（"方案 OK"）只结束 Solution Phase。
- 必须等用户进一步明确发出任务卡指令（"生成任务卡"、"按这个方案出任务卡"、"交给 Claude Code 执行"、"帮我写个任务卡拉去执行"等），才允许进入 Routing Phase 并生成可执行任务卡。
- 未收到任务卡指令前，`ags task compile` 在缺少 `--task-card-requested` 参数时 **必须拒绝输出可执行任务卡**，报告 `executable_allowed=false`、`block_reason=task_card_not_requested`。
- Codex / Cursor 只有在用户发出任务卡指令后，才能调用 `ags task compile --task-card-requested` 生成 canonical task card。
- Claude Code 只消费已形成的任务卡，不得从原始需求或单纯"方案 OK"自行分级和生成任务卡。

**三段门槛：方案 OK → 任务卡指令 → 任务分级路由。** 缺少中间的任务卡指令，不得进入路由。

### 4. Routing Phase（任务分级路由）

- 基于 execution contract（不是原始用户请求）进行 Light / Medium / Heavy 分级
- 按 `protocol/task-routing.md` 选择最小可行工作流
- 生成任务卡并填入 `任务级别：` 字段

### 5. Gate / Execution / Receipt Phase（门禁 / 执行 / 收据）

- 任务卡通过 validator（hard gate）
- 通过 execution-policy resolver（soft resolution）
- Executor 按 resolved policy 执行
- 完成后输出 delivery report 和 receipt

## 角色

| 角色 | 职责 |
|------|------|
| **Cursor** | Ambient preflight、需求澄清、方案形成、用户确认、执行契约化、任务分级路由、生成任务卡、按 `cursor` 执行 IDE 任务、复核结果 |
| **Codex** | Ambient preflight、诊断、方案收敛、用户确认、执行契约化、任务分级路由、生成任务卡、按 `codex-local` 执行本地任务、复核结果 |
| **Claude Code** | 按 `claude-code` 任务卡执行、验证、输出交付报告。Claude Code 只消费已形成的任务卡，不从原始需求提前分级 |

Cursor 和 Codex 都可以生成任务卡（使用 `task-card-template.md`）。任务卡通过 `Executor` 和 `Runtime adapter` 声明实际执行者。

**关键分工：** Codex / Cursor 负责 preflight → solution → contract → routing（生命周期阶段 1-4）；Claude Code 只消费阶段 4 产出的任务卡（生命周期阶段 5）。Claude Code 不得从原始用户自然语言请求自行推断任务级别。

## Executor 入口规则

For substantive planning tasks, Codex may run evolution recall after reading
project memory and before producing a plan/task card.
Evolution recall output is advisory evidence only.
It cannot change task level, permission mode, review gate, or verification gate.

Executor 收到任务卡后，必须先读取以下文件再开始工作：

1. `AGENTS.md`
2. `CLAUDE.md`
3. `protocol/agent-task-protocol.md`（本文件）
4. `protocol/task-routing.md`
5. `protocol/runtime-adapters.md`
6. `protocol/cursor-skill-index.md`
7. 当前任务涉及目录下的 `CLAUDE.md`（如 `scripts/CLAUDE.md`、`tests/CLAUDE.md`、`config/CLAUDE.md`）
8. 运行 `git status --short`，记录当前已有改动

本私有主库的 canonical 协议入口是 `protocol/`。如果任务卡或复制材料仍
引用旧 `docs/agent-workflow/...` 路径，Executor 必须映射到同名
`protocol/...` 文件并报告文档漂移；不得自行创建 `docs/agent-workflow/`
目录来满足旧引用。

## Runtime adapter

任务卡必须显式声明：

- `Executor`
- `Runtime adapter`
- `Execution surface`
- `Permission mode`
- `Parallelism`
- `Review gate`
- `Verification gate`

这些字段按 `protocol/runtime-adapters.md` 解释。未指定执行器时，按项目 operating protocol 执行：Cursor / Codex 负责 framing 与复核，Claude Code 可执行边界清晰的实现任务。

**重要：** Runtime adapter 字段（Permission mode、Parallelism、Execution surface、launch args）只在任务卡已形成后生效。Preflight 和 Solution phase 不属于 runner 执行范围，不受这些字段约束。

## 任务分级

任务分级定义在 `protocol/task-routing.md`，此处只列出执行规则摘要。

**前置条件：** 任务分级发生在 execution contract 形成之后，不是原始用户请求之后。分级依据是已确认的方案内容，不是用户的第一句自然语言消息。

### Light

- 单文件或小范围改动
- Executor 直接 read → execute → verify
- 不改数据、向量库、历史产物

### Medium

- 跨文件、配置、共享模块或行为边界变化
- Executor 先给简短 root cause 或 design note，再 execute → verify

### Heavy

- 涉及数据、向量库、历史产物、迁移、不可逆操作、架构调整
- Executor 先给 root cause / design / implementation plan / verification plan
- **等待确认后再改代码**
- 默认 `Permission mode: plan-only`
- 只有当前任务获得明确人工确认后，才可进入 `edit-with-confirmation` 或 `execute-and-verify`

任务卡中声明的级别优先于 Executor 自行判断。如 Executor 发现实际风险高于任务卡标注的级别，必须停止并报告，不得自行降级执行。

## Resume / 压缩恢复保护

遇到"继续"、上下文压缩恢复、task-notification 接续或后台任务恢复时：

- 如果任务级别是 Heavy，Executor 必须重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 如果当前上下文没有明确的人工批准执行 mutation，Executor 必须停在 plan / confirmation gate。
- 不得把"继续"、恢复通知、上一轮计划或压缩摘要理解为自动批准 Heavy 写入。
- 如果无法确认当前任务卡、目标仓库或批准状态，停止并报告，不得继续执行。

## 安全规则

Executor 执行期间必须遵守：

- 不读取、打印、修改 secrets（`.env` 密钥项、Keychain、token、credential 文件）
- 不运行 destructive git 命令（`push --force`、`reset --hard`、`checkout .`、`restore .`、`clean -f`、`branch -D`），除非任务卡显式授权
- 不回滚用户未要求回滚的改动
- 不处理任务卡范围外的无关 dirty changes
- 不擅自删除、覆盖、迁移历史数据
- 不安装新依赖，除非先说明必要性并等待确认
- 如任务风险升级（遇到预期外的数据冲突、权限问题、不可逆操作），停止并报告

## Skill Governance 治理

本地 Agent 技能同步机制纳入同一个三方协议。技能治理的 canonical 协议文件为
`protocol/skill-governance.md`（总协议）和 `governance/skill-sync.md`（同步阶段边界）。
套件级脚本入口（`scripts/govern-new-skills.sh`）将在 Phase 2 实现。

Cursor / Codex 处理技能新增、下载、拖拽导入、proposal、adoption log 或 ignore list 相关任务时，必须：

- 使用 `protocol/task-card-template.md` 的固定骨架生成任务卡
- 把所有写入限制、dry-run 要求、禁止自动更新命令写进任务卡
- 默认先运行 scan / dry-run，再由人工确认 adopt / ignore
- 复核 Claude Code 的 delivery report、生成的 proposal / adoption log，再判断是否完成
- 遵守 `protocol/skill-governance.md` 的写入规则硬门禁

Executor 执行这类任务时，除入口规则外还必须遵守：

- 默认只能执行只读 scan/check 操作或 dry-run 命令
- 只有任务卡明确授权时，才允许写入 `governance/skill-adoption-log.yaml`、`governance/skill-ignore-list.yaml`、`manifests/suite.yaml` 或 `governance/backups/`
- 不得直接写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`
- 不得自动运行 `lark-cli update`、`npx skills add/remove/update` 或插件安装/更新命令
- 不得自动安装未审查技能；真正接纳必须由人工确认
- 不得接管外部官方 CLI 或项目自管输出层技能；`notebooklm`、Hermes 输出层技能、TempoFlow 输出层业务契约只能引用，不能通过套件 adopt / update / 打包
- Hermes / TempoFlow 输出层产物（如 `notebooklm_task_card`、`local_context_pack`、`fairness_check_questions`）是业务运行时契约，不得改写为开发套件任务卡或技能治理对象

## 验证规则

Executor 完成前必须：

- 运行最窄相关验证命令
- 报告实际命令和结果
- 无法验证时必须说明原因
- 声称完成前必须有验证证据

## Review Gate 规则

Stop review hook 已废弃，不再是最终执行点，也不是自动阻塞依据。Review 由任务卡显式声明、由人工显式触发，并按任务级别执行：

| 任务级别 | Review gate |
|---|---|
| Light | 完成验证后运行 `caveman-review` 或等价轻量 diff review；如发现风险高于 Light，升级 Medium。 |
| Medium | Codex 最终 Review gate；Executor 完成验证后将任务状态标为"部分完成 / 等待 Codex review"，由 Codex 审查通过后再放行。 |
| Heavy | 先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为"部分完成 / 等待人工 adversarial review"，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。 |

Executor 完成交付前必须报告 review gate 状态；Medium / Heavy 在对应人工 review 完成前只能标为"部分完成"。Light 若在 `caveman-review` 中发现跨文件协议、权限、hook、数据写入、路径迁移或生成物同步风险，必须升级为 Medium 并等待 Codex review。

如果 Medium / Heavy 的实现和验证已经完成，但对应 review gate 尚未完成，
交付报告中的 `## 任务状态` 必须写"部分完成 / 等待 Codex review"或
"部分完成 / 等待人工 adversarial review"，不得写"完成"。只有验证通过且
对应 review gate 通过后，才能把治理状态写为"完成"。

## Runtime Hook Policy

安装套件后，Claude Code 和 Codex 只保留运行时必需 hook：

- Claude Code：`UserPromptSubmit` 同步 skill alias；`UserPromptSubmit`
  读取本地记忆入口；`PreToolUse(Bash)` 使用 `rtk hook claude`；`Stop`
  在检测到粘贴任务卡和交付报告时自动归档到 `task-memory.md` / `task-archive/`。
- Codex：`UserPromptSubmit` 同步 skill alias。

Stop review hook 已废弃，不再是最终执行点。需要 review 时，由任务卡的 Review gate 或人工显式触发对应 review 流程。

## 交付报告

Executor 完成后必须输出以下格式的交付报告：

```markdown
# 任务交付报告

## 任务状态
完成 / 部分完成 / 未完成

一句话结论：

## 改动内容
修改文件：
- `path`: 改动摘要

新增文件 / 输出物：
- `path`: 用途

删除文件：
- 无 / `path`: 原因

## 验证结果
已运行：
- `command` → 结果

未验证内容：
- 无 / 说明原因

## 风险提示
- 风险项

## 下一步建议
- 建议项
```

## 技能标记

任务卡末尾可包含 `[skill: xxx]` 标记。Executor 收到标记后，按 `protocol/cursor-skill-index.md` 打开对应 SKILL.md 并按其指引执行。

常用标记：`[skill: tdd]`、`[skill: diagnose]`、`[skill: review]`、`[skill: verify]`、`[skill: commit]`。

## 任务卡模板

Cursor / Codex 使用 `protocol/task-card-template.md` 生成任务卡。任务卡的输入必须是已确认的方案或 execution contract，不能是原始用户自然语言请求。

**规则进协议，差异进任务卡。** 固定规则写在本文件，单次任务差异写在任务卡中。

### 任务卡形态边界

任务卡形态只有两种：

1. **Project task card**：项目内 canonical 任务卡。在本私有主库中即
   `protocol/task-card-template.md` 定义的固定骨架，默认使用此形态。
2. **Global fallback task card**：全局 fallback 任务卡。仅在项目没有可访问
   task-card protocol、跨仓库、外部 agent 无法访问项目文件等场景使用。

“完整”“压缩”“可粘贴”“可复制”“compact”“full”都不是任务卡形态，
只能表达前台展示偏好。它们不得改变任务卡骨架、标题、槽位顺序或从
Project task card / Global fallback task card 之外创造第三种格式。

自由 runbook、临时 prompt、阶段性执行简报、header-only runtime block、
target-first task brief、文档式任务说明都不得称为任务卡。需要执行时，
必须重新编译为 Project task card 或 Global fallback task card。

对话前台输出任务卡时，默认使用普通 Markdown，让客户端自然换行；只有
用户明确要求单个 literal copy block、文件 artifact，或任务卡内含嵌套
代码块且必须整体复制时，才用外层 fenced block。

## 何时使用 Global fallback / 自包含 Prompt

只有以下情况才需要把完整协议重复粘贴进 Global fallback task card 或
自包含 prompt：

- 跨仓库执行（Executor 工作目录不是本仓库）
- 外部 agent 执行（没有本项目上下文）
- Executor 无法访问本仓库文件

其他情况一律使用 Project task card + 引用本协议。
