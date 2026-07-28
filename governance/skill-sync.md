# Static Skill Snapshot Governance

> AGS 0.3.6 的能力分发原则：上游显式升级一次，运行时只读一份。

## 原则

- 第三方版本只在维护者执行 update/release 时审查。
- 本地每个宿主只保留当前静态 snapshot，不保留历史 bundle。
- 普通 preflight、resource read、route、apply、inventory 和 verify 不联网。
- 没有运行时 adopt、ignore、rollback、dedupe 或 sync。
- 没有 user overlay、source registry、usage ledger 或持久备份。

## 更新流程

```text
审查上游 release/commit/license
→ 更新 tracked manifest 与 canonical body
→ 运行验证
→ setup/update 原子写入各宿主当前 snapshot
→ 新连接 preflight
→ 旧 lease 失效
```

宿主集合包括 `claude-code`、`codex`、`omp`、`cursor` 和 `codebuddy-code`。每个宿主
独立验证；“文件安装可见”不能替代 `ActiveSkillTable` 的 `Active + Ready + Routable`
证据。

## 删除策略

AGS 不创建 `.bak`、历史 snapshot、持久 quarantine 或计划型 rollback 数据。升级时
旧 current snapshot 被原子替换；版本恢复依赖重新安装一个经审查的 release，而不是
运行时隐藏接口。
