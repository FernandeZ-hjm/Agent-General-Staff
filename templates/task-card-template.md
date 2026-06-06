# Task Card Template

Cursor / Codex 使用此模板生成任务卡，交给指定 Executor 执行。

**输入来源：** 任务卡的输入必须是已确认的方案或 execution contract（参见
`protocol/agent-task-protocol.md` 生命周期阶段 3），不能是原始用户自然语言请求。
Codex / Cursor 必须先完成 ambient preflight → solution phase → user confirmation，
形成 execution contract 后，再把 contract 填入此模板。不得把用户第一句聊天消息直接
当作 Light / Medium / Heavy 分级的依据。

固定规则在 `protocol/agent-task-protocol.md` 和 `protocol/runtime-adapters.md`，不要重复粘贴进任务卡。

---

~~~~markdown
## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- protocol/agent-task-protocol.md
- protocol/task-routing.md
- protocol/runtime-adapters.md
- protocol/project-profile.md
- protocol/context-memory.md
- protocol/cursor-skill-index.md

Executor: Codex / Claude Code / Cursor / Human / Other

Runtime adapter: codex-local / claude-code / cursor / generic

Execution surface: local-workspace / cli / ide / web / remote-control / background-agent

Permission mode: read-only / plan-only / edit-with-confirmation / execute-and-verify

Parallelism: none / subagent / worktree / multi-session / agent-team

任务级别：Light / Medium / Heavy

Heavy 写入批准规则按 protocol/agent-task-protocol.md 执行；“继续”、上下文压缩恢复或 task-notification 接续不算 Heavy 写入批准。

Review gate:
- 按 protocol/agent-task-protocol.md 的 Review Gate 规则执行当前任务级别。

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
按 protocol/agent-task-protocol.md 输出 delivery report。

[skill: verify]
~~~~

---

## 使用说明

- **Cursor / Codex**：先完成 ambient preflight → solution phase → user confirmation，形成 execution contract。然后等待用户明确发出任务卡指令（"生成任务卡"、"按这个方案出任务卡"、"交给 Claude Code 执行"等）。只有收到任务卡指令后，才调用 `ags task compile --task-card-requested` 或手动将 execution contract 填入此模板。对话前台默认可输出压缩执行卡，以 `## 任务卡` 作为统一抬头；固定规则通过协议路径引用，不把整套骨架全文展开给用户确认。不得把原始用户自然语言请求直接当作任务卡输入。不得在用户仅说"方案 OK"而未发出任务卡指令时生成可执行任务卡。
- **Executor**：读取任务卡 + 引用的协议文件，执行并交付。
- 固定规则（安全、分级、runtime adapter、Review gate、验证、交付格式）在协议文件中，任务卡不再重复。
- 为了保持执行稳定性和缓存友好性，任务卡必须使用固定骨架：标题、字段顺序、基础措辞保持不变；只在固定槽位填写动态任务内容。
- `项目画像` 是稳定上下文入口。项目存在 `config/agent-project-profile.yaml` 时只引用路径或提取必要短事实，不把整份画像粘进任务卡；项目无画像时填写 `无`。
- `记忆胶囊` 是人工项目宪章入口。存在本地 capsule 时只引用路径，不粘贴长记忆；没有 capsule 时填写 `无`。Executor 开始任务前必须读取 capsule；如同目录存在 `task-memory.md`，也必须读取。若任务目标与 capsule 的 `## 项目设计目的` 冲突，停止并报告。
- `任务存档` 是自动任务记忆入口。存在本地 `task-memory.md` 时填写该路径；没有任务记忆时填写 `无`。使用 runner 执行后，最终交付报告会先沉淀到本机 `task-memory.md` / `task-archive/`，再打印到前台；完整证据保存在 `$HOME/.agents/memory/projects/<project-slug>/task-archive/`。
- 默认不生成 `.md` 文件产物；只有用户明确要求落盘或需要 runner 直接消费文件时，才创建任务卡文件。
- 对话交付给 Claude Code 的任务卡必须是一个连续 fenced `markdown` block，不按环节拆成多个片段，便于用户一次复制。默认输出可执行的压缩任务卡；“可粘贴”“可复制给 Claude Code”“直接发给 CC 执行”仍然必须编译成 canonical compact task card，不是自由 prompt 或长 runbook；只有用户明确要求“完整骨架”“完整任务卡”“full prompt/self-contained prompt”或 runner/file artifact 需要时，才展开完整模板。
- 对话最终输出只要包含 `Executor: Claude Code`，就必须输出一个可执行任务卡块，且任务卡内容第一条非空行必须是 `## 任务卡`；若生成结果不是这个形态，必须丢弃并重写，不得把自由 runbook、`text` fence 或 prose-first prompt 交给用户粘贴。
- 压缩执行卡固定字段顺序为：`## 任务卡`、`路径`、`Executor`、`Runtime adapter`、`Execution surface`、`Permission mode`、`Parallelism`、`任务级别`、`读取`、`任务`、`目标`、`非目标`、`关键路径`、`验证`、`停止条件`、`交付`、技能标记。
- 压缩执行卡的可读性格式必须稳定：`任务：` 只写一句话；如任务需要拆分条目，把条目放入 `目标：`。`目标：`、`非目标：`、`关键路径：`、`验证：`、`停止条件：`、`交付：` 只要包含多项，就必须把字段名单独成行，后续每项单独换行；不得写成 `目标：1. ... 2. ...`、`验证：- ... - ...` 这种 inline list。推荐格式：
  ```markdown
  目标：
  1. goal_1
  2. goal_2
  非目标：
  - non_goal_1
  - non_goal_2
  ```
