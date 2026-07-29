---
name: "ags-doctor"
description: "当用户提到 /ags doctor、AGS Doctor、AGS doctor，或需要诊断 AGS 状态时使用。"
---

# AGS Doctor

这是 AGS 自有的宿主前台命令技能。运行 `ags doctor --target .` 并优先汇总失败项。

它不是 MCP `SkillTarget`；宿主必须按静态 CLI hint 直接调用。

此技能期望的 AGS 版本：0.3.8。
