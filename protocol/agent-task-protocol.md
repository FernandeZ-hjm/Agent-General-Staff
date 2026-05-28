# Agent Task Protocol

Agent Governance Suite 三方代理协作协议。定义 Cursor / Codex / Claude Code 如何交接任务，并通过 runtime adapter 支持不同执行器。

**这是唯一 canonical 协议文件。不得为 Cursor、Codex、Claude Code 各自创建独立协议。**

## 角色

| 角色 | 职责 |
|------|------|
| **Cursor** | 需求澄清、风险分级、架构判断、任务拆解、生成任务卡、按 `cursor` 执行 IDE 任务、复核结果 |
| **Codex** | 诊断、方案收敛、生成任务卡、按 `codex-local` 执行本地任务、复核结果 |
| **Claude Code** | 按 `claude-code` 任务卡执行、验证、输出交付报告 |

Cursor 和 Codex 都可以生成任务卡（使用 `task-card-template.md`）。任务卡通过 `Executor` 和 `Runtime adapter` 声明实际执行者。

## Executor 入口规则

Executor 收到任务卡后，必须先读取以下文件再开始工作：

1. `AGENTS.md`
2. `CLAUDE.md`
3. `docs/agent-workflow/agent-task-protocol.md`（本文件）
4. `docs/agent-workflow/task-routing.md`
5. `docs/agent-workflow/runtime-adapters.md`
6. `docs/agent-workflow/cursor-skill-index.md`
7. 当前任务涉及目录下的 `CLAUDE.md`（如 `scripts/CLAUDE.md`、`tests/CLAUDE.md`、`config/CLAUDE.md`）
8. 运行 `git status --short`，记录当前已有改动

## Runtime adapter

任务卡必须显式声明：

- `Executor`
- `Runtime adapter`
- `Execution surface`
- `Permission mode`
- `Parallelism`
- `Review gate`
- `Verification gate`

这些字段按 `docs/agent-workflow/runtime-adapters.md` 解释。未指定执行器时，按项目 operating protocol 执行：Cursor / Codex 负责 framing 与复核，Claude Code 可执行边界清晰的实现任务。

## 任务分级

任务分级定义在 `docs/agent-workflow/task-routing.md`，此处只列出执行规则摘要：

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

遇到“继续”、上下文压缩恢复、task-notification 接续或后台任务恢复时：

- 如果任务级别是 Heavy，Executor 必须重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 如果当前上下文没有明确的人工批准执行 mutation，Executor 必须停在 plan / confirmation gate。
- 不得把“继续”、恢复通知、上一轮计划或压缩摘要理解为自动批准 Heavy 写入。
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

本地 Agent 技能同步机制纳入同一个三方协议。项目内如有专用治理文档，按项目任务卡引用；套件分发场景使用 `scripts/govern-new-skills.sh` 进行 scan / adopt / ignore / list。

Cursor / Codex 处理技能新增、下载、拖拽导入、proposal、adoption log 或 ignore list 相关任务时，必须：

- 使用 `docs/agent-workflow/task-card-template.md` 的固定骨架生成任务卡
- 把所有写入限制、dry-run 要求、禁止自动更新命令写进任务卡
- 默认先运行 scan / dry-run，再由人工确认 adopt / ignore
- 复核 Claude Code 的 delivery report、生成的 proposal / adoption log，再判断是否完成

Executor 执行这类任务时，除入口规则外还必须遵守：

- 默认只能运行 `scripts/govern-new-skills.sh scan`、`list`、或不写入的 dry-run 命令
- 只有任务卡明确授权时，才允许写入 `global-skills/`、`skill-packs/`、`proposals/`、`governance/skill-adoption-log.yaml` 或 `governance/skill-ignore-list.yaml`
- 不得直接写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`
- 不得自动运行 `lark-cli update`、`npx skills add/remove/update` 或插件安装/更新命令
- 不得自动安装未审查技能；真正接纳必须由人工确认

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
| Light | 完成前自查 diff；提交前建议运行 `caveman-review`。 |
| Medium | 人工 Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 review”，并提醒操作者手动运行 `/codex:review` 后再放行。 |
| Heavy | 先计划后执行；人工 Adversarial Review gate；Executor 完成验证后将任务状态标为“部分完成 / 等待人工 adversarial review”，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。 |

Executor 完成交付前必须报告 review gate 状态；Medium / Heavy 在人工 review 完成前只能标为“部分完成”，并在下一步建议中提醒操作者运行对应 review 命令。

## Runtime Hook Policy

安装套件后，Claude Code 和 Codex 只保留运行时必需 hook：

- Claude Code：`UserPromptSubmit` 同步 skill alias；`PreToolUse(Bash)` 使用 `rtk hook claude`。
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

任务卡末尾可包含 `[skill: xxx]` 标记。Executor 收到标记后，按 `docs/agent-workflow/cursor-skill-index.md` 打开对应 SKILL.md 并按其指引执行。

常用标记：`[skill: tdd]`、`[skill: diagnose]`、`[skill: review]`、`[skill: verify]`、`[skill: commit]`。

## 任务卡模板

Cursor / Codex 使用 `docs/agent-workflow/task-card-template.md` 生成任务卡。

**规则进协议，差异进任务卡。** 固定规则写在本文件，单次任务差异写在任务卡中。

## 何时使用完整自包含 Prompt

只有以下情况才需要把完整协议重复粘贴进任务卡：

- 跨仓库执行（Executor 工作目录不是本仓库）
- 外部 agent 执行（没有本项目上下文）
- Executor 无法访问本仓库文件

其他情况一律使用任务卡 + 引用本协议。
