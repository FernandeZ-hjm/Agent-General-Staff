---
name: "ags-setup"
description: "当用户提到 /ags setup、AGS Setup、AGS setup，或需要初始化/升级本机 AGS 环境：先运行 preflight，再用 `ags setup --yes --force`，并通过 `ags verify --scope local`、`ags agents scan` 复核。"
---

# AGS Setup

这是本机 AGS 命令技能，用来把明确的 AGS 本机 runtime 初始化/升级操作路由到已安装的 `ags` CLI 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先运行 AGS preflight。MCP 可用时优先调用 `ags_preflight`；CLI fallback：

```bash
ags session preflight --for codex --target .
```

在 Claude Code 使用 `--for claude-code`；在 CodeBuddy-Code 使用 `--for codebuddy-code`。如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。

## 路由

这是 `routing_surface=host_command` 的宿主前台命令技能，不是 AGS MCP
`SkillTarget`。不要把 `ags-setup` 提交给 `ags_route_request`，也不要通过刷新
capability snapshot 尝试让它进入 `ActiveSkillTable`。

初始化或升级本机 AGS runtime。首次安装先查看检测结果并确认要批准的宿主：

```bash
ags setup
ags setup --yes --force --lifecycle-hosts <host1,host2|detected|none>
ags verify --scope local
ags agents scan
```

已有安装未改变宿主选择时可继续运行 `ags setup --yes --force`。不要把 setup
当成公开版发布命令。

## 安全边界

不要绕过 AGS 做临时初始化。只有用户明确要求任务卡/交接、handoff contract 已独立确认，且不存在未决或重开的 solution work 时，才可生成可执行任务卡；缺少任一条件都不得生成。

此技能期望的 AGS 产品版本：0.4.11。
