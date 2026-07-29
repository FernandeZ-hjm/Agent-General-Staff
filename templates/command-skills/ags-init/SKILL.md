---
name: "ags-init"
description: "当用户提到 /ags init、AGS Init、AGS init，或需要纳管当前项目时使用。"
---

# AGS Init

这是 AGS 自有的宿主前台命令技能。运行 `ags init --target .`，完成后运行
`ags session preflight --for <host> --target .`。

它不是 MCP `SkillTarget`；宿主必须按静态 CLI hint 直接调用。

此技能期望的 AGS 版本：0.3.8。
