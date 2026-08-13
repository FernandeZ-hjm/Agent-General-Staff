---
name: "ags-agents"
description: "当用户提到 /ags agents、AGS Agents、让其他 Agent 接入 AGS、把 Claude Code/Codex/OMP/Cursor/CodeBuddy-Code 纳入 AGS 治理、配置/复核 AGS MCP 注册、或问某个 Agent 是否能看到 AGS 时使用。运行 `ags agents scan` 盘点宿主，`ags agents govern` 预览接入方案，明确 `--apply` 时仅安装 AGS 自有记忆生命周期适配器，`ags agents verify --host <host>` 复核能力与记忆闭环。"
---

# AGS Agents

这是本机 AGS 命令技能，用来把其他 Agent 宿主接入 AGS 治理入口：先看哪些宿主存在，再给出 AGS MCP 注册/治理建议，最后复核该宿主是否看得到 AGS。

## 必须先执行

对目标仓库先运行 AGS preflight。MCP 可用时优先调用 `ags_preflight`；CLI fallback：

```bash
ags session preflight --for codex --target .
```

在 Claude Code 使用 `--for claude-code`；在 CodeBuddy-Code 使用 `--for codebuddy-code`。如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。
在 Oh My Pi 使用 `--for omp`。

## 路由

这是 `routing_surface=host_command` 的宿主前台命令技能，不是 AGS MCP
`SkillTarget`。不要把 `ags-agents` 提交给 `ags_route_request`。

盘点本机 Agent 宿主与 AGS MCP 注册状态：

```bash
ags agents scan
```

生成“让宿主进入 AGS 治理”的纳管建议。默认只预览，不写配置：

```bash
ags agents govern
```

用户明确同意后，只安装 AGS 自有原生记忆生命周期适配器：

```bash
ags agents govern --agent <claude-code|codex|cursor|codebuddy-code|omp> --apply --format json
```

该写动作会结构化合并 Claude Code/Codex/Cursor/CodeBuddy-Code 的工作区原生 hook，
或安装工作区 OMP 原生 extension，并写 action receipt。它不运行外部 MCP
registrar；AGS MCP 注册命令仍只返回建议。

复核指定宿主的能力可见性和原生记忆启动/结束闭环：

```bash
ags agents verify --host <host>
```

常见 host id：`claude-code`、`codex`、`omp`、`cursor`、`codebuddy-code`、`workbuddy`。

## 判断口径

- `scan` 是只读事实盘点：看 CLI、配置目录、macOS app bundle、AGS MCP 注册证据。
- `govern` 默认是纳管计划；显式 `--apply` 只写 AGS 自有
  Claude/Codex/Cursor/CodeBuddy-Code/OMP 工作区记忆适配器并生成 receipt，外部
  MCP 注册始终 advice-only。
- `verify` 同时复核 capability route 与该宿主的原生 memory adapter；严格模式要求两者都闭合。
- OMP 会发现 `.omp/agent/skills` 和 `.agents/skills`，记忆闭环由目标工作区的
  `.omp/extensions/ags-lifecycle.js` 承载。其 MCP 注册仍由 OMP 自身配置
  决定；未运行真实宿主 probe 时不得断言连接已建立。

## 安全边界

不要绕过 AGS 做临时初始化。不要把 AGS MCP 当作普通第三方 MCP；它是 host initialization adapter。只有用户明确要求任务卡/交接、handoff contract 已独立确认，且不存在未决或重开的 solution work 时，才可生成可执行任务卡；缺少任一条件都不得生成。

此技能期望的 AGS 产品版本：0.4.16。
