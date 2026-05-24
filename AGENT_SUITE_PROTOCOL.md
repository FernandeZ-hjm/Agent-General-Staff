# Agent Suite Protocol

本文件定义 Agent 开发套件的角色分工、任务流转和交付标准。所有接入此套件的项目和 Agent 运行时共用同一份 canonical 协议。

## 模式门禁（协议级强制规则）

所有三方 Agent（Cursor、Codex、Claude Code）在进入任何工具模式或调用专用工具之前，必须先扫描已安装技能库，存在匹配技能时必须先调用该技能，再进入目标模式。此项为协议级硬约束，任何 Agent 不得绕过。

违反此规则的典型行为：直接调用 EnterPlanMode 而不先触发 auto-brainstorm、声称完成而不先触发 auto-verify、遇到错误直接改代码而不先触发 auto-debug。

## 角色

| 角色 | 职责 |
|------|------|
| **Codex** | 诊断、方案收敛、生成任务卡、复核 Claude Code 交付结果 |
| **Cursor** | 需求澄清、风险分级、架构判断、任务拆解、生成任务卡、复核 Claude Code 交付结果 |
| **Claude Code** | 按任务卡执行、运行验证、输出交付报告 |

Codex 和 Cursor 都可以生成任务卡，Claude Code 只执行任务卡。

## 任务卡路由规则

### 硬约束：只允许两类任务卡

任务执行提示词的合法输出格式有且仅有以下两类：

1. **项目任务卡** — 项目内存在 `docs/agent-workflow/task-card-template.md` 时使用固定骨架。
2. **全局 fallback 任务卡** — 项目无任务卡协议、跨仓库、外部 agent 执行、或 Claude Code 不可访问项目文件时使用 `templates/fallback-task-cards/{light,medium,heavy}.md`。

以下格式为**禁止**：
- 自由 runbook / checklist
- 自造提示词格式
- 机器专用完整模板（MacBook、Mac mini、新机器等）
- 阶段专用模板（"Step 1 专用"、"MacBook Step 2" 等）
- Cursor 专用 / Codex 专用 / Claude Code 专用协议

### 路由规则

1. 如果当前项目有 `docs/agent-workflow/task-card-template.md`，使用项目任务卡骨架。
2. 项目 `docs/agent-workflow/task-cards/*.md` 只能作为填槽模块（slot-filling modules），不能作为独立完整任务卡。
3. 如果项目无任务卡协议、跨仓库执行、或 Claude Code 不可访问项目文件，回退到本套件的全局 fallback 模板。
4. 跨机器 bootstrap、MacBook/Mac mini/新机器安装场景统一使用全局 fallback 模板；机器差异用变量表达（`$SUITE_REPO`、`$TARGET_HOME`、`$HOME`），不得新建专用完整模板。
5. 任务卡中声明的级别优先于 Claude Code 自行判断。如果 Claude Code 发现实际风险高于标注级别，必须停止并报告。

## 任务分级

| 级别 | 执行模式 | 触发条件 |
|------|---------|---------|
| Light | 直接 read → execute → verify | 单文件、小范围、不改数据 |
| Medium | 先给简短 design note → execute → verify | 跨文件、配置、共享模块 |
| Heavy | 先给 root cause / 设计 / 计划，等待确认后再执行 | 数据、向量库、历史产物、迁移、架构调整、不可逆操作 |

## 安全规则

所有 Agent 必须遵守：

- 不读取、打印、修改 secrets（.env 密钥项、Keychain、token、credential 文件）
- 不运行 destructive git 命令，除非任务卡显式授权
- 不回滚用户未要求回滚的改动
- 不处理任务卡范围外的不相关改动
- 不擅自删除、覆盖、迁移历史数据
- 不安装新依赖，除非先说明必要性并等待确认

## 交付报告

Claude Code 完成后必须输出以下格式：

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

## Agent Toolchain 治理

本地 Agent 技能/插件同步统一由 `governance/agent-toolchain-sync-governance.md` 和套件 scripts 管理。任何更新必须先 dry-run check，再人工 diff 决策，禁止自动覆盖。

## 规则进协议，差异进任务卡

固定规则写在本文件和治理文档中。单次任务差异写在任务卡中。不为 Cursor、Codex、Claude Code 各自维护独立协议。
