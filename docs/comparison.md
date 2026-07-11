# AGS 与相关工具的关系

本文件说明 Agent General Staff（AGS）与 CrewAI、LangGraph、Claude Code Skills、
Cursor Rules 等工具的定位差异与协作关系。

**核心定位：AGS 不替代这些工具，而是治理它们进入工程执行流程的方式。**
在支持 MCP 的宿主里，AGS MCP 是这套治理的初始化入口：先显式调用
`ags_preflight`，再进入方案、任务卡、策略解析和验证链路；CLI preflight 只在
MCP 不可用时作为降级路径。

## 一句话总结

| 工具 | 定位 | AGS 与它的关系 |
|---|---|---|
| **CrewAI** | 多 Agent 任务编排框架 | AGS 治理 CrewAI 的任务入口、权限边界和验证门禁 |
| **LangGraph** | 图结构 Agent 工作流引擎 | AGS 治理 LangGraph 的节点行为的执行契约和 receipt |
| **Claude Code Skills** | Agent 能力扩展单元 | AGS 治理 skills 的推荐、安装确认、更新记录和回滚边界 |
| **Cursor Rules** | IDE 级项目指令 | AGS 治理跨 runtime 的协议一致性，Cursor Rules 是 IDE 入口的一种表达 |
| **Codex** | Agent 工程入口 | AGS 通过 AGS MCP 初始化门禁和协议，定义 Codex 在 preflight/solution/routing/review 中的角色边界 |

## 详细对比

### AGS vs CrewAI

**CrewAI** 是一个多 Agent 编排框架。它定义 Agent、Task、Crew 等抽象，让开发者
用 Python 描述"谁做什么、按什么顺序做"。CrewAI 的核心价值在于**任务编排和工作流
定义**。

**AGS** 不编排 Agent 之间的任务分配。AGS 关心的是：无论用什么框架（CrewAI、
LangGraph、或直接调用 Claude Code/Cursor），Agent 进入项目执行时：
- 有没有先通过 AGS MCP `ags_preflight`，或在 MCP 不可用时通过 CLI preflight，
  了解项目上下文？
- 有没有形成方案并获得用户确认？
- 有没有明确的任务卡和权限边界？
- 执行后有没有验证证据和 receipt？

**关系**：如果一个团队使用 CrewAI 编排多个 Agent 完成复杂任务，AGS 可以为每个
Agent 的**任务入口**提供 boundary check —— 确保 Agent 在项目约束内运行，不会越权
或跳过验证。AGS 是 CrewAI 的 governance wrapper，不是替代品。

### AGS vs LangGraph

**LangGraph** 是一个基于图结构的 Agent 工作流框架。它用节点和边定义 Agent 执行的
控制流，适合需要复杂分支、循环和人机交互的工作流场景。LangGraph 关心的是
**如何执行**。

**AGS** 关心的是**执行的前提条件**和**执行后的可验证性**。它不在工作流内部
做状态管理，而是在工作流启动前检查：
- 宿主是否已完成 AGS MCP `ags_preflight` 初始化门禁？
- 任务卡是否合法（validator）？
- 权限模式是否合适（policy resolver）？
- 权限是否收敛为 `plan-only` / `execute-and-verify` 两态，且独立 stop 条件是否满足？

**关系**：AGS 可以作为一个 LangGraph workflow 的 **pre-launch gate** 和
**post-execution receipt 层**。LangGraph 负责工作流怎么跑，AGS 负责能不能跑、
跑完怎么证明。两者互补，不冲突。

### AGS vs Claude Code Skills

**Claude Code Skills** 是 Claude Code 的本地能力扩展单元。每个 skill 是一个
Markdown 指令文件，告诉 Agent 如何处理特定场景（如 TDD、commit message 生成、
代码审查）。Skills 的价值在于**可组合的领域能力**。

**AGS 的技能治理层**做的是另外一件事：
- **推荐（recommend）**: 列出值得安装的第三方技能
- **扫描（scan）**: 检查本地已安装技能的状态
- **提案（propose）**: 生成安装计划及影响评估
- **确认安装（confirm）**: 仅在用户显式确认后执行安装
- **审计记录（adopt/ignore log）**: 记录每次技能变更
- **回滚边界**: 不自动更新，不静默覆盖用户目录

