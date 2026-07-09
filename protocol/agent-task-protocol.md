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
- 读取 preflight 暴露的项目上下文、协议文件、记忆路径和相关文档
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

### 3.6. 入口意图识别与前台输出形态门禁（deterministic）

任务卡请求"入口前绕过"的根因是入口意图识别依赖模型自由判断。AGS 提供确定性门禁，宿主必须使用，不得仅凭模型判断把"给我提示词"当成普通 prose：

- **入口意图识别**：`ags gate prompt-request <request>`（或 MCP `ags_solution_check`
  的 `detected_task_card_request` / `detected_triggers` 字段）对用户请求跑确定性
  分类器（`prompt-request-classifier`）。命中"给我提示词""生成提示词""任务卡"
  "交给 Claude Code""给 CC 执行""写个 prompt""handoff""让 Claude 做"等中英文
  触发词时 `decision=require_task_card`，宿主必须进入任务卡生成闭环。检测只是
  信号，不替代任务卡指令门槛——`executable_allowed` 仍要求显式任务卡指令。
- **前台输出形态门禁**：任务卡请求命中、或最终输出包含 `Executor:` 字段时，
  前台最终输出第一条非空行必须是 `## 任务卡`。`ags gate output <candidate>` 做
  确定性校验：首行非 `## 任务卡` → `block_reason=bad_output_shape`；首行正确但
  validator 不通过 → `block_reason=validation_failed`；两者都阻断（`decision=stop`，
  退出码 1）并发出 `governance_miss` 事件。
- **`governance_miss` 事件**：当任务卡请求即将以非 canonical 形态离开前台时，
  `ags gate output` 在输出中发出 `governance_miss`（字段 `event`、`detected_kind`、
  `matched_triggers`、`blocked_reason`、`stage`、`sample_redacted`）。AGS 自身不
  落盘——由宿主决定是否持久化样本用于规则升级。AGS 写入边界不变（只读）。
- **fail-closed 生成路径**：`gate prompt-request`（必须出卡）→ `task compile
  --task-card-requested`（生成器，未被替换）→ `gate output`（校验 canonical，否则
  stop + miss）。任一步失败都不得向前台输出可执行任务卡。

此门禁不改变 AGS 权威边界：分类器是确定性信号源，不是新的授权层；任务级别、
权限模式、Review gate、Verification gate 仍由协议与任务卡决定。

### 3.7. Advisory Intent No-Mutation Gate（咨询意图不写入门禁，deterministic）

咨询、评估、建议类请求不是执行授权。同一确定性分类器
（`prompt-request-classifier`）在命中"你看看是否""是否需要""要不要""建议怎么做"
"评估一下""你觉得""should we""evaluate"等中英文咨询触发词时，标记
`detected_advisory_intent=true` 且 `mutation_allowed=false`，`ags gate
prompt-request` 返回 `decision=advisory_no_mutation`、
`block_reason=advisory_intent_no_mutation`；MCP `ags_solution_check` 暴露
`detected_advisory_intent` / `mutation_allowed` / `advisory_block_reason`。

- **命中时宿主仍可做**：preflight、只读检索、诊断、方案形成、风险说明、澄清问题。
- **命中时宿主不得做**：write-type tool call、格式化、安装依赖、配置修改、任务卡
  生成、实现。
- **解除条件**：只有明确执行授权（"按这个改""开始实现""落地这个方案""去改"
  "implement this""go ahead"等）才把 `mutation_allowed` 置回 `true`。裸"执行"
  不算授权（避免误伤"执行策略/执行器/执行门禁"讨论）。

三类门禁互不替代，必须区分：

| 门禁 | 关注 | 触发 | 阻断 |
|---|---|---|---|
| Advisory intent no-mutation | 意图级 | 咨询触发词 | write-type 操作 |
| Task-card request gate | 形态级 | 任务卡指令 | 非 canonical 可执行输出 |
| Execution permission gate | 策略级 | 任务卡 permission mode | 越权 launch |

Advisory gate 是 Phase 1 入口前置；它不降级也不升级任务级别、权限模式、Review
gate、Verification gate。

