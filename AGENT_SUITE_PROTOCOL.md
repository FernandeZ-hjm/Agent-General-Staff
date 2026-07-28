# Agent Suite Protocol

本文件是 Agent General Staff 公开版治理控制面协议概述。Canonical 协议文件位于本仓库
`protocol/` 目录下，自包含，不依赖私有基础设施或私有仓库。

Current product version: **0.3.6**.

这是 Agent General Staff Public Edition 的当前 latest 产品版本。AGS 负责准入、
授权、策略、验证、回执、能力快照和记忆闭环，不提供任务队列、Agent 调度器、
并行执行器或多 Agent 协商。协议中的
`2.0-*` schema 标识和下文历史 `2.x` release 标题属于独立的 wire/history
版本面，不代表当前产品版本。

## 本仓角色

此仓库是 Agent General Staff 公开可分发版本，提供 Rust 原生 CLI 工具链 (`ags`)，包含：

- `ags task validate` — 任务卡格式与语义校验
- `ags policy resolve` — 执行策略解析
- `ags policy explain` — 逐条输出策略规则解释、rule IDs、安全断言
- `ags policy check` — 校验 + 解析，按 gate 结果 exit
- `ags doctor` — 套件健康诊断
- `ags setup` — 写入公开安全的本机 AGS runtime、MCP 片段、Claude `/ags` 入口和 Codex AGS 命令技能
- `ags init` — 对用户项目执行 AGS managed-block 接入
- `ags mcp serve --transport stdio` — 启动公开版 AGS MCP 服务
- `ags mcp status` / `ags mcp restart` — 查询或重启当前工作区服务
- `ags host lifecycle` / `ags memory` — 统一四宿主的 Rust 记忆闭环
- `ags bootstrap --dry-run` — 引导干运行模拟
- `ags project detect` / `ags protocol status` / `ags agent instructions` — M2 Agent 感知能力（只读）
- `ags project integrate --dry-run|--confirm` — 增量融合 AGS 托管入口块到用户项目入口文件，不覆盖用户自有内容
- `ags session preflight --for codex|claude-code|cursor|omp` — 聚合 Agent 唤醒检查（CLI 降级/独立检查入口，不依赖 skill governance）
- `ags verify --scope local|release|promotion` — 结构化验证入口；`release`
  自包含验证公开源码树，`promotion` 只在显式提供 public worktree 时验证 A→B
  边界，二者都提供稳定 CheckItem 模型和 text/json 双格式报告

AGS 定位为开发相关工作中的**常驻工程中枢**，不是需要用户单独唤出的 CLI 工具箱。
公开版的用户入口包含 Claude Code `/ags`、Codex `$ags-setup` / `$ags-init` /
`$ags-skill` / `$ags-doctor` 命令技能，以及 `ags mcp serve` 提供的 MCP 内核桥。
凡是 AGS 相关任务，都必须优先通过 AGS MCP 显式调用 `ags_preflight`；CLI 预检只作为
MCP 不可用时的降级路径。
`ags mcp serve` 是薄 stdio adapter，连接或启动按工作区 canonical path 唯一键控的
AGS daemon。host 不参与实例键；Codex、Claude Code、Cursor 与 OMP 是同一工作区
服务的不同客户端。每个客户端拥有独立 `session_id`、preflight binding 和
DecisionLease，能力快照由 daemon 在生命周期内单点缓存且保持不变。断开客户端不等于关闭
daemon；升级必须先停止旧实例，再启动新二进制。
开发请求到达时，宿主在 preflight 后读取当前宿主能力薄目录，结合完整对话形成
typed `HostRouteProposal` 并交给严格只读的 `ags_route_request`。自然语言语义判断只在
宿主发生；Skill Resolver 只按 snapshot 精确校验 skill/entrypoint，不做关键词或
fallback。只有 `ags_apply_action` 能以一次性 lease/action 引用消费固定机器动作。
不得从原始用户请求直接跳到 Light / Medium / Heavy
分级。"方案 OK" 只确认方案，不授权写入；
用户明确授权同会话修改时进入 `direct-edit`，明确要求任务卡或跨 Agent 交接时才进入
task-card handoff。宿主 Plan mode 在方案闭合后以
`--host-plan-mode-final --confirmed-handoff-contract` 直接把最终产物写成唯一
canonical 任务卡；用户批准后退出 Plan mode 并派发同一张卡，不再生成第二份。
普通显式交接使用 `ags task compile --task-card-requested
--confirmed-handoff-contract`。这些门槛只约束任务卡生成，不是所有
本地执行的前置门槛。

