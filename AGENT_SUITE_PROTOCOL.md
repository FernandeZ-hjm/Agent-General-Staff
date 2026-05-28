# Agent Suite Protocol

本文件定义 Agent 开发套件的角色分工、任务流转和交付标准。所有接入此套件的项目和 Agent 运行时共用同一份 canonical 协议。

## 模式门禁（协议级强制规则）

所有三方 Agent（Cursor、Codex、Claude Code）在进入任何工具模式或调用专用工具之前，必须先扫描已安装技能库，存在匹配技能时必须先调用该技能，再进入目标模式。此项为协议级硬约束，任何 Agent 不得绕过。

违反此规则的典型行为：直接调用 EnterPlanMode 而不先触发 auto-brainstorm、声称完成而不先触发 auto-verify、遇到错误直接改代码而不先触发 auto-debug。

## 角色

| 角色 | 职责 |
|------|------|
| **Codex** | 诊断、方案收敛、生成任务卡、按 `codex-local` 执行本地任务、复核交付结果 |
| **Cursor** | 需求澄清、风险分级、架构判断、任务拆解、生成任务卡、按 `cursor` 执行 IDE 任务、复核交付结果 |
| **Claude Code** | 按 `claude-code` 任务卡执行、运行验证、输出交付报告 |

任务卡通过 `Executor` 和 `Runtime adapter` 声明实际执行者。Codex 和 Cursor 默认负责 framing 与复核；当任务卡明确设置 `Executor: Codex` 或 `Executor: Cursor` 时，也可以作为执行者。

## 任务卡路由规则

### Task Card Compiler v2

用户需求可以是自然语言，但交给执行器的输出必须编译为固定任务卡骨架。
`config/agent-project-profile.yaml` 是项目画像入口，只能用于填充任务卡动态 slot，
不得替代任务卡，也不得生成第三种模板。

编译规则：

- 固定 cache anchor、标题、字段顺序和基础措辞不变。
- 项目画像、仓库事实和用户请求只进入 `项目画像`、`背景`、`相关路径`、
  `本次任务相关文件`、`适用治理文档`、`实施要求`、`验证` 等固定槽位。
- `任务存档` 只引用本机记忆目录中的自动任务记忆入口，不复制长日志。
- 动态命令输出不得加入“读取并遵守”清单。
- 无法从项目画像或仓库事实确认的内容，写成 stop condition，不编造默认值。

### Context Memory

跨对话记忆通过本地 `context-capsule.md` 和 `task-memory.md` 提供连续性，但不得替代任务卡。
任务卡只能在固定 `记忆胶囊` slot 中引用 capsule 路径，不复制长记忆。

记忆规则：

- 默认存放在 `$HOME/.agents/memory/projects/<project-slug>/`。
- `context-capsule.md` 是人工维护的项目宪章，必须包含
  `## 项目设计目的`。runner / hook / capture 不得覆盖，自动总结不得改写；
  只能由用户明确要求时手动修改。
- 每次任务开始前必须读取 `context-capsule.md`；如果任务目标和项目设计目的冲突，
  Agent 必须停止并报告。
- 自动记忆刷新 `task-memory.md`，并把完整收据写入 `task-archive/`；不得覆盖
  `context-capsule.md`。
- 完整历史任务证据存放在同目录 `task-archive/`。
- 记忆可以提供项目事实、近期决策、验证习惯和收据引用。
- 记忆不是 Heavy 写入批准，也不能覆盖当前任务卡、用户请求或 live repo 证据。
- 自动学习只能生成 proposal 或 session memory，不得直接改规则、技能或任务卡骨架。

### Doctor / Orchestrator Policy

成熟度工具必须默认只读：

- `suite-doctor.sh` 只检查套件健康和漂移，不执行 repair、apply、pull、push。
- `security-doctor.sh` 只报告 hooks、危险命令、疑似 secrets 和边界风险，不自动清理。
- `run-task-card.sh --auto` 只能根据任务卡声明选择执行层；不得升级 `Permission mode`，
  不得绕过 Review gate 或 Verification gate。

