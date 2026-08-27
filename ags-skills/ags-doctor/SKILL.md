---
name: ags-doctor
description: 当用户提到 /ags doctor、AGS Doctor，或诊断 AGS workspace、Agent、能力、宿主 hook 与入口漂移时使用。doctor 是只读命令。
metadata:
  ags_version: "v0.4.21"
---

# AGS Doctor (v0.4.21, contract v3)

只读体检，零写入。输出 JSON 含：矩阵 lint、宿主 hook 健康、能力路由、
证据链完整性、入口漂移（sync-on-update 自检）。

```bash
ags doctor --workspace .
ags check governance|matrix|capabilities|evidence --workspace . --format json
ags status --workspace .
```

healthy=false 时逐项排查：
- `lint_findings`：ags.toml 矩阵/段位问题；
- `hosts[].mode`：cli / mcp / hooks / unwired；
- `capability_routes` / `capability_audit_clean`：项目审计状态；只在
  `ags check capabilities` 时作为阻断；
- `evidence_chain_ok`：false 表示证据链断裂；
- `entry_drift`：入口文件/规则/技能落后于 0.4.21 —— 跑 `ags update` 修复。
- `git_projection_drift`：本地 entry filter/exclude 缺失，或 Git baseline
  仍含历史 AGS block。
- `git_projection_repair`：出现 projection drift/error 时可直接执行的修复命令。
- `version` / `build`：固定 `v0.4.21` 与独立 `commit[.dirty]` 构建身份。
- `core_healthy`：矩阵、证据、能力和投影内核状态；
- `experience_healthy`：宿主 `ags mcp` 注册、生命周期 Hook 与 memory store 状态；
- `experience`：逐宿主连接、SessionStart/SessionEnd 和记忆路径证据。

⚠️ 旧语法（`ags doctor all`、`ags doctor --target`）在 v3 已删除。
