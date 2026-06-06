# Agent Suite Protocol

本文件是 Agent Governance Suite 协议概述。**Canonical 协议权威版本**
位于开发版私有套件仓：

```
/Volumes/AI Project/agent-governance-suite-private/AGENT_SUITE_PROTOCOL.md
```

## 本仓角色

此仓库 (`agent-governance-suite-private`) 是 Agent Governance Suite 开发版私有主库，提供
Rust 原生 CLI 工具链 (`ags`)，包含：

- `ags task validate`（别名：`task-card-validator`） — 任务卡格式与语义校验（首个稳定节点）
- `ags sync check`（别名：`workflow-sync-check`） — 工作流协议漂移与安全断言检查
- `ags doctor`（别名：`suite-doctor`） — 套件健康诊断
- `ags bootstrap --dry-run`（别名：`bootstrap-dry-run`） — 引导干运行模拟检查
- `ags policy resolve`（别名：`resolve-policy`） — 执行策略解析器
- `ags project detect` — 检测项目身份与 AGS 集成状态（M2）
- `ags protocol status` — 检查协议文件状态与治理需求（M2）
- `ags agent instructions` — 导出 Agent 专属项目说明（M2）
- `ags session preflight` — 聚合 Agent 唤醒检查（M2 kernel activation 入口）
- `ags verify --scope local|full|release` — 结构化验证入口，提供稳定 CheckItem 模型和 text/json 双格式报告