首个非空行已经是 `## 任务卡` 的输入属于 existing task card。入口必须先校验：
合法卡直接进入 policy / gate / LaunchPlan，非法卡 fail closed；两者都不得回落到新任务卡生成。Runner 返回 `HOST_EXECUTION_REQUIRED`，不声称已经执行或验证。
任务卡权限由 `Execution mode`、`Execution topology` 和
`Delegation planning` 三个显式字段共同决定。权限只允许在 LaunchPlan 和
Delivery Report 中向下收缩；Heavy 只追加独立 review gate，不重写权限。
破坏性、外部写入和发布仍走各自独立 stop 条件。

## 协议入口

Canonical 协议文件位于本仓库：

- `AGENT_SUITE_PROTOCOL.md` — 套件级协议概述（本文件）
- `protocol/agent-task-protocol.md` — 执行决策、任务卡与 review 规则（含完整生命周期：ambient preflight → solution → user decision → direct-edit 或 task-card handoff → gate / verification / receipt）
- `protocol/task-card-template.md` — handoff 路径的固定任务卡骨架（输入：已确认方案）
- `protocol/runtime-adapters.md` — 执行器/权限/review/resume 规则（仅在任务卡形成后生效）
- `protocol/task-routing.md` — light/medium/heavy 路由（方案确认后执行，不前置分级）
- `protocol/skill-governance.md` — 技能治理协议（推荐/说明/只读边界）
- `protocol/project-profile.md` — 项目画像协议（用户项目集成后自行生长）
- `protocol/context-memory.md` — 上下文记忆协议（公开版只发布协议和空白模板）
- `protocol/cursor-skill-index.md` — Cursor / skill routing 索引
- `protocol/mcp-server.md` — AGS MCP host initialization adapter 协议
- `manifests/suite.yaml` — 公开版 suite manifest
- `manifests/skills-registry.yaml` — governed skill registry + routing metadata
- `manifests/mcp-registry.yaml` — governed MCP registry

第一方治理、验证、发布和生命周期逻辑全部由 Rust `ags` 内核提供。
`scripts/` 只保留 OMP 必须加载的薄 JS 事件适配器；它不解析任务卡、不计算
权限、不校验 hash，也不生成 receipt。

## Task Card Validation

Rust task-card validator (`crates/ags-task-contract`) 是唯一的 canonical
任务卡格式门禁。它提供格式校验、字段值检查、字段组合检查、保护路径分析、矛盾检测和
Execution Authority Gate。

## Execution-Policy Resolver

`crates/ags-governance-decision` 是 runner 前的策略解析层。它消费 validator 输出的结构化字段，
产出 `ResolvedExecutionPolicy` — 包含实际应使用的 execution mode、topology、
启动参数、降级原因和停止条件。resolver 只读，不启动 runner；`ags policy resolve`
提供唯一 CLI 入口，不保留隐藏兼容别名。
解析规则（M1–M10）写入 `protocol/runtime-adapters.md`。

## Exact Release Manifest

`crates/ags-verification::release_manifest` 只负责公开发行边界：A→B 按精确文件清单、
B 自有 rewrite/overlay 的固定哈希以及禁止项 fail closed。旧的章节解析、allowlist
和多目标 drift 引擎已删除；A→S 由 Git fast-forward 与 tree identity 证明。

## Public-Full Sanitized Boundary

公开版是 **public-full-sanitized**：保留 AGS 满血核心能力、项目入口文件、规则、
记忆胶囊机制、任务存档机制和第三方技能治理框架；只清除私有数据和本机运行状态。

公开版应包含：

- Rust `ags` workspace（`Cargo.toml`、`Cargo.lock`、`crates/`）和核心命令面；
- 公开 AGS MCP crate、`protocol/mcp-server.md`、公开安全的 MCP resources/prompts；
- `AGENTS.md`、`CLAUDE.md`、`WORKSPACE.md`、`AGENT_SUITE_PROTOCOL.md`；
- `protocol/`、`templates/`、`scripts/`、公开 docs、manifest 和治理规范；
- 空白记忆模板：`templates/memory/context-capsule.md`、`task-memory.md`、
  `archive-index.md`、`task-archive/README.md`；
- 项目入口融合模板：`templates/project-integration/AGENTS.md.template`、
  `templates/project-integration/CLAUDE.md.template`；
- 静态能力治理：第三方能力只在显式升级时审查，setup/update 为每个宿主生成唯一
  current snapshot；请求路径不联网、不扫描、不刷新。

公开版不得包含：

- `target/`、release/debug `ags` 二进制、构建缓存或临时日志；
- 用户真实记忆、真实任务归档、真实 receipt、真实交付报告；
- 已安装第三方技能、本地技能包、`global-skills/`、`skill-packs/`；
- `$HOME/.agents`、`$HOME/.codex`、`.claude/local/` 等本机配置状态；
- 私有路径、用户名、私有仓库名、密钥、token 或公司/个人敏感上下文。

