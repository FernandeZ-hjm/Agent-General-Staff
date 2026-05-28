# Task Card Template

Cursor / Codex 使用此模板生成任务卡，交给指定 Executor 执行。

固定规则在 `docs/agent-workflow/agent-task-protocol.md` 和 `docs/agent-workflow/runtime-adapters.md`，不要重复粘贴进任务卡。

---

~~~~markdown
## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/runtime-adapters.md
- docs/agent-workflow/project-profile.md
- docs/agent-workflow/context-memory.md
- docs/agent-workflow/cursor-skill-index.md

Executor: Codex / Claude Code / Cursor / Human / Other

Runtime adapter: codex-local / claude-code / cursor / generic

Execution surface: local-workspace / cli / ide / web / remote-control / background-agent

Permission mode: read-only / plan-only / edit-with-confirmation / execute-and-verify / autonomous-low-risk

Parallelism: none / subagent / worktree / multi-session / agent-team

任务级别：Light / Medium / Heavy

Heavy 写入批准规则按 docs/agent-workflow/agent-task-protocol.md 执行；“继续”、上下文压缩恢复或 task-notification 接续不算 Heavy 写入批准。

Review gate:
- 按 docs/agent-workflow/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

任务：
<一句话任务描述>

背景：
<只写本次任务差异，不重复长期协议>

项目画像：
- 无 / `config/agent-project-profile.yaml`

记忆胶囊：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`
- 若存在，同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

相关路径：
- `path_1`
- `path_2`

本次任务相关文件：
- `path_or_doc_1`
- `path_or_doc_2`

适用治理文档：
- 无 / `<project-specific-governance-doc>`

目标：
1. goal_1
2. goal_2

非目标：
- non_goal_1
- non_goal_2

实施要求：
- requirement_1
- requirement_2
- 如果 Claude Code 启动目录不是实际修改的仓库根目录，或任务会跨仓库修改，开始执行前必须在启动目录写入 `.claude/review_targets.json`：
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
- `review_targets.json` 是单次任务状态，开始执行时必须重写，并覆盖所有实际会被读写的 git 仓库；未能确认实际目标仓库时停止并报告，不要继续执行。

验证：
Verification gate:
- commands:
  - <verification command>
- expected evidence:
  - <test result / diff summary / report path>
- stop condition:
  - <when to pause and report instead of continuing>

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。

[skill: verify]
~~~~

---

## 使用说明

- **Cursor / Codex**：复制上面的任务卡模板，填写 Executor 和 Runtime adapter 后交给指定执行器。对话前台默认可输出压缩执行卡，但必须使用固定压缩骨架，以 `AGENT_SUITE_COMPACT_TASK_CARD_V1` 作为首行 cache anchor，保持标题、字段顺序和基础措辞稳定，只填动态 slot；固定规则通过协议路径引用，不把整套骨架全文展开给用户确认。
- **Executor**：读取任务卡 + 引用的协议文件，执行并交付。
- 固定规则（安全、分级、runtime adapter、Review gate、验证、交付格式）在协议文件中，任务卡不再重复。
- 为了保持执行稳定性和缓存友好性，任务卡必须使用固定骨架：标题、字段顺序、基础措辞保持不变；只在固定槽位填写动态任务内容。
- `项目画像` 是稳定上下文入口。项目存在 `config/agent-project-profile.yaml` 时只引用路径或提取必要短事实，不把整份画像粘进任务卡；项目无画像时填写 `无`。
- `记忆胶囊` 是人工项目宪章入口。存在本地 capsule 时只引用路径，不粘贴长记忆；没有 capsule 时填写 `无`。Executor 开始任务前必须读取 capsule；如同目录存在 `task-memory.md`，也必须读取。若任务目标与 capsule 的 `## 项目设计目的` 冲突，停止并报告。
- `任务存档` 是自动任务记忆入口。存在本地 `task-memory.md` 时填写该路径；没有任务记忆时填写 `无`。使用 runner 执行并启用 memory capture 后，`task-memory.md` 会自动刷新，完整证据保存在 `$HOME/.agents/memory/projects/<project-slug>/task-archive/`。
- 默认不生成 `.md` 文件产物；只有用户明确要求落盘或需要 runner 直接消费文件时，才创建任务卡文件。
- 对话交付给 Claude Code 的任务卡必须是一个连续 fenced `markdown` block，不按环节拆成多个片段，便于用户一次复制。默认输出可执行的压缩任务卡；只有用户明确要求“完整骨架”或 runner/file artifact 需要时，才展开完整模板。
- 压缩执行卡固定字段顺序为：`AGENT_SUITE_COMPACT_TASK_CARD_V1`、`遵循固定字段顺序，只填动态 slot。`、`路径`、`Executor`、`Runtime adapter`、`Execution surface`、`Permission mode`、`Parallelism`、`任务级别`、`读取`、`任务`、`目标`、`非目标`、`关键路径`、`验证`、`停止条件`、`交付`、技能标记。
- 任务卡正文包含内嵌代码块时，外层必须使用 `~~~~markdown` / `~~~~`，不得使用三反引号 ` ```markdown `；本模板包含 `.claude/review_targets.json` 的 ` ```json ` 示例，使用三反引号外层会被内部代码块提前截断。
- 远程控制、SSH、挂载目录、跨仓库任务中，`cwd` 不一定等于实际修改仓库。任务卡必须显式要求 Executor 为本次任务重写 `.claude/review_targets.json`，让显式 review 的审查范围对准实际目标仓库。
- Executor 启动后按固定顺序读取：
  1. 稳定协议文件：`AGENTS.md`、`CLAUDE.md`、`docs/agent-workflow/agent-task-protocol.md`、`docs/agent-workflow/task-routing.md`、`docs/agent-workflow/runtime-adapters.md`、`docs/agent-workflow/cursor-skill-index.md`。
  2. 稳定上下文文件：任务卡声明的 `项目画像`、`记忆胶囊`、同目录 `task-memory.md` 和 `任务存档`，如存在。
  3. 本次任务相关文件：任务卡中列出的相关路径、治理文档、待审查代码或数据说明。
  4. 动态命令输出：如 `git status --short`、验证命令、脚本检查结果，只记录在执行过程或交付报告的验证/状态部分，不放进“读取并遵守”清单。
