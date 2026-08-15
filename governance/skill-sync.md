# Static Skill Snapshot Governance

> AGS v0.4.20 的能力分发原则：上游显式升级一次，运行时只读一份。

## 原则

- 官方/随包能力版本只在维护者执行 update/release 时审查；纯第三方 Skill 可在外部拉取后进入机器私有纳管事务。
- 本地每个宿主只保留当前静态 snapshot，不保留历史 bundle。
- 普通 decide、resource read、apply、inventory 和 check 不联网。
- `ags govern skill install/remove` 只生成密封计划；唯一写入口是随后显式的 `ags apply <action_ref>`。
- 普通请求路径没有 adopt、ignore、rollback、dedupe 或 sync，也没有 user overlay、usage ledger 或持久备份。

## 更新流程

```text
官方能力：审查上游 release/commit/license → 更新 tracked manifest 与 canonical body
纯第三方：外部工具拉取 → govern skill install → 确认密封计划 → ags apply
→ 运行验证
→ setup/update 原子写入各宿主当前 snapshot
→ 新连接建立 authenticated workspace session
→ 旧 action_ref 失效
```

任意规范化 HostId 都可作为 Generic Agent 以 `cli|mcp|hybrid` surface 纳管。
官方 Adapter 只增加探针和 hooks，不构成宿主 allowlist。“文件安装可见”不能替代
`ActiveSkillTable` 的 `Active + Ready + Routable` 证据。

## 删除策略

AGS 不创建 `.bak`、历史 snapshot、持久 quarantine 或计划型 rollback 数据。升级时
旧 current snapshot 被原子替换；版本恢复依赖重新安装一个经审查的 release，而不是
运行时隐藏接口。
