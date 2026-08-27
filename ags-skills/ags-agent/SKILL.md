---
name: ags-agent
description: Govern any normalized Generic Agent (claude-code, codex, cursor, codebuddy, omp, dsh, ...) via ags.toml [hosts] and doctor health; no admission allowlist.
metadata:
  ags_version: "v0.4.21"
---

# AGS Agent (v0.4.21, contract v3)

任意新宿主无需源码适配。能执行命令时走通用 CLI 注册；支持 MCP 时通过
`ags_decide` 注册。注册入口决定 transport，native hooks 仅是已知宿主的
增强投影。

宿主注册 = `ags.toml` 的 `hosts = [{ id, surface }]`（surface: cli|mcp），
由 `ags init` 生成骨架、sealed `ags govern host-projection` 完成连接和生命周期投影。

体检：`ags doctor --workspace .` 逐宿主报告 mode（cli / mcp / hooks /
unwired）。native hooks 是生命周期增强，不是第三种 surface；任意新宿主直接选择 CLI 或 MCP。

通用注册：CLI 使用 `ags govern host-register`；MCP 使用
`govern.host.register`。旧 `ags agent register/probe` 命令仍不存在。
