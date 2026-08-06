---
name: "ags-skill"
description: "当用户提到 /ags skill、AGS Skill、AGS skill，或要查看、核实、显式纳管本机第三方 Skill 时使用。普通路由只读取静态快照；纳管必须经过机器私有的 plan/hash/apply。"
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
ags skill adopt <local-skill-dir> [--metadata <routing.yaml>] [--host <host>] --format json
ags skill adopt <local-skill-dir> [--metadata <routing.yaml>] [--host <host>] --yes --plan-hash <hash>
ags skill status <skill-id>
ags skill remove <skill-id> --format json
ags skill remove <skill-id> --yes --plan-hash <hash>
ags capability inventory
ags capability verify --host <host> --strict
ags capability snapshot --host <host>
```

普通请求路径只消费显式生成的静态快照：不联网、不扫描上游、不更新 epoch、不写
私有注册表。`ags-setup`、`ags-init`、`ags-doctor`、`ags-agents` 是宿主直接调用的命令
技能，注册表中为 `not-routable`；不得把它们提交给 `ags_route_request` 的
`SkillTarget`。只有 ActiveSkillTable 中 `active + ready + routable` 的条目能路由。

AGS 官方能力升级属于显式 update/release 工作。纯第三方 Skill 使用另一条机器私有
通道：先由外部工具拉取源码，再用 `ags skill adopt` 审计许可证、普通文件边界、内容
哈希与可选 routing metadata。默认只输出计划；apply 必须提交同一计划哈希。成功后，
body 进入 `$AGS_RUNTIME_HOME/skill-bodies` 的不可变修订，来源与语义元数据进入私有
registry，所选宿主只保留指向该 body 的薄索引，并显式重建对应宿主快照。任何第三方
正文和能力专属配置都不得写入 AGS Git。

`--metadata` 是机器私有 YAML，可补充 `summary`、`intent_tags`、正反例、entrypoints、
invoke hint、auth 与版本。宿主仍基于完整任务语义做唯一判断；AGS Resolver 只验证宿主
提交的精确 `skill_id` 是否在 preflight 绑定快照中 active。纳管或移除后必须重启 AGS
MCP 并重新 preflight；旧 binding 必须 fail closed。

OMP 是一等宿主。验证时使用 `--host omp`；其能力必须出现在 OMP 自己的静态快照中，
不能借用 Codex 或 Claude 的可见性结论。

此技能期望的 AGS 产品版本：0.4.14。