### 3.8. Quiet-by-Default 前台输出（可审计不等于过程直播）

AGS 产生大量治理证据（完整 preflight、change lane、verification item、trace、
receipt）。默认前台只暴露单一决策状态 `visible_status`，完整证据进入 trace /
receipt / 任务存档，可定位但不直播。

- **决策状态**：`OK` / `NEEDS_USER_DECISION` / `BLOCKED_BY_POLICY` /
  `RISK_ESCALATED` / `DONE_WITH_RECEIPT` / `ADVISORY_NO_MUTATION`，由确定性
  `derive_visible_status` 按严重度降序推导。MCP `ags_preflight` /
  `ags_solution_check` / `ags_policy_resolve` 响应新增 optional `visible_status`
  字段。
- **quiet 只影响前台展示**：不影响 trace / receipt / archive 写入。静默不等于不
  记录——receipt（`receipt_id` + 收据文件）、verification report、task-archive
  仍完整归档。
- **向后兼容**：`visible_status` 是新增 optional 字段（`skip_serializing_if`），
  既有 JSON 字段全部保留；完整 preflight / policy / verification 明细仍在响应中。

### 3.9. Value Route（效价比路由，advisory）

方案形成后、任务分级路由前，显式选出"仍能覆盖风险的最小执行路径形态"。这是
对 `protocol/task-routing.md`"选择最小可行工作流"思想的结构化，落实全局效价比
原则：单位成本下的最高可靠工程产出。

**Value Route 只塑造执行路径形态，是 advisory 信号。** 它不是第四个任务级别，
不替代也不改变 Light / Medium / Heavy 分级、permission mode、Review gate 或
Verification gate。AGS 协议、task-card validator、execution-policy resolver 和各
gate 仍是权威；planner 拥有最终路径决定权。

**路径形态（canonical 5 种）：**

| 形态 | 含义 | 典型场景 |
|---|---|---|
| `read-only-advisory` | 只诊断 / 回答，不写入 | 咨询意图、未授权 mutation |
| `direct-edit` | 本地有界改动，直接改后验证 | 小范围、低风险、Light |
| `plan-first` | 先 root cause / design / plan，确认后再改 | Medium / Heavy 规划 |
| `claude-code-route` | 框定有界任务卡，handoff 给 Claude Code CLI | 委派执行 |
| `stop-for-scope` | scope / authority / risk 不清 → 停下报告 | 范围或授权不明 |

成本阶梯：`read-only-advisory < direct-edit < plan-first < claude-code-route`；
`stop-for-scope` 是正交逃生口，不由确定性信号自动推荐，由 planner / host 在范围
不清时选择。

**确定性 advisory 暴露。** AGS 在 `ags_solution_check`（MCP）和 `ags gate
prompt-request`（CLI）输出中暴露 `value_route` 块，复用与入口门禁相同的确定性
`prompt-request-classifier` 信号。字段至少包括：`recommended_path`、`rationale`、
`rejected_lighter{path,reason}`、`rejected_heavier{path,reason}`、
`requires_user_confirmation`、`needs_planner_judgment`、`advisory`（恒为 true）、
`authority_note`（固定边界声明）。检测是信号不是授权——与
`detected_task_card_request` 同范式。

**采纳 / 拒绝逻辑（必须记录）：** 为什么不用更轻路径（`rejected_lighter`：更轻会
漏覆盖风险），为什么不用更重路径（`rejected_heavier`：更重是过度投入），为什么
选当前路径（`rationale`）。planner 把最终 value-route 决定写进方案文本或任务卡
`背景` / `实施要求`。Claude Code executor 消费任务卡里已确定的路径，不得用 Value
Route 重写任务级别、permission mode、Review gate 或 Verification gate。

**证据格式**（方案、任务卡或 delivery report 中记录）：

```
Value Route: path=<形态>, confirm=<required|not-required>, rejected_lighter=<形态:原因>, rejected_heavier=<形态:原因>
```

### 3.10. Capability Route（能力路由，advisory）