- 如果输入材料以 `Executor:`、`Runtime adapter:`、`Permission mode:` 或 `Task level:` 开头，那只是 runtime 字段草稿，不是任务卡。生成器必须把它作为原始任务意图重新填入本模板或全局 fallback 模板；不得原样交付给 Claude Code。
- 如果输入材料以 `目标：`、`背景：`、`硬性要求：`、`建议验证命令：`、`停止条件：` 或 `交付格式：` 开头，且包含 `[skill: ...]` 或明显是要粘贴给 Claude Code/Cursor/Codex 的执行简报，那也只是原始任务意图，不是任务卡。生成器必须把它编译进本模板或全局 fallback 模板；不得保留源 section 顺序后原样交付。
- `[skill: xxx]` 是任务卡元数据，只能出现在规范任务卡末尾；不得附在自由文本 prompt 或 `text` fence 后面。
- `autonomous-low-risk` 尚未进入 Rust canonical gate：Rust task-card-validator 暂不校验此模式。在 validator 实现 Light-only、protected-path 禁止、Heavy 禁止等硬门禁之前，任务卡不得使用 `autonomous-low-risk` 作为 Permission mode 值；使用该值的任务卡会被 validator 拒绝（`AUTONOMOUS_LOW_RISK_NOT_IN_CANONICAL_GATE`）。runtime-adapters.md 中的定义保留为协议目标，不代表当前 canonical gate 已实现。
- 任务卡字段使用 `任务级别：`。`Task level:` 只能出现在用户原始材料或外部笔记中，不能作为最终任务卡字段。
- 任务卡正文包含内嵌代码块时，外层必须使用 `~~~~markdown` / `~~~~`，不得使用三反引号 ` ```markdown `；本模板包含 `.claude/review_targets.json` 的 ` ```json ` 示例，使用三反引号外层会被内部代码块提前截断。
- 实际任务卡进入 runner 前必须通过 Rust task-card-validator 只读校验（`cargo run -p ags-cli -- task validate <task-card>` 或 `bash scripts/validate.sh <task-card>`；旧 `task-card-validator` 命令仅作为隐藏兼容别名保留）；对话输出可通过 `bash scripts/validate.sh -` 从 stdin 校验；校验失败时停止，不进入执行或收据流程。
- 远程控制、SSH、挂载目录、跨仓库任务中，`cwd` 不一定等于实际修改仓库。任务卡必须显式要求 Executor 为本次任务重写 `.claude/review_targets.json`，让显式 review 的审查范围对准实际目标仓库。
- Executor 启动后按固定顺序读取：
  1. 稳定协议文件：`AGENTS.md`、`CLAUDE.md`、`protocol/agent-task-protocol.md`、`protocol/task-routing.md`、`protocol/runtime-adapters.md`、`protocol/cursor-skill-index.md`。
  2. 稳定上下文文件：任务卡声明的 `项目画像`、`记忆胶囊`、同目录 `task-memory.md` 和 `任务存档`，如存在。
  3. 本次任务相关文件：任务卡中列出的相关路径、治理文档、待审查代码或数据说明。
  4. 动态命令输出：如 `git status --short`、验证命令、脚本检查结果，只记录在执行过程或交付报告的验证/状态部分，不放进“读取并遵守”清单。
- 只有跨仓库、外部 agent、或 Executor 无法访问本项目文件时，才使用完整自包含长 prompt。
- 任务级别按 `protocol/task-routing.md` 定义。
- **Task-card request gate**：`ags task compile` 在没有 `--task-card-requested` 参数时拒绝输出可执行任务卡，报告 `executable_allowed=false`、`block_reason=task_card_not_requested`。只有用户明确发出任务卡指令后，generator 才能带 `--task-card-requested` 调用 compiler。参见 `protocol/agent-task-protocol.md` 生命周期阶段 3.5。
- Executor、Runtime adapter、Execution surface、Permission mode、Parallelism、Verification gate 按 `protocol/runtime-adapters.md` 定义；Review gate 的唯一规则表在 `protocol/agent-task-protocol.md`。
- 需要让 runner 自动选择执行层时，可以使用 `scripts/run-task-card.sh <task-card> --auto`；auto mode 不会提高任务卡声明的权限。
- runner 默认启用 Learning Runner：任务卡进入执行前会临时编译 Task IR / compiled brief 并注入给 Claude Code；这些编译产物不是第三种任务卡格式，默认不长期保留，只在本地 memory 中沉淀可复用 `learning-gaps/`。
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
- `任务存档`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`；没有任务记忆时填 `无`。runner 执行后默认由 `context-memory.sh capture` 刷新本机 `task-memory.md`、归档完整收据，并在沉淀后打印最终交付报告；直接粘贴给 Claude Code 的任务卡在 Stop hook 检测到交付报告后也会自动归档。
- `适用治理文档`：填写项目内治理文档；如无项目治理文档，填写 `AGENT_SUITE_PROTOCOL.md`。
- `非目标`：明确不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`，不得运行 `lark-cli update`、`npx skills add/remove/update`，不得接管外部官方 CLI 或项目自管输出层技能，不得自动应用 patch。
- `实施要求`：说明默认先 scan / dry-run，人工确认后才能 adopt / ignore。
- `边界声明`：如任务涉及 `notebooklm`、Hermes 输出层技能、TempoFlow 输出层业务契约、`notebooklm_task_card`、`local_context_pack` 或 `fairness_check_questions`，必须写明它们只可被引用，不能被开发套件 adopt / update / 打包。
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
