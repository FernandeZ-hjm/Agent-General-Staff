---
name: "ags-doctor"
description: "当用户提到 /ags doctor、AGS Doctor，或诊断 AGS runtime、workspace、Agent 与能力可见性时使用。doctor 是只读 Operation。"
---

# AGS Doctor

AGS 产品版本：0.4.20

```bash
ags doctor --workspace . --format json
```

Doctor 只诊断 AGS runtime、authenticated workspace session、Generic Agent surface、
capability snapshot 与 AGS-owned 投影。它不运行项目测试、不自动修复、不重启、不迁移；
需要变更时由对应领域命令生成计划，再显式 `ags apply`。

snapshot missing/stale 只阻断精确 Skill/MCP 选择，不阻断 doctor 本身。若目标不明确，
返回结构化 workspace_required/workspace_ambiguous，不猜 HOME、最近项目或 managed-projects。

此技能期望的 AGS 产品版本：0.4.20，contract v2。
