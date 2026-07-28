---
name: "ags-setup"
description: "当用户提到 /ags setup、AGS Setup、AGS setup，或需要初始化本机 AGS 环境时使用。"
---

# AGS Setup

这是 AGS 自有的宿主前台命令技能。运行 `ags setup --yes --force`，完成后用
`ags doctor --target .` 复核。

它不是 MCP `SkillTarget`；宿主必须按静态 CLI hint 直接调用。

此技能期望的 AGS 版本：0.3.7。
