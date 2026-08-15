# Task Card Template

Cursor / Codex 使用此模板生成任务卡，交给指定 Executor 执行。

**输入来源：** 任务卡的输入必须是已确认的方案或 execution contract（参见
`protocol/agent-task-protocol.md` 生命周期），不能是原始用户自然语言请求。
Codex / Cursor 必须先完成 ambient preflight，复用请求中已批准的
execution contract；只在 contract 仍缺失或关键决策未定时才形成并确认方案。
然后把 contract 填入此模板。不得把用户第一句聊天消息直接
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

Contract ID: tc-<16 lowercase hex>

Handoff source: explicit-handoff / host-plan-mode / existing-card

Executor: Codex / Claude Code / Cursor / OMP / Human / Other

Runtime adapter: codex-local / claude-code / cursor / omp / generic

Execution surface: local-workspace / cli / ide / web / remote-control / background-agent

Execution mode: plan-only / single-writer / fanout-in-card / fanout-cross-card

Execution topology: single / parallel / worktree

Execution effort: low / normal / high / exhaustive

Delegation planning: no / yes

任务级别：Light / Medium / Heavy

Heavy 的 review gate 规则按 protocol/agent-task-protocol.md 执行；任务级别不改写三个权限字段。“继续”、上下文压缩恢复或 task-notification 接续不会改写任务卡权限。

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
- AGS start hook 已注入时以注入上下文为准；否则同步读取同目录 `task-memory.md`；不得覆盖 `context-capsule.md`

任务存档：
- 无 / `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`

目标文件夹路径：
- `<absolute path to target folder>`

相关路径：
- `path_1`
- `path_2`

本次任务相关文件：
- `path_or_doc_1`
- `path_or_doc_2`

适用治理文档：
- 无 / `<project-specific-governance-doc>`

目标：
- G-01: goal_1
- G-02: goal_2

验收标准：
- AC-01 -> G-01: <可观察、可判定的通过条件>
- AC-02 -> G-02: <可观察、可判定的通过条件>

非目标：
- non_goal_1
- non_goal_2

子任务编排：
- mode: none / optional / required
- <可选槽位：声明可拆分结构、子任务边界、只读/可写范围、回收要求；省略即 mode=none>
- constraints:
  - <子任务约束：只读/可写边界、禁止越界、结果汇总回主 executor>

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
  - V-01 -> AC-01: <verification command or explicit manual check>
  - V-02 -> AC-02: <verification command or explicit manual check>
- expected evidence:
  - EV-01 -> AC-01: <test result / diff summary / report path>
  - EV-02 -> AC-02: <test result / diff summary / report path>
- stop condition:
  - <when to pause and report instead of continuing>

交付：
- 按 protocol/agent-task-protocol.md 输出 delivery report。
- 报告必须回填本卡 `Contract ID`、LaunchPlan `task_card_hash`，并逐项闭环全部 `G-*`、`AC-*`、`V-*`；`partial` / `blocked` 的未闭环 ID 集合必须与非闭环状态完全相等（含待审 `review-gate`），不得缺失、夹带或隐藏。
- 报告使用 Closure schema 1.1，回填 `task-card-hash`、`launch-plan-hash`、`execution-mode-used`、`execution-topology-used` 与 `delegation-used`。
- 报告落盘后运行 `ags govern task close --task-card <task-card> --launch-plan <launch-plan> --delivery-report <delivery-report> --workspace <repo> --format json`，再用返回的 `action_ref` 调用 `ags apply`；该 Operation 验证并原子生成 receipt 与 session closure pointer。
~~~~

---

## 使用说明