- 只有跨仓库、外部 agent、或 Executor 无法访问本项目文件时，才使用完整自包含长 prompt。
- 任务级别按 `docs/agent-workflow/task-routing.md` 定义。
- Executor、Runtime adapter、Execution surface、Permission mode、Parallelism、Verification gate 按 `docs/agent-workflow/runtime-adapters.md` 定义；Review gate 的唯一规则表在 `docs/agent-workflow/agent-task-protocol.md`。
- 需要让 runner 自动选择执行层时，可以使用 `scripts/run-task-card.sh <task-card> --auto`；auto mode 不会提高任务卡声明的权限。
- 涉及本地 Agent 技能同步、proposal、adoption log 或 ignore list 时，必须引用项目内对应治理文档；如无项目治理文档，使用套件级 `scripts/govern-new-skills.sh` 流程。最终输出仍使用本文件的固定任务卡骨架。

## 与全局提示词生成器的关系

全局 `prompt-maker` 在本项目中应优先生成本任务卡格式，而不是完整自包含长 prompt。

### 硬约束：只允许两类

任务执行提示词有且仅有两类合法格式：

1. **本项目任务卡** — 本文件定义的固定骨架。
2. **全局 fallback 任务卡** — `templates/fallback-task-cards/{light,medium,heavy}.md`。

禁止自由 runbook、机器专用模板、阶段专用模板、或任何不属于以上两类的自造格式。

选择顺序：

1. 如果当前项目可访问本文件，使用本文件的固定任务卡骨架。
2. 只有跨仓库、外部 agent、或 Executor 无法访问本项目文件时，才回退到全局 fallback 模板。

## Skill Governance 治理任务补充

涉及本地 Agent 技能、下载/拖拽导入、proposal、adoption log 或 ignore list 时，固定任务卡按以下方式填槽：

- `相关路径`：列出 `global-skills/`、`skill-packs/`、`proposals/skill-adoption/`、`governance/skill-adoption-log.yaml`、`governance/skill-ignore-list.yaml`。
- `本次任务相关文件`：列出本次涉及的 skill 源目录、proposal、adoption log 或 ignore list。
- `项目画像`：如存在，填写 `config/agent-project-profile.yaml`；不要复制无关画像内容。
- `记忆胶囊`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`；不要复制长记忆。开始执行前同步读取同目录 `task-memory.md`。
- `任务存档`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`；没有任务记忆时填 `无`。runner 执行后由 `context-memory.sh capture` 刷新本机 `task-memory.md` 并归档完整收据。
- `适用治理文档`：填写项目内治理文档；如无项目治理文档，填写 `AGENT_SUITE_PROTOCOL.md`。
- `非目标`：明确不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`，不得运行 `lark-cli update`、`npx skills add/remove/update`，不得自动应用 patch。
- `实施要求`：说明默认先 scan / dry-run，人工确认后才能 adopt / ignore。
- `Verification gate`：优先使用 `bash scripts/govern-new-skills.sh scan`、`bash -n` 脚本语法检查和 `bash scripts/verify.sh`。
- `交付`：必须说明是否生成 proposal / adoption log / ignore list，是否触碰本地 skill 目录，仍需人工确认的事项。

## Heavy 任务补充

Heavy 任务可在任务卡中追加：

```markdown
实施流程：
1. 阅读与诊断 → 输出 root cause / 设计 / 计划 → 等待确认
2. 确认后执行
3. 验证与交付

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 当前上下文没有明确人工批准 mutation 时，停在 plan / confirmation gate。
- 不得把“继续”理解为 Heavy 写入批准。

基线保护：
- 不修改、删除、覆盖（列出受保护数据/目录）
```
