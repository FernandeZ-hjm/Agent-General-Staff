---
name: "ags-skill"
description: "当用户提到 /ags skill、AGS Skill、AGS skill，或需要管理第三方技能时使用。"
---

# AGS Skill

这是 AGS 自有的宿主命令技能，也是 `ags-governance-ops` 组中唯一允许作为
`SkillTarget` 精确路由的条目。

## 必须先执行

对目标仓库先运行 AGS preflight：

```bash
ags session preflight --for <host> --target .
```

## 路由

运行 `ags skill inventory` 查看静态目录。第三方来源只在明确的安装或升级流程中
更新；完成后运行 `ags capability snapshot --write --host <host>` 刷新一次静态快照，
并用 `ags skill verify --host <host> --strict` 复核。

## 安全边界

运行时没有 adopt、ignore、rollback、dedupe 或 sync 写入面。不要把 `ags-setup`、
`ags-init`、`ags-doctor` 或 `ags-agents` 提交为 MCP `SkillTarget`；它们由宿主按
冻结的 CLI hint 直接调用。

此技能期望的 AGS 版本：0.3.7。
