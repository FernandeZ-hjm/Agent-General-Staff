---
name: ags-setup
description: "Initialize or upgrade the AGS machine runtime: install verified binaries, converge official skills and the machine lock, then run the sealed workspace update and Doctor."
metadata:
  ags_version: "v0.4.21"
---

# AGS Setup / Upgrade (v0.4.21, contract v3)

本机运行时装设与升级：

1. 内部环境可从 stable checkout 构建；公开用户使用签名 release / npm
   launcher。CLI/MCP 统一通过 `ags` / `ags mcp`；
2. `ags setup --source-root <checkout-or-release-runtime>`：写
   `~/.ags/v3/install.json`，
   同步官方 Skills 与全局 rules、刷新机器 lock（幂等，唯一安装入口；
   install.json 缺失时 `ags update` / `ags doctor` 会 fail closed 并提示）；
3. `ags update --workspace <any-adopted-project>`（sealed）→ `ags apply`：
   刷新项目 capability audit、机器 lock、全部注册项目入口管理块、
   `~/.agents/rules`、`~/.agents/skills/ags-*`；官方技能每次从 stable 覆盖，
   同版本修复也会收敛；update 是收敛器——预检失败零写入，重跑即收敛；
4. `ags doctor` 确认 healthy 且 install_ok=true、entry_drift 为空。
5. Agent 运行 sealed `ags govern host-projection`，确认
   `experience_healthy=true`；用户不手改宿主 MCP 或 Hook 配置。

⚠️ v2 的 setup saga / snapshot / MaintenanceService 已删除。
