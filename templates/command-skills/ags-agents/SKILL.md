---
name: "ags-agents"
description: "当用户提到 /ags agents、AGS Agents、AGS agents，或需要纳管本机 Agent 宿主时使用。"
---

# AGS Agents

这是 AGS 自有的宿主前台命令技能。先运行 `ags agents scan`，按用户授权执行对应
`ags agents govern --agent <host> --apply`，最后用 `ags agents verify --host <host>` 复核。

它不是 MCP `SkillTarget`；宿主必须按静态 CLI hint 直接调用。

此技能期望的 AGS 版本：0.3.7。