Value Route 解决"用哪种**执行路径形态**覆盖风险（效价比）"；Capability Route 解决
"针对这个需求，应该**建议宿主显式唤醒哪个被纳管的能力**（skill / MCP / CLI-backed），
以及它当前是否可达"。两者**并列、同为确定性 advisory 信号**，由同一入口暴露，互不替代。

**Capability Route 只给唤醒建议，不是授权层。** 它**不**自动调用任何 skill / MCP / CLI，
**不**阻断或改写用户请求，**不**改变 Light / Medium / Heavy 分级、permission mode、
Review gate、Verification gate 或任务卡 gate。只有 AGS gate 能阻断；Capability Route
永远不阻断。AGS 负责判断、路由、给出显式唤醒建议；是否唤醒由宿主 / 用户决定。

**单一事实源 = manifest。** 路由元数据只来自 `manifests/skills-registry.yaml` 和
`manifests/mcp-registry.yaml` 的 `routing:` 块（`intent_tags` / `scope_tags` /
`mutation_surface` / `requires_auth` / `cost_class` / `invoke_hint` / `route_priority` /
`is_compatibility_alias`）。没有 routing 元数据的能力一律不被路由——不存在内置兜底表。

**parent capability + internal entrypoint。** 真实能力本体（host 可见 / 可登记 / 可 verify 的
`skill` / `mcp` / `cli-backed`）与其内部入口（playbook / MCP tool / CLI subcommand / prompt）分层：
内部入口只在 manifest `route_targets:` 段以 `routing.parent` 声明，是 registry-only route target，
**不进 suite.required、不产生 expected host 缺口、不参与 sync / apply**。Capability Route 可命中具体
内部入口并在输出中展示 `entrypoint`，但 `primary` **必须**解引用到真实 host-visible parent（绝不是内部
入口本身，也绝不是 `capability_group` / `upstream_group`）；入口可用性继承 parent。

**子路由仍是 advisory。** 对 Matt/Superpowers 这类第三方技能族，Capability Route 可先命中
根 demand（如 `matt-superpowers`），再依据每个本名技能自己的 `scope_tags` / `route_priority`
给出 `subroute` 审计块和普通 `recommendations[]`。`subroute` 只展示 family 与已选本名技能，
不产生别名、不自动调用、不改变任何 gate。

**fail-closed 只关乎可达性，不阻断请求。** `auth_status` 在路由时按运行时推导，
**绝不**写入 tracked manifest（`requires_auth` 但无运行时凭据证据 → `required-unknown`，
非 `configured`）。能力非 `available`（canonical 缺失 / 当前 host 不可见 / 需鉴权 /
不健康）时给出 `fallback` 提示并降级为 `degraded`，绝不据此阻断用户。

**真实 active host / target（不无证据硬猜）。** MCP `ags_solution_check` 的路由
host / target 优先取显式 `active_host`(或 `agent`) / `target` 参数，其次取成功
`ags_preflight` 记录的 normalized agent / resolved target；都缺失时走 host-agnostic
保守降级（无正向可用证据），不硬编码假装可用的 host。target 解析能从子目录向上定位
manifest 根。

**确定性 advisory 暴露。** AGS 在 `ags_solution_check`（MCP）和 `ags gate prompt-request`
（CLI）输出中暴露 `capability_route` 块，与 `value_route` 并列。字段至少包括：
`demand_kind`、`matched_demand_triggers`、`active_host`、`recommendations[]`
（`capability_name` / `capability_kind` / `availability` / `auth_status` / `invoke_hint` /
`route_priority` / `is_compatibility_alias` …）、`primary`、可选 `entrypoint`、可选 `subroute`
（`family` / `matched_intent` / `selected_capabilities`）、`status`
（`routed` / `degraded` / `no-demand-detected` / `no-capability-for-demand`）、`fallback`、
`advisory`（恒为 true）、`authority_note`（固定边界声明：只建议显式唤醒、不自动调用、
不阻断、不改任何 AGS gate）。

Claude Code executor 消费任务卡里已确定的能力路径，不得用 Capability Route 重写任务级别、
permission mode、Review gate 或 Verification gate。

