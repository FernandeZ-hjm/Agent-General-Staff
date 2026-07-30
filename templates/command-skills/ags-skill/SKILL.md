---
name: "ags-skill"
description: "当用户提到 /ags skill、AGS Skill、AGS skill，或要查看、核实宿主当前可见的 AGS Skill 时使用。只读取权威清单和已安装静态快照；不在运行时下载、adopt、ignore、sync 或 rollback。"
---

# AGS Skill

这是 AGS 静态 Skill catalog 的宿主命令入口。

## 先完成 preflight

```bash
ags session preflight --for <codex|claude-code|omp|cursor|codebuddy-code> --target .
```

## 可用命令

```bash
ags skill inventory
ags skill verify --host <host> --strict
ags capability inventory
ags capability verify --host <host> --strict
ags capability snapshot --host <host>
```

普通读取只消费安装时生成的静态快照：不联网、不扫描上游、不更新 epoch、不写
overlay。`ags-setup`、`ags-init`、`ags-doctor`、`ags-agents` 是宿主直接调用的命令
技能，注册表中为 `not-routable`；不得把它们提交给 `ags_route_request` 的
`SkillTarget`。只有 ActiveSkillTable 中 `active + ready + routable` 的条目能路由。

第三方能力升级属于显式 AGS update/release 工作：先审查并更新仓库内权威清单和
canonical body，再运行 setup/update 重建一次宿主静态快照。运行时不提供单项
adopt/ignore/rollback/sync 接口。

OMP 是一等宿主。验证时使用 `--host omp`；其能力必须出现在 OMP 自己的静态快照中，
不能借用 Codex 或 Claude 的可见性结论。

此技能期望的 AGS 产品版本：0.4.0。
