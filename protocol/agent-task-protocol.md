# Agent Task Protocol

Multi-Agent Engineering Kit 三方代理协作协议。定义 Cursor / Codex / Claude Code 如何交接任务。

**这是唯一 canonical 协议文件。不得为 Cursor、Codex、Claude Code 各自创建独立协议。**

## 角色

| 角色 | 职责 |
|------|------|
| **Cursor** | 需求澄清、风险分级、架构判断、任务拆解、生成任务卡、复核 Claude Code 结果 |
| **Codex** | 诊断、方案收敛、生成任务卡、复核 Claude Code 结果 |
| **Claude Code** | 读取协议和任务卡，具体执行、验证、输出交付报告 |

Cursor 和 Codex 都可以生成任务卡（使用 `task-card-template.md`），Claude Code 只执行任务卡。

## Claude Code 入口规则

Claude Code 收到任务卡后，必须先读取以下文件再开始工作：

1. `AGENTS.md`
2. `CLAUDE.md`
3. `docs/agent-workflow/agent-task-protocol.md`（本文件）
4. `docs/agent-workflow/task-routing.md`
5. `docs/agent-workflow/cursor-skill-index.md`
6. 当前任务涉及目录下的 `CLAUDE.md`（如 `scripts/CLAUDE.md`、`tests/CLAUDE.md`、`config/CLAUDE.md`）
7. 运行 `git status --short`，记录当前已有改动

## 任务分级

任务分级定义在 `docs/agent-workflow/task-routing.md`，此处只列出执行规则摘要：

### Light

- 单文件或小范围改动
- Claude Code 直接 read → execute → verify
- 不改数据、向量库、历史产物

### Medium

- 跨文件、配置、共享模块或行为边界变化
- Claude Code 先给简短 root cause 或 design note，再 execute → verify

### Heavy

- 涉及数据、向量库、历史产物、迁移、不可逆操作、架构调整
- Claude Code 先给 root cause / design / implementation plan / verification plan
- **等待确认后再改代码**

任务卡中声明的级别优先于 Claude Code 自行判断。如 Claude Code 发现实际风险高于任务卡标注的级别，必须停止并报告，不得自行降级执行。

## 安全规则

Claude Code 执行期间必须遵守：

- 不读取、打印、修改 secrets（`.env` 密钥项、Keychain、token、credential 文件）
- 不运行 destructive git 命令（`push --force`、`reset --hard`、`checkout .`、`restore .`、`clean -f`、`branch -D`），除非任务卡显式授权
- 不回滚用户未要求回滚的改动
- 不处理任务卡范围外的无关 dirty changes
- 不擅自删除、覆盖、迁移历史数据
- 不安装新依赖，除非先说明必要性并等待确认
- 如任务风险升级（遇到预期外的数据冲突、权限问题、不可逆操作），停止并报告

## Agent Toolchain 治理

本地 Agent 技能/插件同步机制纳入同一个三方协议，由 `docs/agent-workflow/agent-toolchain-sync-governance.md` 和 `tools/agent-toolchain/` 管理。

Cursor / Codex 处理技能、插件、上游同步、proposal、accept、patch、基线哈希相关任务时，必须：

- 先读取 `docs/agent-workflow/agent-toolchain-sync-governance.md`
- 使用 `docs/agent-workflow/task-card-template.md` 的固定骨架生成任务卡
- 需要 Agent Toolchain 专项约束时，读取 `docs/agent-workflow/task-cards/agent-toolchain-sync-task-card.md` 作为填槽模块
- 把所有写入限制、dry-run 要求、禁止自动更新命令写进任务卡
- 复核 Claude Code 的 delivery report、生成的 proposal / patch / log，再判断是否完成

Claude Code 执行这类任务时，除入口规则外还必须遵守：

- 默认只能运行 `tools/agent-toolchain/check-agent-toolchain-updates.sh --no-network`、`propose-agent-toolchain-update.sh`、`accept-agent-toolchain-update.sh --dry-run`
- 只有任务卡明确授权时，才允许生成 proposal、报告或 review-only patch
- 不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`
- 不得自动运行 `lark-cli update`、`npx skills add/remove/update` 或插件安装/更新命令
- 不得自动应用 `tools/agent-toolchain/acceptance-patches/` 下的 patch；真正接纳必须由人工确认

## 验证规则

Claude Code 完成前必须：

- 运行最窄相关验证命令
- 报告实际命令和结果
- 无法验证时必须说明原因
- 声称完成前必须有验证证据

## 交付报告

Claude Code 完成后必须输出以下格式的交付报告：

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

任务卡末尾可包含 `[skill: xxx]` 标记。Claude Code 收到标记后，按 `docs/agent-workflow/cursor-skill-index.md` 打开对应 SKILL.md 并按其指引执行。

常用标记：`[skill: tdd]`、`[skill: diagnose]`、`[skill: review]`、`[skill: verify]`、`[skill: commit]`。

## 任务卡模板

Cursor / Codex 使用 `docs/agent-workflow/task-card-template.md` 生成任务卡。

**规则进协议，差异进任务卡。** 固定规则写在本文件，单次任务差异写在任务卡中。

## 何时使用完整自包含 Prompt

只有以下情况才需要把完整协议重复粘贴进任务卡：

- 跨仓库执行（Claude Code 工作目录不是本仓库）
- 外部 agent 执行（没有本项目上下文）
- Claude Code 无法访问本仓库文件

其他情况一律使用任务卡 + 引用本协议。
