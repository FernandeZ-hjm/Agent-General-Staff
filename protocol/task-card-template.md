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

Executor: Codex / Claude Code / Cursor / Human / Other

Runtime adapter: codex-local / claude-code / cursor / generic

Execution surface: local-workspace / cli / ide / web / remote-control / background-agent

Permission mode: plan-only / execute-and-verify

Parallelism: none / subagent / worktree / multi-session / agent-team

Execution effort: low / normal / high / exhaustive

Workflow authority: none / within-card / plan-only / allowed

任务级别：Light / Medium / Heavy

Heavy 的 review gate 规则按 protocol/agent-task-protocol.md 执行；任务级别不改写 Permission mode（未声明时 compiler 默认 plan-only，显式 execute-and-verify 直接执行并验证）。“继续”、上下文压缩恢复或 task-notification 接续不会改写任务卡权限。

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
1. goal_1
2. goal_2

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
  - <verification command>
- expected evidence:
  - <test result / diff summary / report path>
- stop condition:
  - <when to pause and report instead of continuing>

交付：
按 protocol/agent-task-protocol.md 输出 delivery report。
~~~~

---

## 使用说明

- **Cursor / Codex**：先完成 ambient preflight，再由 MCP `ags_route_request` 返回唯一结构化 `RequestDecision`。`DirectResponse` 直接交付；已有批准 contract 且收到明确同会话修改授权时可走宿主直接执行；仅在关键决策仍未解决时进入 solution phase。明确任务卡/跨 Agent 交接指令且交接契约已经确认后，才调用 `ags task compile --task-card-requested --confirmed-handoff-contract` 或把已确认方案填入本模板。对话前台输出任务卡时必须以 `## 任务卡` 作为统一抬头，并保持固定槽位顺序。不得把原始用户自然语言请求直接当作任务卡输入，也不得为了本地直接执行伪造任务卡请求。
- **Executor**：读取任务卡 + 引用的协议文件，执行并交付。
- 固定规则（安全、分级、runtime adapter、Review gate、验证、交付格式）在协议文件中，任务卡不再重复。
- 为了保持执行稳定性和缓存友好性，任务卡必须使用固定骨架：标题、字段顺序、基础措辞保持不变；只在固定槽位填写动态任务内容。
- `项目画像` 是稳定上下文入口。项目存在 `config/agent-project-profile.yaml` 时只引用路径或提取必要短事实，不把整份画像粘进任务卡；项目无画像时填写 `无`。
- `记忆胶囊` 是人工项目宪章入口。存在本地 capsule 时只引用路径，不粘贴长记忆；没有 capsule 时填写 `无`。AGS-governed host 正常由只读 `SessionStart` memory hook 自动注入 capsule 和同目录 `task-memory.md`；hook 不可用、未安装或外部 executor 无法接收注入时，Executor 开始任务前必须按路径读取。若任务目标与 capsule 的 `## 项目设计目的` 冲突，停止并报告。
- `任务存档` 是自动任务记忆入口。存在本地 `task-memory.md` 时填写该路径；没有任务记忆时填写 `无`。使用 runner 执行后，最终交付报告会先沉淀到本机 `task-memory.md` / `task-archive/`，再打印到前台；完整证据保存在 `$HOME/.agents/memory/projects/<project-slug>/task-archive/`。
- `目标文件夹路径` 是本次任务的实际工作目录或目标仓库根目录，必须填写绝对路径；远程控制、挂载目录、跨仓库或启动目录与目标目录不一致时，以实际会被读写的目标文件夹为准。
- 默认不生成 `.md` 文件产物；只有用户明确要求落盘或需要 runner 直接消费文件时，才创建任务卡文件。
- 技能标记是可选的末尾元数据，不属于任务级别默认项。仅当 `RequestDecision` 的 `SkillDemand` 经 Skill Resolver 精确命中，或已确认 handoff contract 精确命中某项可路由技能时，才在 `交付` 段之后追加 0..n 行 `[skill: <canonical-name>]`；没有精确命中就完全省略。不得默认追加 `[skill: superpowers]` 或按 Light / Medium / Heavy 批量附加技能。
- Verification gate 是协议要求，不默认依赖任何技能。仅当 `RequestDecision` / Skill Resolver 或已确认 contract 精确选择 Superpowers playbook 时，才在 `实施要求` 中写明加载 `superpowers` 父技能和对应 internal entrypoint，并在末尾追加一次父标签；否则不得写入该要求或标签。
- 任务卡只有唯一形态：本文件 `protocol/task-card-template.md` 定义的固定骨架。跨仓库、外部 agent、或 Executor 无法访问本项目文件时，仍使用同一骨架，并把所需固定规则内联进去使其自包含；不得切换到第二套模板或按任务级别选用不同模板文件。任务级别 Light / Medium / Heavy 只是 `任务级别：` 字段值，不决定模板文件。
- “完整”“压缩”“compact”“full”“可粘贴”“可复制给 Claude Code”“直接发给 CC 执行”只是对话展示偏好，不是任务卡形态。compact 任务卡格式已删除：任务卡只有唯一经典固定骨架，这些词不得改变任务卡骨架、标题或槽位顺序，也不得据此生成 compact 骨架或“默认 compact 可执行卡”。
- 对话交付任务卡时，默认使用普通 Markdown 输出整张任务卡，不要用一个外层 fenced code block 包住整卡；这样对话框可以自然换行。只有用户明确要求单个 literal copy block、文件 artifact，或任务卡内含嵌套 fenced 代码块且必须作为一个代码块复制时，才允许外层使用 `~~~~markdown` / `~~~~`。
- 对话最终输出只要包含 `Executor: Claude Code`，就必须输出一个可执行任务卡块，且任务卡内容第一条非空行必须是 `## 任务卡`；若生成结果不是这个形态，必须丢弃并重写，不得把自由 runbook、`text` fence 或 prose-first prompt 交给用户粘贴。
- 需求入口由 MCP `ags_route_request` 的结构化 `RequestDecision` 统一表达；交付前用 `ags gate output <candidate>` 自检 canonical 形态。输出门禁只约束 handoff 产物，不限制已授权的 `direct-edit`。
- 本项目任务卡可读性格式必须稳定：`任务：` 只写一句话；如任务需要拆分条目，把条目放入 `目标：`。`目标：`、`非目标：`、`目标文件夹路径：`、`相关路径：`、`本次任务相关文件：`、`验证：`、`交付：` 只要包含多项，就必须把字段名单独成行，后续每项单独换行；不得写成 `目标：1. ... 2. ...`、`验证：- ... - ...` 这种 inline list。推荐格式：
  ```markdown
  目标：
  1. goal_1
  2. goal_2
  非目标：
  - non_goal_1
  - non_goal_2
  ```