- **Cursor / Codex / Generic Agent**：宿主保留完整对话并完成自然语言解释，生成本 canonical 任务卡后通过 typed `govern.task.validate` Operation 校验。`DirectResponse` 直接交付；已有批准 contract 且收到明确同会话修改授权时走宿主原生 direct-edit；仅在关键决策仍未解决时进入 solution phase。AGS core 不解释原始对话，也不另建第二份最终计划。
- **宿主 Plan mode 适配**：Plan mode 内只能只读探索与关闭决策；最终产物是唯一一张 decision-complete `## 任务卡`。若宿主 UI 要求 `<proposed_plan>`，该标签只是渲染 envelope，内部第一条非空行仍必须是 `## 任务卡`。用户选择 Execute 后，宿主先切换到可执行/Default mode，再把原任务卡正文与 `task_card_hash` 原样交给执行 Agent；不得重新生成、摘要或改写任务卡。宿主 Plan mode 与任务卡 `Execution mode` 是两个独立状态。
- **Executor**：读取任务卡 + 引用的协议文件，执行并交付。
- 固定规则（安全、分级、runtime adapter、Review gate、验证、交付格式）在协议文件中，任务卡不再重复。
- 为了保持执行稳定性和缓存友好性，任务卡必须使用固定骨架：标题、字段顺序、基础措辞保持不变；只在固定槽位填写动态任务内容。
- `Contract ID` 由 compiler 基于已关闭 handoff contract 与 `Handoff source` 确定性生成，格式为 `tc-` + 16 位小写十六进制；调用方不得覆盖。`Handoff source` 只允许 `explicit-handoff`、`host-plan-mode`、`existing-card`，只描述卡的来源，不授权执行。
- `目标` 使用稳定 `G-NN`；`验收标准` 使用 `AC-NN -> G-NN`；`Verification gate` 必须同时声明 `V-NN -> AC-NN` 和 `EV-NN -> AC-NN`。每个目标至少有一个验收标准，每个验收标准至少有一个验证项与一个预期证据项。不得用“按需验证”“测试一下”或无 ID 的自由文本替代。
- `项目画像` 是稳定上下文入口。项目存在 `config/agent-project-profile.yaml` 时只引用路径或提取必要短事实，不把整份画像粘进任务卡；项目无画像时填写 `无`。
- `记忆胶囊` 是人工项目宪章入口。存在本地 capsule 时只引用路径，不粘贴长记忆；没有 capsule 时填写 `无`。AGS-governed host 正常由只读 `SessionStart` memory hook 自动注入 capsule 和同目录 `task-memory.md`；hook 不可用、未安装或外部 executor 无法接收注入时，Executor 开始任务前必须按路径读取。若任务目标与 capsule 的 `## 项目设计目的` 冲突，停止并报告。
- `任务存档` 是任务记忆入口。存在本地 `task-memory.md` 时填写该路径；没有任务记忆时填写 `无`。`ags govern task plan` 只准备 LaunchPlan，不会自动执行任务、写入任务记忆或归档交付；真实 Executor/宿主完成后按项目记忆协议写入。
- `目标文件夹路径` 是本次任务的实际工作目录或目标仓库根目录，必须填写绝对路径；远程控制、挂载目录、跨仓库或启动目录与目标目录不一致时，以实际会被读写的目标文件夹为准。
- 默认不生成 `.md` 文件产物；只有用户明确要求落盘或需要 `ags govern task plan` 从路径读取任务卡时，才创建任务卡文件。
- 技能标记是可选的末尾元数据，不属于任务级别默认项。仅当 typed proposal 或已确认 handoff contract 给出精确 `skill_id`，并且 Skill Resolver 以相同 `entrypoint + snapshot_hash` 准入时，才在 `交付` 段之后追加 0..n 行 `[skill: <canonical-name>]`；没有精确命中就完全省略。不得从关键词或任务级别推导标签。
- Verification gate 是协议要求，不默认依赖任何技能。只有精确 SkillTarget / 已确认 contract 选择 Superpowers 父技能与 internal entrypoint 时，才写入对应要求和一次父标签。
- 任务卡只有唯一形态：本文件 `protocol/task-card-template.md` 定义的固定骨架。跨仓库、外部 agent、或 Executor 无法访问本项目文件时，仍使用同一骨架，并把所需固定规则内联进去使其自包含；不得切换到第二套模板或按任务级别选用不同模板文件。任务级别 Light / Medium / Heavy 只是 `任务级别：` 字段值，不决定模板文件。
- “完整”“压缩”“compact”“full”“可粘贴”“可复制给 Claude Code”“直接发给 CC 执行”只是对话展示偏好，不是任务卡形态。compact 任务卡格式已删除：任务卡只有唯一经典固定骨架，这些词不得改变任务卡骨架、标题或槽位顺序，也不得据此生成 compact 骨架或“默认 compact 可执行卡”。
- 对话交付任务卡时，默认使用普通 Markdown 输出整张任务卡，不要用一个外层 fenced code block 包住整卡；这样对话框可以自然换行。只有用户明确要求单个 literal copy block、文件 artifact，或任务卡内含嵌套 fenced 代码块且必须作为一个代码块复制时，才允许外层使用 `~~~~markdown` / `~~~~`。
- 对话最终输出只要包含 `Executor: Claude Code`，就必须输出一个可执行任务卡块，且任务卡内容第一条非空行必须是 `## 任务卡`；若生成结果不是这个形态，必须丢弃并重写，不得把自由 runbook、`text` fence 或 prose-first prompt 交给用户粘贴。
- 需求入口由宿主构造 typed Operation，并通过 CLI 或 `ags_decide` 进入同一 registry。输出门禁只约束 handoff 产物，不限制已授权的同会话 direct-edit。
- 本项目任务卡可读性格式必须稳定：`任务：` 只写一句话；如任务需要拆分条目，把条目放入 `目标：`。`目标：`、`非目标：`、`目标文件夹路径：`、`相关路径：`、`本次任务相关文件：`、`验证：`、`交付：` 只要包含多项，就必须把字段名单独成行，后续每项单独换行；不得写成 `目标：1. ... 2. ...`、`验证：- ... - ...` 这种 inline list。推荐格式：
  ```markdown
  目标：
  - G-01: goal_1
  - G-02: goal_2
  验收标准：
  - AC-01 -> G-01: observable_result_1
  - AC-02 -> G-02: observable_result_2
  非目标：
  - non_goal_1
  - non_goal_2
  ```
