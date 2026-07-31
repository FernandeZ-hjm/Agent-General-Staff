---
name: "ags-doctor"
description: "当用户提到 /ags doctor、AGS Doctor、AGS doctor，或需要诊断 AGS 安装、项目状态、本机 runtime、Agent/skill 可见性时：运行 `ags doctor --target .` 并优先汇总失败项。"
---

# AGS Doctor

这是本机 AGS 命令技能，用来把明确的 AGS 诊断操作路由到已安装的 `ags` CLI 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先运行 AGS preflight。MCP 可用时优先调用 `ags_preflight`；CLI fallback：

```bash
ags session preflight --for codex --target .
```

在 Claude Code 使用 `--for claude-code`；在 CodeBuddy-Code 使用 `--for codebuddy-code`。如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。

## 路由

这是 `routing_surface=host_command` 的宿主前台命令技能，不是 AGS MCP
`SkillTarget`。不要把 `ags-doctor` 提交给 `ags_route_request`。

诊断 AGS 安装和项目状态：

```bash
ags doctor --target .
```

Doctor 同时报告两层结果：

- runtime health：当前 CLI、daemon、宿主连接是否能运行；
- local conformance：runtime、MCP 注册、capability snapshot 和工作区 lifecycle
  adapter 是否等于当前版本的 canonical 生成结果，包括批准宿主集合与实际工作区
  投影是否一致。

已启用宿主存在本地漂移时退出 1；未启用的可选宿主显示 `skip`。远端 latest
只提供建议，离线不阻断。Doctor 只读，不自动迁移或重启；按 finding 给出的明确
修复命令另行执行。

如果用户明确要求修复，先用 dry-run/诊断输出说明将要修什么，再运行受支持的修复命令。

## 安全边界

不要绕过 AGS 做临时初始化。只有用户明确要求任务卡/交接、handoff contract 已独立确认，且不存在未决或重开的 solution work 时，才可生成可执行任务卡；缺少任一条件都不得生成。

此技能期望的 AGS 产品版本：0.4.1。