- 如果输入材料以 `Executor:`、`Runtime adapter:`、`Permission mode:` 或 `Task level:` 开头，那只是 runtime 字段草稿，不是任务卡。生成器必须把它作为原始任务意图重新填入本 canonical 任务卡骨架；不得原样交付给 Claude Code。
- 如果输入材料以 `目标：`、`背景：`、`硬性要求：`、`建议验证命令：`、`停止条件：` 或 `交付格式：` 开头，且包含 `[skill: ...]` 或明显是要粘贴给 Claude Code/Cursor/Codex 的执行简报，那也只是原始任务意图，不是任务卡。生成器必须把它编译进本 canonical 任务卡骨架；不得保留源 section 顺序后原样交付。
- `[skill: xxx]` 是任务卡元数据，只能出现在规范任务卡末尾；不得附在自由文本 prompt 或 `text` fence 后面。
- `Permission mode` 只允许 `plan-only` 和 `execute-and-verify`；生成器不得输出第三种过渡、确认或自治模式。
- 任务卡字段使用 `任务级别：`。`Task level:` 只能出现在用户原始材料或外部笔记中，不能作为最终任务卡字段。
- 如果用户明确要求单个 literal copy block 或文件 artifact，且任务卡正文包含内嵌代码块时，外层必须使用 `~~~~markdown` / `~~~~`，不得使用三反引号 ` ```markdown `；本模板包含 `.claude/review_targets.json` 的 ` ```json ` 示例，使用三反引号外层会被内部代码块提前截断。
- 实际任务卡进入 runner 前必须通过 Rust task-card-validator 只读校验（`cargo run -p ags-cli -- task validate <task-card>` 或 `bash scripts/validate.sh <task-card>`；旧 `task-card-validator` 命令仅作为隐藏兼容别名保留）；对话输出可通过 `bash scripts/validate.sh -` 从 stdin 校验；校验失败时停止，不进入执行或收据流程。
- 首个非空行已经是 `## 任务卡` 的输入是已有任务卡：合法卡跳过生成，直接进入 policy / runner；非法卡停止，不得回落为原始意图重新生成。
- 远程控制、SSH、挂载目录、跨仓库任务中，`cwd` 不一定等于实际修改仓库。任务卡必须显式要求 Executor 为本次任务重写 `.claude/review_targets.json`，让显式 review 的审查范围对准实际目标仓库。
- Executor 启动后按固定顺序读取：
  1. 稳定协议文件：`AGENTS.md`、`CLAUDE.md`、`protocol/agent-task-protocol.md`、`protocol/task-routing.md`、`protocol/runtime-adapters.md`、`protocol/cursor-skill-index.md`。
  2. 稳定上下文文件：任务卡声明的 `项目画像`、`记忆胶囊`、同目录 `task-memory.md` 和 `任务存档`，如存在；AGS start hook 已注入的记忆上下文可作为本项的已读证据，hook 不可用时按路径读取。
  3. 本次任务相关文件：任务卡中列出的目标文件夹路径、相关路径、治理文档、待审查代码或数据说明。
  4. 动态命令输出：如 `git status --short`、验证命令、脚本检查结果，只记录在执行过程或交付报告的验证/状态部分，不放进“读取并遵守”清单。