- 如果输入材料以 `Executor:`、`Runtime adapter:`、`Execution mode:` 或 `Task level:` 开头，那只是 runtime 字段草稿，不是任务卡。生成器必须把它作为原始任务意图重新填入本 canonical 任务卡骨架；不得原样交付给 Claude Code。
- 如果输入材料以 `目标：`、`背景：`、`硬性要求：`、`建议验证命令：`、`停止条件：` 或 `交付格式：` 开头，且包含 `[skill: ...]` 或明显是要粘贴给 Claude Code/Cursor/Codex 的执行简报，那也只是原始任务意图，不是任务卡。生成器必须把它编译进本 canonical 任务卡骨架；不得保留源 section 顺序后原样交付。
- `[skill: xxx]` 是任务卡元数据，只能出现在规范任务卡末尾；不得附在自由文本 prompt 或 `text` fence 后面。
- `Execution mode` 只允许 `plan-only`、`single-writer`、`fanout-in-card`、`fanout-cross-card`。
- `Execution topology` 只允许 `single`、`parallel`、`worktree`。
- `Delegation planning` 只允许 `no`、`yes`，只授权制定委派方案，不授予多写者权限。
- 任务卡字段使用 `任务级别：`。`Task level:` 只能出现在用户原始材料或外部笔记中，不能作为最终任务卡字段。
- 如果用户明确要求单个 literal copy block 或文件 artifact，且任务卡正文包含内嵌代码块时，外层必须使用 `~~~~markdown` / `~~~~`，不得使用三反引号 ` ```markdown `；本模板包含 `.claude/review_targets.json` 的 ` ```json ` 示例，使用三反引号外层会被内部代码块提前截断。
- 实际任务卡进入执行前必须通过 Rust validator 只读校验（`ags govern task validate --task-card <task-card> --workspace <repo> --format json`）；校验失败时停止，不进入执行或收据流程。
- 首个非空行已经是 `## 任务卡` 的输入是已有任务卡：合法卡跳过生成，直接进入 policy / runner；非法卡停止，不得回落为原始意图重新生成。
- 远程控制、SSH、挂载目录、跨仓库任务中，`cwd` 不一定等于实际修改仓库。任务卡必须显式要求 Executor 为本次任务重写 `.claude/review_targets.json`，让显式 review 的审查范围对准实际目标仓库。
- Executor 启动后按固定顺序读取：
  1. 稳定协议文件：`AGENTS.md`、`CLAUDE.md`、`protocol/agent-task-protocol.md`、`protocol/task-routing.md`、`protocol/runtime-adapters.md`、`protocol/cursor-skill-index.md`。
  2. 稳定上下文文件：任务卡声明的 `项目画像`、`记忆胶囊`、同目录 `task-memory.md` 和 `任务存档`，如存在；AGS start hook 已注入的记忆上下文可作为本项的已读证据，hook 不可用时按路径读取。
  3. 本次任务相关文件：任务卡中列出的目标文件夹路径、相关路径、治理文档、待审查代码或数据说明。
  4. 动态命令输出：如 `git status --short`、验证命令、脚本检查结果，只记录在执行过程或交付报告的验证/状态部分，不放进“读取并遵守”清单。
