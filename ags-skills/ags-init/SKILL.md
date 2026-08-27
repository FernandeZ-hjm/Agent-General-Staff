---
name: ags-init
description: Attach exactly one project with the contract-v3 projection, install absent hook templates, register it for sync-on-update, and preserve user files. Idempotent on re-run.
metadata:
  ags_version: "v0.4.21"
---

# AGS Init (v0.4.21, contract v3)

采用一个 workspace（sealed 两段式）：

```bash
ags init --workspace <path> [--slug <id>] [--role A|S|B]
ags apply <ACTION_REF> --workspace <path>   # 单次消费
```

事务内容：ags.toml（缺失才写；已存在一律保留）、缺位 hooks 模板、
ownership 清单、注册进 `~/.ags/v3/managed.json`、安装当前版本入口管理块
（AGENTS.md/CLAUDE.md/HERMES.md/CODEBUDDY.md，带版本戳，用户内容零触碰）。
Git workspace 同时安装 repository-local entry filter 与 runtime exclude；
managed block 留在工作树，但 clean 后不进入 Git。
重跑 init 幂等：跳过已存在文件，只补注册与入口同步。

⚠️ 旧语法（`ags setup`、`ags init --migration ...`）在 v3 已删除。