- 跨仓库、外部 agent、或 Executor 无法访问本项目文件时，使用同一 canonical 骨架的自包含形态（内联所需固定规则），不另立 fallback 任务卡格式。
- 任务级别按 `protocol/task-routing.md` 定义。
- **Task-card handoff gate**：`ags task compile` 需要 `--task-card-requested` 与 `--confirmed-handoff-contract` 两个结构化信号；缺少时分别以 `task_card_not_requested` 或 `handoff_contract_not_confirmed` 拒绝，输入重开 solution work 时以 `solution_formation_required` 拒绝。此规则不限制已授权的同会话 `direct-edit`。参见 `protocol/agent-task-protocol.md` 生命周期阶段 3.5。
- Executor、Runtime adapter、Execution surface、Permission mode、Parallelism、Verification gate 按 `protocol/runtime-adapters.md` 定义；Review gate 的唯一规则表在 `protocol/agent-task-protocol.md`。
- `Execution effort` 使用中性执行强度语义（`low` / `normal` / `high` / `exhaustive`），默认 `normal`；它只表示思考强度，绝不映射为权限、并行或 review 豁免。宿主私有深度/工作流触发词（如 `ultracode`）不得写进任务卡前台生成路径，只能由 claude-code adapter / runner 按 resolved policy 在执行层翻译；`ultracode` 仅作为旧值解析兼容保留，task compiler 不再生成。
- `Workflow authority` 声明是否允许 subagent / workflow（`none` / `within-card` / `plan-only` / `allowed`），默认 `none`；它只声明授权，不直接点火。
- `子任务编排` 是可选槽位，`mode` 取 `none` / `optional` / `required`，默认 `none`（省略即 `none`）。`mode != none` 时 validator 要求 `Workflow authority` 非 none 且 `Parallelism` 为 subagent/worktree/multi-session/agent-team；该槽位只声明可拆分结构、子任务边界与回收要求，真正 subagent / workflow 点火仍由 claude-code adapter / runner 按 resolved policy 翻译，不由任务卡正文触发。子任务只能装可并行工作（只读审计 / 实现 / 文档同步 / 测试补充）；最终验证、交付报告、commit、push、release gate 必须由主 executor 独做，子任务结果合并为单一 diff 后由主 executor 统一验证与交付（见 `protocol/runtime-adapters.md` §Subtask Scope Rules）。
- `scripts/run-task-card.sh` 是薄包装层，把校验 / gate / policy / adapter / 收据规划全部委托给 Rust runner（`ags run`）。它实际只支持 `--check-only`（gate 预览后停止）、`--dry-run`（输出完整 launch plan 不执行）、`--current-task-approval`（向 resolver 传递 audit/hint 信号，不解锁执行权限，级别不因此降级或提权）、`--approve-writes`（audit/hint 信号；仍可作为 M9 generic-adapter 能力上限 override）、`--format text|json`（透传给 `ags run`）。包装层本身不实现执行层自动选择，不会提高任务卡声明的权限。
- 自动执行层选择 / Learning Runner / 临时 Task IR / compiled brief 注入 / `learning-gaps/` 沉淀均为协议目标，planned（尚未实现）：当前 `scripts/run-task-card.sh` 与 `ags run` 不编译 Task IR，不注入 compiled brief，不沉淀 learning-gaps。任务卡不得假设这些行为已生效。
- 涉及本地 Agent 技能同步、proposal、adoption log 或 ignore list 时，必须引用项目内对应治理文档；如无项目治理文档，使用前台技能治理控制台 `ags skill`（`inventory` / `dedupe` / `update` / `sync` / `verify`）。套件级 `scripts/govern-new-skills.sh` 为 Phase 2 规划项，尚未实现，不得作为可运行步骤引用。最终输出仍使用本文件的固定任务卡骨架。

