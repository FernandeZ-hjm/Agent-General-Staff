---
name: ags-setup
description: "Initialize an already obtained AGS runtime, or use the canonical sealed upgrade flow; verify five binaries, converge official skills and the machine lock, then run Doctor."
metadata:
  ags_version: "v0.4.21"
---

# AGS Setup and Upgrade (v0.4.21, contract v3)

先区分四个入口：安装器取得可信运行时；`setup` 初始化机器内容；`upgrade`
迁移运行时版本；`update` 只收敛能力锁与项目投影；`init` 只采用项目。

1. 内部环境可从 stable checkout 构建；公开用户使用签名 release / npm
   launcher。CLI/MCP 统一通过 `ags` / `ags mcp`；
2. `ags setup --source-root <checkout-or-release-bundle>`：先运行并核对同版本的
   `ags / ags-mcp / ags-host / ags-policy / ags-release`，再写
   `~/.ags/v3/install.json`、同步官方 Skills 与全局 rules、刷新 machine lock。
   setup 不下载、不选择版本；任何缺失或版本不一致都 fail closed，且仅在
   rules、完整官方 Skills 与 machine lock 全部收敛后写入 install record；
3. 后续版本迁移使用 `ags upgrade check` → `ags upgrade plan` →
   `ags apply <ACTION_REF>` → `ags upgrade verify <ACTION_REF>`。recover 也先返回
   sealed action_ref；七日提醒只提示，不自动下载或 apply；
4. `ags update --workspace <any-adopted-project>`（sealed）→ `ags apply`：
   刷新项目 capability audit、机器 lock、全部注册项目入口管理块、
   `~/.agents/rules`、`~/.agents/skills/ags-*`；官方技能每次从 stable 覆盖，
   同版本修复也会收敛；update 是收敛器——预检失败零写入，重跑即收敛；
5. `ags doctor` 确认 healthy 且 install_ok=true、entry_drift 为空。
6. Agent 运行 sealed `ags govern host-projection`，确认
   `experience_healthy=true`；用户不手改宿主 MCP 或 Hook 配置。

⚠️ v2 的 setup saga / snapshot / MaintenanceService 已删除。
