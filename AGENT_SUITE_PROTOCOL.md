# Agent Suite Protocol

本文件是 Agent Governance Suite 公开版协议概述。Canonical 协议文件位于本仓库
`protocol/` 目录下，自包含，不依赖私有基础设施或私有仓库。

## 本仓角色

此仓库是 Agent Governance Suite 公开可分发版本，提供 Rust 原生 CLI 工具链 (`ags`)，包含：

- `ags task validate`（别名：`task-card-validator`） — 任务卡格式与语义校验
- `ags policy resolve`（别名：`resolve-policy`） — 执行策略解析
- `ags policy explain` — 逐条输出策略规则解释、rule IDs、安全断言
- `ags policy check` — 校验 + 解析，按 gate 结果 exit
- `ags sync check`（别名：`workflow-sync-check`） — 协议漂移检查
- `ags doctor`（别名：`suite-doctor`） — 套件健康诊断
- `ags bootstrap --dry-run`（别名：`bootstrap-dry-run`） — 引导干运行模拟
- `ags project detect` / `ags protocol status` / `ags agent instructions` — M2 Agent 感知能力（只读）
- `ags session preflight --for codex|claude-code|cursor` — 聚合 Agent 唤醒检查（kernel activation 入口，不依赖 skill governance）
- `ags verify --scope local|full|release` — 结构化验证入口，提供稳定 CheckItem 模型和 text/json 双格式报告

AGS 定位为开发相关工作中的**常驻工程中枢**，不是需要用户单独唤出的 CLI 工具箱。
开发请求到达时，AGS 治理自动接入：ambient preflight → solution formation →
user confirmation ("方案 OK") → user task-card instruction ("生成任务卡") →
execution contract → task routing → gate / execution / receipt。不得从原始用户
请求直接跳到 Light / Medium / Heavy 分级。"方案 OK" 只结束方案阶段，必须等用户
明确发出任务卡指令后才进入路由；`ags task compile` 以 `--task-card-requested`
强制执行此门槛。

## 协议入口

Canonical 协议文件位于本仓库：

- `AGENT_SUITE_PROTOCOL.md` — 套件级协议概述（本文件）
- `protocol/agent-task-protocol.md` — 任务卡与 review 规则（含完整生命周期：ambient preflight → solution → execution contract → routing → gate / execution / receipt）
- `protocol/task-card-template.md` — 固定任务卡骨架（输入：已确认的 execution contract）
- `protocol/runtime-adapters.md` — 执行器/权限/review/resume 规则（仅在任务卡形成后生效）
- `protocol/task-routing.md` — light/medium/heavy 路由（方案确认后执行，不前置分级）
- `protocol/skill-governance.md` — 技能治理协议（推荐/说明/只读边界）

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
- 比较不同目标之间的协议文件漂移
- 验证关键协议安全断言在目标中完整存在
- 区分 legal redaction（allowlist）和 dangerous drift
- 输出结构化 text/JSON drift report

workflow-sync-check 是 **read-only drift checker**，不决定任务是否进入 plan-only，
不替代或影响 execution-policy / resolve-policy 的执行决策。

## Public-Full Sanitized Boundary

公开版是 **public-full-sanitized**：保留 AGS 满血核心能力、项目入口文件、规则、
记忆胶囊机制、任务存档机制和第三方技能治理框架；只清除私有数据和本机运行状态。

公开版应包含：

- Rust `ags` workspace（`Cargo.toml`、`Cargo.lock`、`crates/`）和核心命令面；
- `AGENTS.md`、`CLAUDE.md`、`WORKSPACE.md`、`AGENT_SUITE_PROTOCOL.md`；
- `protocol/`、`templates/`、`scripts/`、公开 docs、manifest 和治理规范；
- 空白记忆模板：`templates/memory/context-capsule.md`、`task-memory.md`、
  `archive-index.md`、`task-archive/README.md`；
- 空白治理审计骨架：`governance/skill-adoption-log.yaml`、
  `governance/skill-ignore-list.yaml`；
- 确认式技能治理和安装能力：scan/check/propose/install/adopt/ignore 等命令必须
  遵守 dry-run、人工确认、不得静默覆盖用户目录的门禁。

公开版不得包含：

- `target/`、release/debug `ags` 二进制、构建缓存或临时日志；
- 用户真实记忆、真实任务归档、真实 receipt、真实交付报告；
- 已安装第三方技能、本地技能包、`global-skills/`、`skill-packs/`；
- `$HOME/.agents`、`$HOME/.codex`、`.claude/local/` 等本机配置状态；
- 私有路径、用户名、私有仓库名、密钥、token 或公司/个人敏感上下文。

## Skill Governance

Agent Governance Suite 在公开版中提供完整的技能治理框架，但不预装第三方技能或
用户本地技能。`protocol/skill-governance.md` 定义推荐、扫描、检查、提案、确认安装、
审计记录和回滚边界。公开版用户如需安装第三方开发技能，可以使用
`ags skill install --skill <name> --confirm` 或参考 `docs/skill-recommendations.md`。
所有第三方技能必须由用户自行选择可信来源并显式确认安装。

## Protocol Safety Assertions

workflow-sync-check 强制执行以下关键协议安全断言。缺失或矛盾改写始终为 FAIL，
即使在 public 目标上也不能被 allowlist 掩盖：

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