AGS 定位为开发相关工作中的**常驻工程中枢**，不是需要用户单独唤出的 CLI 工具箱。
开发请求到达时，AGS 治理自动接入：ambient preflight → solution formation →
user confirmation (\"方案 OK\") → user task-card instruction (\"生成任务卡\") →
execution contract → task routing → gate / execution / receipt。不得从原始用户
请求直接跳到 Light / Medium / Heavy 分级。\"方案 OK\" 只结束方案阶段，必须等用户
明确发出任务卡指令后才进入路由；`ags task compile` 以 `--task-card-requested`
强制执行此门槛。

## 协议入口

Canonical 协议文件位于本仓库：

- `AGENT_SUITE_PROTOCOL.md` — 套件级协议概述
- `protocol/agent-task-protocol.md` — 任务卡与 review 规则（含完整生命周期：ambient preflight → solution → execution contract → routing → gate / execution / receipt）
- `protocol/task-card-template.md` — 固定任务卡骨架（输入：已确认的 execution contract）
- `protocol/runtime-adapters.md` — 执行器/权限/review/resume 规则（仅在任务卡形成后生效）
- `protocol/task-routing.md` — light/medium/heavy 路由（方案确认后执行，不前置分级）
- `protocol/skill-governance.md` — 技能治理总协议（source of truth、候选来源层级、治理生命周期、写入规则）
- `governance/skill-sync.md` — 技能同步阶段边界（scan/candidate/proposal/adopt/ignore/manifest/backup/rollback）

## Task Card Validation

Rust task-card-validator (`crates/task-card-validator`) 是唯一的 canonical
任务卡格式门禁。它提供格式校验、字段值检查、字段组合检查、保护路径分析、矛盾检测和
Execution Authority Gate。

## Execution-Policy Resolver

`crates/execution-policy` 是 runner 前的策略解析层。它消费 validator 输出的结构化字段，
产出 `ResolvedExecutionPolicy` — 包含实际应使用的 permission mode、parallelism、
启动参数、降级原因和停止条件。resolver 只读，不启动 runner；`ags policy resolve`
提供主 CLI 入口，旧 `ags resolve-policy` 仅作为隐藏兼容别名保留。
解析规则（M1–M10）写入 `protocol/runtime-adapters.md`。

## Workflow Sync Check

`crates/workflow-sync-check` 是多目标协议漂移检查器，负责：
- 比较 A 私有主库 ↔ S stable ↔ public/core-only 之间的协议文件漂移
- 验证关键协议安全断言在 stable 和 public/core-only 中完整存在
- 区分 legal redaction（allowlist）和 dangerous drift
- 输出结构化 text/JSON drift report

workflow-sync-check 是 **read-only drift checker**，不决定任务是否进入 plan-only，
不替代或影响 execution-policy / resolve-policy 的执行决策。

## Public/Core-Only 发布边界

稳定版推公开版时，只推公开可分发内容：公开协议、任务卡模板、项目集成模板、安装脚本、
基础校验脚本、公开文档、license/changelog、公开 manifest/capability 元数据，以及
公开发布清单明确包含的 public-safe 规则和技能。

Rust `ags` 工具链是私有版治理工具，不是公开版 payload。公开版不得携带 `Cargo.toml`、
`Cargo.lock`、`crates/`、`target/`、release/debug `ags` 二进制，或私有诊断/同步工具实现。
私有版可以用这些工具检查 public/core-only，但 public/core-only 不发布这些工具。

### Public/Core-Only Skill Allowlist

公开版技能分发遵循最小 allowlist 原则：

- **允许携带**：public-safe 技能（经过安全审查、无敏感路径依赖、license 兼容），且必须在
  public release manifest 中显式列出
- **禁止携带**：
  - 私有 Rust 治理工具链（`Cargo.toml`、`Cargo.lock`、`crates/`、`target/`、`ags` binaries）
  - 私有诊断/同步工具实现（`crates/workflow-sync-check/`、`crates/suite-doctor/` 等）
  - 私有运行层实现（runner adapter、hook installer、capture scripts 的非公开部分）
  - 非 public-safe 技能（含内网 URL、私有仓库引用、未审查网络请求、`curl | bash` 模式）
  - 个人 profile 技能（`manifests/suite.yaml` 中 `personal` 段的内容）
- **技能治理文件**：`governance/skill-adoption-log.yaml` 和 `governance/skill-ignore-list.yaml`
  是私有治理审计日志，不属于 public payload。公开版只携带 `manifests/suite.yaml` 中
  allowlist 的 public-safe 技能及其 manifest entry
- **边界检查**：`ags sync check` 在 public/core-only 目标上检查 `PUBLIC_FORBIDDEN_PAYLOAD`
  时，会标记不应出现在公开版的私有技能和治理文件

## Skill Governance

Agent Governance Suite 的技能治理子系统由以下文件组成：

- `protocol/skill-governance.md` — 技能治理总协议：source of truth 声明、候选来源层级
  （GitHub/插件/CLI/拖拽目录仅作为候选）、治理生命周期（discover → scan → proposal →
  dry-run → apply → verify）、写入规则硬门禁（dry-run 先行、diff before apply、人工确认、
  禁止静默覆盖用户目录、禁止接管外部 CLI）
- `governance/skill-sync.md` — 同步阶段边界定义：scan、candidate check、proposal、
  adopt、ignore、manifest、backup、rollback 各阶段的输入/输出/门禁，以及 Phase 2
  Rust read-only inventory 的字段契约
- `governance/skill-adoption-log.yaml` — 已接纳技能 append-only 审计日志
- `governance/skill-ignore-list.yaml` — 已拒绝/忽略技能审计日志（append-only 或显式
  supersede，禁止静默删除历史记录）
- `manifests/suite.yaml` — 套件 manifest，区分 required / optional / personal profile

**当前状态：Phase 1。** 协议和 schema 骨架已落地，不实现 Rust CLI。Phase 2 将基于
这些 schema 实现 `ags skill scan|check` 只读 inventory，以及后续的 proposal/adopt CLI。

技能治理遵循 AGS 标准生命周期。涉及技能同步、adoption、ignore 的任务必须使用
`protocol/task-card-template.md` 的固定骨架生成任务卡，并遵守 `protocol/skill-governance.md`
的写入规则。

## Protocol Safety Assertions

workflow-sync-check 强制执行以下关键协议安全断言。缺失或矛盾改写始终为 FAIL，
即使在 public/core-only 目标上也不能被 allowlist 掩盖：

1. **ultracode thinking-only**: `Execution effort: ultracode` 只是 thinking intensity，
   不改变 permission mode、不启用 parallelism、不添加 launch args。
2. **Heavy downgrade**: Heavy 任务无 explicit write approval 必须降级到 plan-only
   并要求 confirmation gate。
3. **read-only/plan-only no-write**: read-only 和 plan-only 不得产生 write-type launch args，
   active parallelism 和 headless/background-agent 必须被 strip 或 stop。
4. **runner resolver-first**: runner 必须消费 `ags policy resolve --format json` 输出的
   `effective_*` / `allowed_launch_args`，不得从原始任务卡字段直接拼接执行参数。

## M2 Agent Awareness (Project Discovery)

M2 提供只读命令，让 Agent 和操作者无需查询任务卡即可了解项目身份、协议状态和专属指令：

```bash
# 检测项目身份与 AGS 集成状态
ags project detect
ags project detect --target /path/to/repo --format json

# 检查协议文件状态、校验器入口、风险边界和 review/verify/receipt 要求
ags protocol status
ags protocol status --target /path/to/repo --format json

# 导出 Agent 专属项目说明
ags agent instructions --for codex
ags agent instructions --for claude-code
ags agent instructions --for cursor

# Kernel activation — 聚合唤醒检查（组合以上所有能力）
ags session preflight --for codex
ags session preflight --for claude-code --format json
ags session preflight --for cursor --target /path/to/repo
```

`ags session preflight` 是默认 kernel activation 唤醒入口。它将 project detect、
protocol status、agent instructions 聚合为单一只读报告，包含 memory capsule/task-memory
路径、stop conditions、warnings、failures 和下一步建议。它不依赖 skill governance —
核心 kernel activation 独立于第三方 skill governance。

所有 M2 命令均为只读；不安装 hook、不启动 runner、不执行任务。exit code：0 = suite/integrated/clean，1 = partial/not-integrated/failures，2 = 参数错误。

## 技能标记

任务卡末尾可包含 `[skill: xxx]` 标记。常用：`[skill: tdd]`、`[skill: diagnose]`、
`[skill: review]`、`[skill: verify]`、`[skill: zoom-out]`。