- 跨仓库、外部 agent、或 Executor 无法访问本项目文件时，使用同一 canonical 骨架的自包含形态（内联所需固定规则），不另立 fallback 任务卡格式。
- 任务级别按 `protocol/task-routing.md` 定义。
- **Task-card handoff gate**：显式交接与宿主 Plan mode 都必须先形成 confirmed handoff contract。两条路径都只生成交接 artifact，不授权 mutation；输入重开 solution work 时停止。此规则不限制已授权的同会话 `direct-edit`。参见 `protocol/agent-task-protocol.md` 生命周期阶段 3.5。
- Executor、Runtime adapter、Execution surface、Execution mode、Execution topology、Verification gate 按 `protocol/runtime-adapters.md` 定义；Review gate 的唯一规则表在 `protocol/agent-task-protocol.md`。
- `Execution effort` 使用中性执行强度语义（`low` / `normal` / `high` / `exhaustive`），默认 `normal`；它只表示思考强度，绝不映射为权限、并行或 review 豁免。其他旧值一律拒绝。
- `Delegation planning` 只允许 `no` / `yes`，默认 `no`。`yes` 允许宿主按卡内 `子任务编排` 制定并执行 subagent / workflow 方案，但不单独授予多写者、并行、worktree、commit 或外部写入权限；这些权限仍由任务卡的 `Execution mode`、`Execution topology`、显式 mutation/commit 边界与 lane 约束共同决定。
- `子任务编排` 是可选槽位，`mode` 取 `none` / `optional` / `required`，默认 `none`。任务卡声明的 Execution mode、Execution topology、Delegation planning、是否允许 commit 与 lane 约束高于通用 playbook 默认值；每条 lane 只在授权边界内产出，主 executor 统一集成、验证与交付。
- `ags govern task plan` 是唯一 LaunchPlan 准备 Operation。允许时返回 `HOST_EXECUTION_REQUIRED`；它不启动宿主、不执行任务、不验证结果、不写最终收据。
- 涉及本地 Agent 技能目录时，必须引用项目内对应治理文档。普通任务只读取静态快照；显式纳管任务使用 `ags govern skill install|remove` 的密封 plan/apply 协议，来源必须完整性绑定。最终输出仍使用本文件的固定任务卡骨架。

## 与全局提示词生成器的关系