## 与全局提示词生成器的关系

`ags task compile` 必须生成本文件定义的唯一 canonical 任务卡骨架，不另立第二套格式。

### 硬约束：唯一合法模板

任务执行提示词只有唯一合法骨架：本文件 `protocol/task-card-template.md` 定义的固定骨架。

- AGS / 项目协议可访问时，生成该骨架并引用项目协议文件，不重复固定规则。
- Executor 无法访问项目文件（跨仓库、外部 agent、自包含 prompt）时，仍用同一骨架，把所需固定规则内联进去使其自包含。这是同一骨架的交付形态，不是第二套模板。

禁止自由 runbook、机器专用模板、阶段专用模板、compact 骨架、按级别拆分的模板文件，或任何不属于该唯一骨架的自造格式。任务级别 Light / Medium / Heavy 只是 `任务级别：` 字段值，不决定模板文件。

## Skill Governance 治理任务补充

涉及本地 Agent 技能、下载/拖拽导入、proposal、adoption log 或 ignore list 时，固定任务卡按以下方式填槽：

- `相关路径`：列出 `global-skills/`、`skill-packs/`、`proposals/skill-adoption/`、`governance/skill-adoption-log.yaml`、`governance/skill-ignore-list.yaml`。
- `目标文件夹路径`：填写本次技能治理实际读写的仓库根目录或目标技能根目录的绝对路径。
- `本次任务相关文件`：列出本次涉及的 skill 源目录、proposal、adoption log 或 ignore list。
- `项目画像`：如存在，填写 `config/agent-project-profile.yaml`；不要复制无关画像内容。
- `记忆胶囊`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/context-capsule.md`；不要复制长记忆。AGS start hook 已注入时以注入上下文为准；hook 不可用时，开始执行前同步读取同目录 `task-memory.md`。
- `任务存档`：如存在，填写 `$HOME/.agents/memory/projects/<project-slug>/task-memory.md`；没有任务记忆时填 `无`。任务开始由 Start hook 链路（`context-memory-start.py`）只读注入 capsule / `task-memory.md`；任务结束后由 Stop hook 链路（`claude-stop-memory-capture.py` → `context-memory.sh capture`）归档完整收据并刷新本机 `task-memory.md`；这些链路由 `ags setup --yes --register-claude` 安装脚本并挂载到工作区 SessionStart/Stop pipeline，`context-capsule.md` 绝不被自动覆盖。若本机尚未运行 setup 安装该链路，任务卡不得假设记忆注入或刷新会自动发生；可重新运行 setup 或由人工读取/沉淀。
- `适用治理文档`：填写项目内治理文档；如无项目治理文档，填写 `AGENT_SUITE_PROTOCOL.md`。
- `非目标`：明确不得写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`，不得运行 `lark-cli update`、`npx skills add/remove/update`，不得接管外部官方 CLI 或项目自管输出层技能，不得自动应用 patch。
- `实施要求`：说明默认先 scan / dry-run，人工确认后才能 adopt / ignore。
- `边界声明`：如任务涉及 `notebooklm`、Hermes 输出层技能、TempoFlow 输出层业务契约、`notebooklm_task_card`、`local_context_pack` 或 `fairness_check_questions`，必须写明它们只可被引用，不能被开发套件 adopt / update / 打包。
- `Verification gate`：优先使用 `ags skill inventory`（前台技能治理控制台的只读盘点）、`bash -n` 脚本语法检查和 `bash scripts/verify.sh`。`scripts/govern-new-skills.sh scan` 为 Phase 2 规划项、尚未实现，不得作为 Verification gate 步骤引用。
- `交付`：必须说明是否生成 proposal / adoption log / ignore list，是否触碰本地 skill 目录，仍需人工确认的事项。

## Heavy 任务补充

Heavy 任务只能追加与当前 `Permission mode` 匹配的分支，不得把两个分支同时写进任务卡。

`Permission mode: plan-only`：

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

`Permission mode: execute-and-verify`：

```markdown
实施流程：
1. 阅读与必要诊断
2. 按任务卡直接实施
3. 验证与交付；不追加新的 plan 轮次

Resume / 压缩恢复保护：
- 遇到“继续”、上下文压缩恢复或 task-notification 接续时，重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- 保持 `execute-and-verify`，继续执行并验证；Heavy 只追加独立 review gate。

基线保护：
- 不修改、删除、覆盖（列出受保护数据/目录）
```