**Machine-local enrollment（运行时证据，非 manifest）。** 某能力是否被路由唤醒，取决于本机
enrollment 证据 `<runtime_home>/capability-route/enrollment.json`（`runtime_home` 解析顺序：
`AGS_RUNTIME_HOME` → `AGS_HOME` → `~/.ags/runtime`）。enrollment 模式四档：

| mode | 纳入路由的能力 |
|---|---|
| `off` | 无（所有命中降级为 advisory） |
| `suite-only`（默认） | suite-managed 技能与其 registry-only route target |
| `adopted` | suite-managed 技能 + 被治理第三方 MCP |
| `review-all` | 所有带 routing 元数据的能力（最广） |

证据由 `ags setup --capability-route <mode>`（`--yes` 才写）写入 AGS runtime home，**绝不**写进
tracked manifest，**绝不**含真实凭据。证据缺失 / 损坏 / 未知 mode 一律 **fail-closed 为 off**：
对应能力 availability 记为 `capability-not-enrolled`（advisory degraded），仍**不阻断**用户请求、
不改任何 gate。`auth_status` 仍按运行时推导（`requires_auth` 但无运行时凭据证据 →
`required-unknown`），**绝不**在 tracked manifest 写 `configured`。

**自动别名已退役。** 旧自动别名不再作为 Capability Route primary，也不再作为任务卡
`[skill: ...]` 元数据被接受；其 demand 路由到 canonical 后继（debug →
`diagnosing-bugs`、verify → `verification-before-completion`、brainstorm → `grill-with-docs`）。
任务卡 validator 只接受 `manifests/skills-registry.yaml` 中 `route_state: routable` 且
`invoke_hint` 形如 `[skill: ...]` 的当前技能标记；未知、历史或非 active 标记统一按
`UNKNOWN_OR_INACTIVE_SKILL_TAG` 拒绝。历史采纳/移除事实仍由 `governance/skill-adoption-log.yaml`
append-only 审计记录承接，不再维护前台旧别名映射表。

**任务卡 skill tag 三闸（静态 + 运行期）。** validator 的编译期 `include_str!` 静态门只覆盖
前两闸（registry routable + 合法 invoke_hint）。`[skill: xxx]` 进入任务卡还须过第三闸——
**运行期可用性**：当前机器 capability snapshot 判定该能力对目标 host `available`（enrolled +
canonical present + auth 满足 + host-visible + healthy）。`degraded` / `auth-required` /
`not-visible` / `not-enrolled` / `unmanaged` / not-routable 一律拒绝。运行期闸由
`ags gate skill-tags --for <host> <card>`（消费机器本地快照，决策 stop/allow，确定性 fail-closed）
执行，且**已接入任务卡执行主链路**：`ags run`（`scripts/run-task-card.sh` 委托的 canonical
runner）在 launch-plan 阶段自动对任务卡尾部 `[skill: …]` 跑等价检查，按 runtime adapter
推导 host（`claude-code` / `codex` / `cursor`；`generic` → host-agnostic fail-closed），任一
tag 不可用即 `gate_decision=stop`（`gate_error_kind=skill_tags_unavailable`）、不出 launch
args、不生成 receipt。`check-only` 仍只过离线 policy 闸、不触发运行期 skill 闸，保证 validator
静态确定性不变。registry 仍是"是否允许路由"的静态权威，运行期发现/快照永不把 not-routable 提权为 routable。
被发现的系统/用户技能（`host-system`/`discovered`/`project-local`）默认 not-routable，必须先在
registry 显式纳管（`route_state: routable`，系统技能用 `source.type: host-system` 不写机器路径）
才可被任务卡显式调用。详见 `protocol/skill-governance.md` 的"动态全机能力路由"章。