**关系**：AGS 是技能的 **governance wrapper**。Skill 本身定义 Agent 能做什么，
AGS 管理这些能力的安装、更新和审计。AGS 不重写 skill，不打包 skill，不接管 skill
的运行时行为。

### AGS vs Cursor Rules

**Cursor Rules** 是 Cursor IDE 中项目级或用户级的 `.cursorrules` / `.cursor/rules/`
配置文件。它们告诉 Cursor Agent 项目约定、代码风格和特殊指令。Cursor Rules
的价值在于**IDE 内 Agent 的上下文约束**。

**AGS** 提供的是 **跨 runtime 的协议层**：
- Cursor Rules 是 IDE 内的；AGS 的任务卡和协议对 Cursor、Codex、Claude Code 通用
- Cursor Rules 关注代码风格和项目约定；AGS 关注执行权限、任务交接和验证门禁
- Cursor Rules 是静态指令；AGS 提供 validator、resolver、receipt 等可执行的 gate

**关系**：一个项目可以同时有 Cursor Rules（告诉 Cursor 怎么写代码）和 AGS 协议
（告诉所有 Agent 什么能做什么不能做、任务怎么交接）。AGS 的 `project integrate`
命令可以在不覆盖用户已有 Cursor Rules 的前提下，将 AGS 治理入口增量合并到项目
入口文件中。

### AGS vs Codex

**Codex** 是 Agent 工程入口（由 OpenAI 提供的终端 Agent runtime）。在 AGS 的
三方协作模型中，Codex 承担 AGS MCP preflight、方案形成、任务分级路由和最终
review 的职责。

**AGS** 定义这个模型中的角色分工和协议边界：
- Codex 先通过 AGS MCP `ags_preflight` 完成初始化门禁；MCP 不可用时才降级到
  `ags session preflight`
- Codex 负责 framing（方案形成）和 routing（任务分级）
- Claude Code 执行有边界的实现任务
- Cursor 在 IDE 内完成轻量编辑和验证
- AGS 提供三方共享的协议、任务卡骨架、validator 和 receipt

**关系**：Codex 是 AGS 治理模型中的主要 orchestrator 之一。AGS 不是 Codex 的
替代品——Codex 负责判断和决策，AGS 负责确保判断和决策有记录、有边界、可验证。

## AGS 的独特价值

AGS 的独特价值不是否认上述工具已有的编排、持久化、人机协作、观测或验证能力，
而是把这些能力进入工程执行时需要的治理边界，组合成一套跨 runtime 的本地协议：

1. **执行前 gate** — 任务卡 validator + policy resolver 在 Agent 开始工作前检查
   "能不能做、怎么做、权限够不够"
2. **初始化门禁** — AGS MCP `ags_preflight` 是 MCP 宿主里的优先入口，CLI 是降级路径
3. **三段门槛** — 方案 OK ≠ 可以执行。需要显式任务卡指令才能进入路由
4. **执行后 receipt** — 每次执行都有结构化审计记录，不是聊天日志
5. **跨 runtime 协议** — 同一套任务卡、策略和验证规则对 Codex/Claude Code/Cursor
   通用
6. **技能治理** — 管理第三方技能的安装、审计和回滚，不自动安装不静默覆盖

## 不做什么

AGS **不**做以下事情（这些是上述工具各自的领域）：

- **不**做 Agent 任务编排（→ CrewAI / LangGraph）
- **不**定义 Agent 工作流的控制流和分支（→ LangGraph）
- **不**提供单个 Agent 的领域能力扩展（→ Skills）
- **不**定义项目级的代码风格和约定（→ Cursor Rules / CLAUDE.md）
- **不**替代任何 Agent runtime（→ Claude Code / Codex / Cursor）
- **不**代理或内置外部 advisory MCP；外部 MCP 必须独立安装、独立调用、独立治理

AGS 做的事情是：**当这些工具进入你的工程流程时，确保每一步都有边界、有记录、
可验证。**

## Licensing Note

AGS Public Edition 使用 GPL-3.0-only License。与上述工具（多数使用 MIT、Apache 2.0
等 OSI 许可证）的集成不改变各自的许可条款——AGS 治理的是工程流程，不是代码的
license 派生关系。