### 硬约束：只允许两类任务卡

任务执行提示词的合法输出格式有且仅有以下两类：

1. **项目任务卡** — 项目内存在 `docs/agent-workflow/task-card-template.md` 时使用固定骨架。
2. **全局 fallback 任务卡** — 项目无任务卡协议、跨仓库、外部 agent 执行、或 Executor 不可访问项目文件时使用 `templates/fallback-task-cards/{light,medium,heavy}.md`。

以下格式为**禁止**：
- 自由 runbook / checklist
- 自造提示词格式
- 机器专用完整模板（MacBook、Mac mini、新机器等）
- 阶段专用模板（"Step 1 专用"、"MacBook Step 2" 等）
- Cursor 专用 / Codex 专用 / Claude Code 专用完整协议

### 路由规则

1. 如果当前项目有 `docs/agent-workflow/task-card-template.md`，使用项目任务卡骨架。
2. 如果项目无任务卡协议、跨仓库执行、或 Executor 不可访问项目文件，回退到本套件的全局 fallback 模板。
3. 跨机器 bootstrap、MacBook/Mac mini/新机器安装场景统一使用全局 fallback 模板；机器差异用变量表达（`$SUITE_REPO`、`$TARGET_HOME`、`$HOME`），不得新建专用完整模板。
4. 任务卡中声明的级别优先于 Executor 自行判断。如果 Executor 发现实际风险高于标注级别，必须停止并报告。

### Runtime adapter 规则

任务卡必须包含：

- `Executor`
- `Runtime adapter`
- `Execution surface`
- `Permission mode`
- `Parallelism`
- `Review gate`
- `Verification gate`

字段定义和运行时映射统一由 `protocol/runtime-adapters.md` 管理。不得为不同工具复制完整协议；工具差异只能写入 runtime adapter 或任务卡固定槽位。

## 任务分级

| 级别 | 执行模式 | 触发条件 |
|------|---------|---------|
| Light | 直接 read → execute → verify | 单文件、小范围、不改数据 |
| Medium | 先给简短 design note → execute → verify | 跨文件、配置、共享模块 |
| Heavy | 先给 root cause / 设计 / 计划，等待确认后再执行 | 数据、向量库、历史产物、迁移、架构调整、不可逆操作 |

Heavy 默认 `Permission mode: plan-only`。只有当前任务获得明确人工确认后，才可进入
`edit-with-confirmation` 或 `execute-and-verify`；运行时能力、上一轮上下文或“继续”本身
都不能把 Heavy 任务自动升级为可写入。

## Resume / 压缩恢复保护

遇到“继续”、上下文压缩恢复、task-notification 接续或后台任务恢复时：

- 如果任务级别是 Heavy，Executor 必须重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 如果当前上下文没有明确的人工批准执行 mutation，Executor 必须停在 plan / confirmation gate。
- 不得把“继续”、恢复通知、上一轮计划或压缩摘要理解为自动批准 Heavy 写入。
- 如果无法确认当前任务卡、目标仓库或批准状态，停止并报告，不得继续执行。

## Review Gate

Stop review hook 已废弃，不再是最终执行点，也不是自动阻塞依据。任务卡必须显式声明 Review gate；唯一 Light / Medium / Heavy 规则表维护在 `protocol/agent-task-protocol.md`，其他模板和 runtime 文档只引用该规则，不重复手写。

## 安全规则

所有 Agent 必须遵守：

- 不读取、打印、修改 secrets（.env 密钥项、Keychain、token、credential 文件）
- 不运行 destructive git 命令，除非任务卡显式授权
- 不回滚用户未要求回滚的改动
- 不处理任务卡范围外的不相关改动
- 不擅自删除、覆盖、迁移历史数据
- 不安装新依赖，除非先说明必要性并等待确认

## Review Target 规则

