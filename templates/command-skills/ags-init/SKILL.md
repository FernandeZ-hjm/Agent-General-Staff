---
name: "ags-init"
description: "当用户提到 /ags init、AGS Init、AGS init，或需要纳管当前仓库：先运行 preflight，再运行 `ags init --target .`，最后复核 `ags session preflight --for <host> --target .`。"
---

# AGS Init

这是本机 AGS 命令技能，用来把明确的 AGS 项目纳管操作路由到已安装的 `ags` CLI 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先运行 AGS preflight。MCP 可用时优先调用 `ags_preflight`；CLI fallback：

```bash
ags session preflight --for codex --target .
```

在 Claude Code 使用 `--for claude-code`；在 CodeBuddy-Code 使用 `--for codebuddy-code`。如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。

## 路由

这是 `routing_surface=host_command` 的宿主前台命令技能，不是 AGS MCP
`SkillTarget`。不要把 `ags-init` 提交给 `ags_route_request`。

纳管当前仓库：

```bash
ags init --target .
ags session preflight --for codex --target .
```

需要登记其他仓库时，把 `.` 替换成明确的绝对路径。不要默认触碰其他仓库。
Init 会读取现有 install manifest 中明确批准的 lifecycle hosts，并用统一
`LifecycleProjection` 为当前工作区安装对应适配器；它不会重新探测或自行扩大宿主集合。

## 安全边界

不要绕过 AGS 做临时初始化。只有用户明确要求任务卡/交接、handoff contract 已独立确认，且不存在未决或重开的 solution work 时，才可生成可执行任务卡；缺少任一条件都不得生成。

此技能期望的 AGS 产品版本：0.4.1。