宿主必须生成本文件定义的唯一 canonical 任务卡骨架，不另立第二套格式；AGS 只验证 typed artifact。

### 硬约束：唯一合法模板

任务执行提示词只有唯一合法骨架：本文件 `protocol/task-card-template.md` 定义的固定骨架。

- AGS / 项目协议可访问时，生成该骨架并引用项目协议文件，不重复固定规则。
- Executor 无法访问项目文件（跨仓库、外部 agent、自包含 prompt）时，仍用同一骨架，把所需固定规则内联进去使其自包含。这是同一骨架的交付形态，不是第二套模板。

禁止自由 runbook、机器专用模板、阶段专用模板、compact 骨架、按级别拆分的模板文件，或任何不属于该唯一骨架的自造格式。任务级别 Light / Medium / Heavy 只是 `任务级别：` 字段值，不决定模板文件。

## Skill Governance 治理任务补充

涉及本地 Agent 技能或第三方能力升级时，固定任务卡按以下方式填槽：

- `相关路径`：列出实际 skill 源目录、`manifests/skills-registry.yaml`、`manifests/third-party-capabilities.yaml` 与静态 snapshot 刷新入口。
- `目标文件夹路径`：填写本次技能治理实际读写的仓库根目录或目标技能根目录的绝对路径。
- `本次任务相关文件`：列出本次涉及的 skill 源目录和 registry 文件。
- `项目画像`：如存在，填写 `config/agent-project-profile.yaml`；不要复制无关画像内容。
- `记忆胶囊`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`；不要复制长记忆。AGS start hook 已注入时以注入上下文为准；hook 不可用时，开始执行前同步读取同目录 `task-memory.md`。
- `任务存档`：如存在，填写本机 memory URI；没有任务记忆时填 `无`。宿主生命周期只调用 standalone `ags-host lifecycle`；SessionEnd 仅归档成功 `govern.task.close` 留下的 closure pointer，无 pointer 时安全跳过且不得猜测 transcript。
- `适用治理文档`：填写项目内治理文档；如无项目治理文档，填写 `AGENT_SUITE_PROTOCOL.md`。
- `非目标`：明确不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`，不得运行 `lark-cli update`、`npx skills add/remove/update`，不得接管外部官方 CLI 或项目自管输出层技能，不得自动应用 patch。
- `实施要求`：说明来源变更只发生在明确安装/升级中；验证后每个宿主只刷新一次静态 snapshot。
- `边界声明`：如任务涉及 `notebooklm`、Hermes 输出层技能、TempoFlow 输出层业务契约、`notebooklm_task_card`、`local_context_pack` 或 `fairness_check_questions`，必须写明它们只可被引用，不能被开发套件接管、更新或打包。
- `Verification gate`：优先使用 `ags govern capability inventory`、`ags check governance` 与结构化 `ags test`。
- `交付`：必须说明是否触碰本地 skill 目录、是否刷新静态 snapshot，以及仍需人工确认的事项。

## Heavy 任务补充

Heavy 任务只能追加与当前 `Execution mode` 匹配的分支，不得把两个分支同时写进任务卡。

`Execution mode: plan-only`：

```markdown
实施流程：
1. 阅读与诊断
2. 输出 root cause / 设计 / 实施计划 / 验证计划
3. 停止，不修改文件、不执行写操作

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 保持 `plan-only`；“继续”或压缩摘要不得将其升级为可写权限。

基线保护：
- 不修改、删除、覆盖（列出受保护数据/目录）
```

`Execution mode: single-writer`（fanout 模式按同一闭环增加明确多写者边界）：

```markdown
实施流程：
1. 阅读与必要诊断
2. 按任务卡直接实施
3. 验证与交付；不追加新的 plan 轮次

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 保持任务卡声明的 execution mode/topology，继续执行并验证；Heavy 只追加独立 review gate。

基线保护：
- 不修改、删除、覆盖（列出受保护数据/目录）
```
