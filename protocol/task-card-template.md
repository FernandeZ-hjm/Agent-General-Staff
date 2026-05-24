# Task Card Template

Cursor / Codex 使用此模板生成任务卡，交给 Claude Code 执行。

固定规则在 `docs/agent-workflow/agent-task-protocol.md`，不要重复粘贴进任务卡。

---

~~~~markdown
## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/cursor-skill-index.md

执行者：Claude Code

任务级别：Light / Medium / Heavy

任务：
<一句话任务描述>

背景：
<只写本次任务差异，不重复长期协议>

相关路径：
- `path_1`
- `path_2`

本次任务相关文件：
- `path_or_doc_1`
- `path_or_doc_2`

适用治理文档：
- 无 / `docs/agent-workflow/agent-toolchain-sync-governance.md`

目标：
1. goal_1
2. goal_2

非目标：
- non_goal_1
- non_goal_2

实施要求：
- requirement_1
- requirement_2

验证：
    <verification command>

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。

[skill: verify]
~~~~

---

## 使用说明

- **Cursor / Codex**：复制上面的任务卡模板，填写后交给 Claude Code。
- **Claude Code**：读取任务卡 + 引用的协议文件，执行并交付。
- 固定规则（安全、分级、验证、交付格式）在协议文件中，任务卡不再重复。
- 为了保持执行稳定性和缓存友好性，任务卡必须使用固定骨架：标题、字段顺序、基础措辞保持不变；只在固定槽位填写动态任务内容。
- Claude Code 启动后按固定顺序读取：
  1. 稳定协议文件：`AGENTS.md`、`CLAUDE.md`、`docs/agent-workflow/agent-task-protocol.md`、`docs/agent-workflow/task-routing.md`、`docs/agent-workflow/cursor-skill-index.md`。
  2. 本次任务相关文件：任务卡中列出的相关路径、治理文档、任务模块、待审查代码或数据说明。
  3. 动态命令输出：如 `git status --short`、验证命令、脚本检查结果，只记录在执行过程或交付报告的验证/状态部分，不放进“读取并遵守”清单。
- 只有跨仓库、外部 agent、或 Claude Code 无法访问本项目文件时，才使用完整自包含长 prompt。
- 任务级别按 `docs/agent-workflow/task-routing.md` 定义。
- 涉及本地 Agent 技能/插件同步、proposal、accept、patch、基线哈希时，必须引用 `docs/agent-workflow/agent-toolchain-sync-governance.md`，并读取 `docs/agent-workflow/task-cards/agent-toolchain-sync-task-card.md` 作为填槽参考；最终输出仍使用本文件的固定任务卡骨架。

## 与全局提示词生成器的关系

全局 `claude-execution-prompt-maker` 在本项目中应优先生成本任务卡格式，而不是完整自包含长 prompt。

### 硬约束：只允许两类

任务执行提示词有且仅有两类合法格式：

1. **本项目任务卡** — 本文件定义的固定骨架。
2. **全局 fallback 任务卡** — `templates/fallback-task-cards/{light,medium,heavy}.md`。

禁止自由 runbook、机器专用模板、阶段专用模板、或任何不属于以上两类的自造格式。

选择顺序：

1. 如果当前项目可访问本文件，使用本文件的固定任务卡骨架。
2. 如果任务属于特定治理域，读取 `docs/agent-workflow/task-cards/` 下的对应模块，把其约束填入固定槽位，不另起一套完整模板。
3. 只有跨仓库、外部 agent、或 Claude Code 无法访问本项目文件时，才回退到全局 fallback 模板。

## Agent Toolchain 治理任务补充

涉及本地 Agent 技能、插件、上游同步、proposal、accept、patch 或基线哈希时，固定任务卡按以下方式填槽：

- `相关路径`：列出 `tools/agent-toolchain/`、相关 `data/`、`reports/`、`proposals/`、`acceptance-patches/`、`agent-toolchain-acceptance-log.yaml`。
- `本次任务相关文件`：列出 `docs/agent-workflow/agent-toolchain-sync-governance.md`、`tools/agent-toolchain/README.md`，以及本次涉及的 report / proposal。
- `适用治理文档`：填写 `docs/agent-workflow/agent-toolchain-sync-governance.md`。
- `非目标`：明确不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`，不得运行 `lark-cli update`、`npx skills add/remove/update`，不得自动应用 patch。
- `实施要求`：说明只允许 read-only check、diff proposal、accept dry-run 或任务卡明确授权的 review-only patch。
- `验证`：优先使用 no-network check 和 `bash -n` 脚本语法检查。
- `交付`：必须说明是否生成 proposal / patch / report，是否触碰本地 skill/plugin 目录，仍需人工确认的事项。

## Heavy 任务补充

Heavy 任务可在任务卡中追加：

```markdown
实施流程：
1. 阅读与诊断 → 输出 root cause / 设计 / 计划 → 等待确认
2. 确认后执行
3. 验证与交付

基线保护：
- 不修改、删除、覆盖（列出受保护数据/目录）
```