**doctor / update verify drift（只读）。** `ags doctor` 与 `ags update verify` 含 capability route
drift 检查，覆盖：manifest routing metadata（auto-* 别名标注）、runtime enrollment（present + mode）、
auth-evidence 边界、host visibility（只读不深探，host 可见性权威是 `ags skill verify --host`）。其中
**auth-evidence 边界**是唯一阻断项：tracked manifest 出现 credential key 或 `auth_status: configured`
即 **FAIL**；enrollment 缺失仅 warn/info（advisory degraded），不让 `--strict` 严格失败。新检查只读，
写 AGS-owned runtime 文件只能由 `ags setup --yes` 或 `ags update repair-local --apply` 完成并出 receipt。

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
- Heavy plan（Permission mode: plan-only）：Executor 只读产出 root cause / design / implementation plan / verification plan，待人工批准后再进入修改阶段
- Heavy execute（Permission mode: execute-and-verify）：按任务卡直接执行并验证，不再追加 mutation 前的确认环节；完成后仍标为"部分完成 / 待人工 adversarial review"

**任务级别与 Permission mode 解耦。** 任务级别（Light/Medium/Heavy）是**风险/审查**等级；Permission mode（plan-only / edit-with-confirmation / execute-and-verify）是**当前执行权限**，也是唯一的执行授权。任务级别不改写 Permission mode：

- **Heavy ≠ plan-only**：Heavy 卡保留其声明的 Permission mode，只额外获得 review gate；resolver 不因级别是 Heavy 就降级可执行卡。只要 Permission mode 不是 read-only / plan-only，卡就进入可执行链路，`execute-and-verify` 也不被 cap 到 `edit-with-confirmation`。confirmation 由 Permission mode 决定：`edit-with-confirmation` 在每次改动前暂停确认，`execute-and-verify` 直接执行并验证，级别本身不追加 mutation 确认。
- **未声明 Permission mode 的默认**：当 Heavy 卡未显式声明 Permission mode 时，compiler 填入保守默认 `plan-only`（这是对未声明字段的默认值，不是级别降级；**显式声明**的 Permission mode 一律保留）。
- **approval 信号只是审计/提示**：`current-task-approval`（`实现 / 修复 / 做完` 等，由 `prompt-request-classifier` 确定性识别，**不**取自任务卡文本）与 `--approve-writes` 经 `ApprovalSource` **可审计地**传递，但**不再**是 Heavy 执行解锁条件；`--approve-writes` 仍可作为 generic adapter（M9）能力上限的 override。任务卡自由文本永远不是 approval 来源。
- **plan-only 收敛流程仅按需使用**：只有当卡本身是 plan-only（显式声明或未声明时的默认）时，Executor 才先出 root cause / design / implementation plan / verification plan 并等待人工确认；该收敛流程用于高危发布、迁移、破坏性动作，不是所有 Heavy 卡的强制前置。
- **安全边界不变**：stable/public 发布、外部写入（Lark/邮件/审批）、删除 runtime、credential/auth、删除/迁移/不可逆操作仍各自单独确认或停止；read-only / plan-only 仍不得产生 write-type launch args；可执行 Heavy 卡仍必须走独立 review gate（confirmation 仅由 `edit-with-confirmation` permission mode 触发，不由级别触发）。

任务卡骨架仍只有唯一 canonical card，不因解耦引入第二模板。任务卡中声明的级别优先于 Executor 自行判断。如 Executor 发现实际风险高于任务卡标注的级别，必须停止并报告，不得自行降级执行。

## Resume / 压缩恢复保护

遇到"继续"、上下文压缩恢复、task-notification 接续或后台任务恢复时：

