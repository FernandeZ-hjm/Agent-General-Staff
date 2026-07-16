# Agent Task Protocol

> AGS 0.2.8 的权威任务协议。自然语言只在一个需求路由节点解释一次。

## 完整生命周期

### 1. Ambient Preflight（环境预检）

宿主先调用 MCP `ags_preflight`；MCP 不可用时才使用：

```bash
ags session preflight --for <agent> --target <path>
```

预检只建立项目身份、协议状态、上下文与 stop condition，不做技能选择，也不做任务分级。

### 2. Request Router（唯一需求路由）

调用链固定为：

```text
人类需求 → 宿主对话上下文 → AGS MCP → Request Router → RequestDecision
```

宿主（Codex、Claude Code 等）负责携带当前对话上下文。AGS 路由器无状态，不累计轮次，不设置“继续对话”的超时或轮数上限。信息不足时返回缺失字段；下一轮由宿主带完整上下文重新调用。

`RequestDecision` 的结构化结果：

```text
status: Ready | InsufficientContext { missing }
targets:
  DirectResponse
  Skill { demand: SkillDemand }
  MachineCli { capability: CliCapabilityId, input: TypedCliInput }
```

合法组合矩阵：

| 组合 | 合法 | 含义 |
|---|---:|---|
| `DirectResponse` | 是 | 纯对话或有界内容加工 |
| `Skill` | 是 | 只需要一种方法/领域能力 |
| `MachineCli` | 是 | 只需要机器执行 |
| `Skill + MachineCli` | 是 | 方法能力与机器执行是同一请求的互补目标 |
| `DirectResponse + Skill` | 否 | 直接回复是终止目标，不附带隐式技能执行 |
| `DirectResponse + MachineCli` | 否 | 回复与机器执行语义冲突 |

附加约束：

- 每个决定最多一个 `Skill` 和一个 `MachineCli`。
- `DirectResponse` 必须独占。
- 普通压缩、翻译、格式转换、字段统一、已批准结构执行直接返回 `DirectResponse`。
- 只有明确的大型设计信号（新系统架构、跨模块架构、架构边界、跨 MCP/CLI/Vault 等）才返回系统架构类 `SkillDemand`。
- 现有 `## 任务卡` 首行输入返回 `MachineCli::TaskExecute`，不得重新进入方案形成。
- 明确任务卡交接请求只有在 `confirmed_handoff_contract=true` 时返回 `TaskCompile`；否则返回 `InsufficientContext`。

### 3. Solution Formation（按需）

`RequestDecision` 需要技能或仍缺少设计边界时，宿主才进入相应方案工作。已经批准的执行合同、有界转换、现有任务卡不得仅因出现“架构”“实现”等词再次进入 brainstorming 或 writing-plans。


### 4. Skill Resolution（确定性技能映射）

Skill Resolver 不读取自然语言，只消费闭集 `SkillDemand`：

```text
SkillDemand + ActiveSkillTable → SkillSelection
```

映射来源是 `manifests/skills-registry.yaml` 的 `demand_routes`。一个 demand 必须且只能映射一次。技能不可用时返回治理前置条件失败，不自动选择职责相似的替代技能，也不返回可自动执行的 alternatives。

`ActiveSkillTable` 是以下集合的严格交集：

```text
kind=Skill
∩ canonical_present
∩ health=healthy
∩ route_state=routable
∩ active_host=visible
```

机器快照使用 registry hash、runtime inventory hash、active-table hash 和 snapshot hash 校验。缺失、篡改或 stale 时，Skill 目标停止并提示：

```bash
ags capability snapshot --host <host> --write
```

`DirectResponse` 与纯 `MachineCli` 不依赖技能快照。

### 5. Machine CLI（机器能力面）

Router 返回业务语义级 `CliCapabilityId`，不是 CLI 子命令编排：

- `TaskCompile`
- `TaskExecute`
- `TaskValidate`
- `PolicyResolve`
- `ProjectVerify`
- `SkillTagsVerify`
- `ReceiptVerify`

每个决定最多一个 Machine CLI 目标。`TaskExecute` 内部的 validate → policy → gate → runner → verify → receipt 由 CLI 自己完成，Router 不编排子命令。

MCP 只通过固定 capability-to-argv 映射调用真实 `ags` 可执行文件；禁止 shell、禁止拼接任意命令、禁止在 MCP 内复制 Compiler/Policy/Runner 实现。

### 6. Task-Card Handoff Gate（任务卡交接门槛）

任务卡生成必须同时具备：

1. 明确任务卡/交接请求；
2. 已确认且封闭的 handoff contract。

`ags task compile` 只接受结构化门槛：

```bash
ags task compile <contract> \
  --task-card-requested \
  --confirmed-handoff-contract
```

Compiler 不读取自然语言，不推断技能，不重新判断方案是否充分，只验证并编译结构化 contract。

### 7. Gate / Execution / Receipt

- 现有任务卡：validate → policy resolve → gate → runner。
- 任务卡 permission mode 只有 `plan-only` 与 `execute-and-verify`。
- Light / Medium 默认直接执行；Heavy 默认 `plan-only`，明确授权的 Heavy `execute-and-verify` 可直接执行，但增加独立 review gate。
- destructive、external-write、credential、migration、release 等边界独立停止。
- 完成前执行与风险匹配的验证；写入型动作产出 machine-readable receipt。

## 角色

- 人类：提出需求、补充上下文、批准方案或授权交接。
- 宿主 Agent：保存对话上下文，调用 AGS MCP，消费 `RequestDecision`。
- Request Router：唯一自然语言解释节点。
- Skill Resolver：闭集 demand 到可用技能的确定性映射。
- AGS MCP：宿主适配、结构化消费、固定 CLI 调用。
- AGS CLI：机器能力面与内部工作流编排。
- Compiler：结构化 contract 验证与任务卡编译。
- Policy/Gate/Runner：只消费结构化输入，不读取原始自然语言。

## Skill Governance 治理

任务卡 `[skill: name]` 必须同时满足：registry `routable`、合法 `invoke_hint`、当前 ActiveSkillTable 可用。宿主只发现父技能；父技能内部 playbook 不作为独立任务卡 skill tag。

## 验证规则

声称完成前至少验证：

1. 目标 crate 测试；
2. `cargo test --workspace`；
3. `ags verify --scope full`；
4. 路由正反例与旧入口禁用断言；
5. CodeGraph 索引同步。

## 交付报告

交付报告包含：状态、改动边界、验证结果、剩余风险和未执行的外部动作。不得把“计划完成”写成“代码完成”。

## 任务卡模板

权威模板为 `protocol/task-card-template.md`。原始聊天不是任务卡；只有通过双门槛编译出的结构化交接产物才是任务卡。
