---
name: ags-govern
description: Use the sealed operation registry (skill install/remove, host projection, update) with decide→apply semantics. The registry is exactly the sealed subset in contract v3.
metadata:
  ags_version: "v0.4.21"
---

# AGS Govern (v0.4.21, contract v3)

v3 注册表只含封条子集，全部两段式（seal → `ags apply` 单次消费）：

```bash
ags skill list
ags skill recommend [query]
ags govern skill install --skill-id <id> --path <local-dir> [--ack-risk <id>] --workspace .
ags govern skill remove --skill-id <id> --workspace .
ags govern host-projection --host <id> --surface cli|mcp --lifecycle full [--slug <ascii>] --workspace .
ags update [--sources ...] --workspace .
ags schema [OPERATION]        # 查看注册表/单个操作的 payload 形态
```

第三方 Skill 先经过只读审计并 sealed apply 到
`~/.ags/v3/skill-bodies/<id>/<hash>`；`~/.agents/skills/<id>` 只链接不可变
body。机器 registry/lock 是安装真相；项目
`.ags/capabilities.lock` 只做审计，不参与运行时路由。宿主已选择的 Skill
优先；没有明确匹配时才调用 `ags route`。第三方 MCP 注册仍是
advice-only。

⚠️ v2 的 `govern task/capability/evidence/policy/gate/memory` 操作已全部
删除：任务走 `ags run`，能力查询走 `ags check capabilities`。