`protocol/project-profile.md` 和 `protocol/context-memory.md` 是 public-safe
协议骨架。真实 project profile、context capsule、task archive、receipt 和 delivery
report 属于用户本地生长状态，不应进入公开分发包。

## Project Entry Integration

公开版不得用套件根目录的 `AGENTS.md` / `CLAUDE.md` 覆盖用户项目已有入口文件。
用户项目接入 AGS 时使用增量托管块：

```bash
ags project integrate --target /path/to/repo --dry-run
ags project integrate --target /path/to/repo --confirm
```

该命令只管理 `<!-- AGS:BEGIN managed-entry v2 -->` 到
`<!-- AGS:END managed-entry v2 -->` 之间的 AGS 块。用户自有内容保留在块外。
如果入口文件已有完整托管块，则原地更新该块；如果没有，则追加；如果只存在半截
marker 或发现与 AGS 治理冲突的入口规则，则停止并报告 conflict。确认写入使用原子
替换，不生成持久备份；默认 dry-run 不写文件。

## Skill Governance

Agent General Staff 在公开版中提供静态技能治理框架，但不预装第三方技能或
用户本地技能。`protocol/skill-governance.md` 定义权威清单、显式刷新、精确路由和
写入边界。第三方能力由用户选择可信来源，在维护者审查升级后通过 setup/update
刷新一次当前快照；运行时没有 adopt/ignore/rollback/sync 写入面。

Capability expected 集合以已安装 AGS source authority 为准，不得随执行命令的项目 cwd
变化。registry 声明为 required+routable 的真实父能力即使本体缺失也必须进入 inventory
与 strict verify 分母；内部 playbook 不作为独立 expected skill，但其文件完整性由父能力
承担。`ags doctor` 以 `third-party-capability-routing` 正式检查聚合这一闭环。

## Protocol Safety Assertions

validator、policy 和 release gates 强制执行以下关键协议安全断言。缺失或矛盾改写
始终为 FAIL，公开目标也不能用 rewrite/overlay 掩盖：

1. **ultracode thinking-only**: `Execution effort: ultracode` 只是 thinking intensity，
   不改变 execution authority、不启用 parallel topology、不添加 launch args。
2. **Heavy 级别不重写权限**: 任务级别是风险/审查等级，不是执行授权；权限仅来自
   `Execution mode`、`Execution topology` 与 `Delegation planning`，Heavy 只增加
   独立 review。
3. **plan-only no-write**: plan-only 不得产生 write-type launch args，
   active parallelism 和 headless/background-agent 必须被 strip 或 stop。
4. **runner resolver-first**: runner 必须消费 `ags policy resolve --format json` 输出的
   `effective_*` / `allowed_launch_args`，不得从原始任务卡字段直接拼接执行参数。

## M2 Agent Awareness (Project Discovery)

M2 提供只读命令，让 Agent 和操作者无需查询任务卡即可了解项目身份、协议状态和专属指令：

```bash
# 检测项目身份与 AGS 集成状态
ags project detect
ags project detect --target /path/to/repo --format json

# 增量融合 AGS 入口规则到用户项目入口文件
ags project integrate --target /path/to/repo --dry-run
ags project integrate --target /path/to/repo --confirm

# 检查协议文件状态、校验器入口、风险边界和 review/verify/receipt 要求
ags protocol status
ags protocol status --target /path/to/repo --format json

# 导出 Agent 专属项目说明
ags agent instructions --for codex
ags agent instructions --for claude-code
ags agent instructions --for cursor

# Kernel activation — CLI 降级路径（MCP 不可用时使用）
ags session preflight --for codex
ags session preflight --for claude-code --format json
ags session preflight --for cursor --target /path/to/repo
```

当宿主可调用 AGS MCP 时，`ags_preflight` 是默认 kernel activation 唤醒入口。
`ags session preflight` 是 MCP 不可用时的 CLI 降级路径。两条路径都将 project
detect、protocol status、agent instructions 聚合为单一只读报告，包含 memory
capsule/task-memory 路径、stop conditions、warnings、failures 和下一步建议。
核心 kernel activation 不依赖 skill governance，且独立于第三方 skill governance。

M2 awareness 命令（detect/status/instructions/preflight）均为只读；不安装 hook、
不启动 runner、不执行任务。`project integrate` 默认 dry-run，只有 `--confirm`
才写入入口文件托管块和备份。exit code：0 = suite/integrated/clean，1 =
partial/not-integrated/failures/conflicts，2 = 参数错误。

## 技能标记

任务卡末尾可包含 `[skill: xxx]` 标记。常用：`[skill: superpowers]`、
`[skill: diagnosing-bugs]`、`[skill: review]`、`[skill: codebase-design]`。
Superpowers 内部 playbook（如 `verification-before-completion`、
`test-driven-development`）应在任务正文中声明，只使用父标签
`[skill: superpowers]`，不得把 playbook 名写成独立标签。