- 如果任务级别是 Heavy，Executor 必须重新读取任务卡、运行 `git status --short`，并重新确认 `review_targets`。
- Executor 必须按当前可确认任务卡的 `Permission mode` 行动：`plan-only` 等待明确批准；`edit-with-confirmation` 停在该 permission mode 的确认提示；`execute-and-verify` 可恢复执行并验证。
- 不得把"继续"、恢复通知、上一轮计划或压缩摘要理解为新的 Heavy 写入批准；它们只能触发重新读取任务卡与复核目标。
- 如果无法确认当前任务卡、目标仓库、`Permission mode` 或 review 状态，停止并报告，不得继续执行。

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
- 不得接管外部官方 CLI 或项目自管输出层技能；`notebooklm` 和项目自管业务契约只能引用，不能通过套件 adopt / update / 打包
- 项目自管输出层产物（如 `notebooklm_task_card`、`local_context_pack`、`fairness_check_questions`）是业务运行时契约，不得改写为开发套件任务卡或技能治理对象

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
| Light | 完成验证后运行 `requesting-code-review` 或等价轻量 diff review；如发现风险高于 Light，升级 Medium。 |
| Medium | Codex 最终 Review gate；Executor 完成验证后将任务状态标为"部分完成 / 等待 Codex review"，由 Codex 审查通过后再放行。 |
| Heavy | 人工 Adversarial Review gate；Executor 完成验证后将任务状态标为"部分完成 / 等待人工 adversarial review"，并提醒操作者手动运行 `/codex:adversarial-review` 后再放行。Heavy plan 与 Heavy execute 完成后都必须经此人工 adversarial review。 |

Executor 完成交付前必须报告 review gate 状态；Medium / Heavy 在对应人工 review 完成前只能标为"部分完成"。Light 若在 `requesting-code-review` 或等价轻量 diff review 中发现跨文件协议、权限、hook、数据写入、路径迁移或生成物同步风险，必须升级为 Medium 并等待 Codex review。

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

Executor 完成后必须输出以下格式的交付报告。前台最终输出必须是一个可整体复制的
Markdown 代码块；外层使用四反引号 `markdown` fence，外层代码块前后不要
添加正文。

````markdown
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
````

## 技能标记

任务卡末尾可包含 `[skill: xxx]` 标记。Executor 收到标记后，按 `protocol/cursor-skill-index.md` 打开对应 SKILL.md 并按其指引执行。

常用标记：`[skill: test-driven-development]`、`[skill: diagnosing-bugs]`、`[skill: review]`、`[skill: verification-before-completion]`。

## 任务卡模板

Cursor / Codex 使用 `protocol/task-card-template.md` 生成任务卡。任务卡的输入必须是已确认的方案或 execution contract，不能是原始用户自然语言请求。

**规则进协议，差异进任务卡。** 固定规则写在本文件，单次任务差异写在任务卡中。

### 任务卡形态边界

任务卡只有唯一形态：`protocol/task-card-template.md` 定义的 canonical 固定骨架。

- 项目协议可访问时，生成该骨架并引用本协议文件，不重复固定规则。
- 跨仓库、外部 agent、或 Executor 无法访问本项目文件时，仍使用同一骨架，
  把所需固定规则内联进去使其自包含（self-contained canonical card）。这是同一
  骨架的交付形态，不是第二套模板。任务级别 Light / Medium / Heavy 只是
  `任务级别：` 字段值，不决定模板文件。

“完整”“压缩”“可粘贴”“可复制”“compact”“full”都不是任务卡形态，
只能表达前台展示偏好。compact 任务卡格式已删除：任务卡只有唯一经典固定
骨架，这些词不得改变任务卡骨架、标题、槽位顺序，不得据此生成 compact
骨架或“默认 compact 可执行卡”，也不得在该唯一骨架之外创造第二/第三种格式。

自由 runbook、临时 prompt、阶段性执行简报、header-only runtime block、
target-first task brief、文档式任务说明都不得称为任务卡。需要执行时，
必须重新编译为该 canonical 任务卡骨架。

对话前台输出任务卡时，默认使用普通 Markdown，让客户端自然换行；只有
用户明确要求单个 literal copy block、文件 artifact，或任务卡内含嵌套
代码块且必须整体复制时，才用外层 fenced block。

## 何时使用自包含 Prompt

只有以下情况才需要把完整协议规则内联进自包含 canonical 任务卡：

- 跨仓库执行（Executor 工作目录不是本仓库）
- 外部 agent 执行（没有本项目上下文）
- Executor 无法访问本仓库文件

其他情况一律使用 canonical 任务卡 + 引用本协议。内联固定规则不改变骨架，
只是把被引用规则写进同一骨架，不另立 fallback 形态。