Claude Code / Codex hook 不得假设启动目录 `cwd` 就是实际修改仓库。通过 SSH、远程控制、挂载目录或跨仓库任务执行时，任务卡必须让执行器在启动目录写入 `.claude/review_targets.json`，声明所有实际会被读写的 git 仓库：

```json
{
  "task_level": "Light / Medium / Heavy",
  "targets": [
    {
      "name": "<repo-name>",
      "path": "<absolute path to actual repo>",
      "level": "Light / Medium / Heavy"
    }
  ]
}
```

`review_targets.json` 是单次任务状态，执行器每次任务都必须重写。需要显式 review 时，审查范围必须基于这些实际目标仓库，而不是盲目使用启动 `cwd`。如果任务涉及 cwd 外路径但没有声明 review target，执行器必须停止并报告。

## Runtime Hook Policy

套件不再安装 Stop review hook。运行时 hook 收口为：

- Claude Code：`UserPromptSubmit` 执行 `python3 ~/.claude/sync-skill-aliases.py`
  和 `bash ~/.agents/scripts/memory-start-context.sh`，`PreToolUse(Bash)` 执行
  `rtk hook claude`。
- Codex：`UserPromptSubmit` 执行 `python3 ~/.claude/sync-skill-aliases.py`
  和 `bash ~/.agents/scripts/memory-start-context.sh`。

公开默认版不安装 Stop 维护 hook。若使用者需要在自己的项目中增加 Stop hook，
必须通过本地配置显式生成，并把触发范围、允许写入路径和 push 策略写清楚。

`memory-start-context.sh` 只读 `context-capsule.md` 和 `task-memory.md`，不写文件。
任务结束的自动沉淀由 runner 的 `--memory` capture 完成：复制完整收据到
`task-archive/`，刷新 `task-memory.md`，不覆盖 `context-capsule.md`。

旧的 `leveled-review-gate.mjs`、`review-baseline-snapshot.mjs`、`codex-stop-review-adapter.mjs`
视为历史残留。显式 review 由任务卡 Review gate 或人工门禁触发，不再由 Stop hook 自动弹出。

可选硬阻塞属于第二阶段，不混入当前默认协议。若未来启用，只能通过显式选项打开，例如
`scripts/configure-review-hooks.mjs --enable-stop-review-gate`，或由 suite-owned runner
在 Heavy resume 时检查 approval marker。不得依赖 `.claude/task_level` 作为当前真实 Codex
plugin 的阻塞依据。

## 交付报告

Executor 完成后必须输出以下格式：

```markdown
# 任务交付报告

## 任务状态
完成 / 部分完成 / 未完成

一句话结论：

## 改动内容
修改文件：
- path: 改动摘要

新增文件 / 输出物：
- path: 用途

删除文件：
- 无 / path: 原因

## 验证结果
已运行：
- command → 结果

未验证内容：
- 无 / 说明原因

## 风险提示
- 风险项

## 下一步建议
- 建议项
```

## 技能标记

任务卡末尾可包含 `[skill: xxx]` 标记。收到标记后按 `protocol/cursor-skill-index.md` 执行。

常用标记：`[skill: tdd]`、`[skill: diagnose]`、`[skill: review]`、`[skill: verify]`、`[skill: commit]`、`[skill: zoom-out]`。

## 动态命令输出规则

动态命令输出（如 `git status`、验证输出、脚本检查结果）不得加入"读取并遵守"清单。只放在交付报告的验证/状态部分。

## Skill Governance 治理

本地 Agent 技能同步统一由 `scripts/govern-new-skills.sh`、`governance/skill-adoption-log.yaml` 和 `governance/skill-ignore-list.yaml` 管理。任何更新必须先 scan / dry-run，再人工 diff 决策，禁止自动覆盖。

## 规则进协议，差异进任务卡

固定规则写在本文件、`protocol/runtime-adapters.md` 和治理文档中。单次任务差异写在任务卡中。不为 Cursor、Codex、Claude Code 各自维护独立完整协议。
