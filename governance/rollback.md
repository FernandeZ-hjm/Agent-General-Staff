# Rollback Guide

Agent 开发套件的回滚操作指引。

## 回滚场景

| 场景 | 回滚方式 |
|------|---------|
| bootstrap --apply 后发现内容错误 | 从 backup 目录恢复被覆盖的文件 |
| 技能更新后触发规则异常 | 从 acceptance log 找到 pre-update hash，恢复对应文件 |
| 套件版本升级后大面积异常 | 整体回退到上一版本套件基线 |

## Backup 目录结构

`bootstrap.sh --apply` 执行前自动创建 backup：

```text
~/.agents/backups/
└── suite-backup-20260524-143000/
    ├── manifest.yaml         # 本次应用的 manifest 快照
    ├── changed-files.txt     # 被覆盖文件的清单
    └── files/                # 原始文件副本（保持相对路径）
        ├── .agents/
        │   ├── rules/
        │   │   ├── SOUL.md
        │   │   └── core.md
        │   └── skills/
        │       └── ...
        └── .codex/
            └── RTK.md
```

## rollback.sh 用法

```bash
# 列出所有可用备份
bash scripts/rollback.sh --list

# 检查特定备份的内容
bash scripts/rollback.sh --inspect <backup_dir>

# 从备份恢复（默认 dry-run）
bash scripts/rollback.sh --restore <backup_dir>

# 从备份恢复（实际执行）
bash scripts/rollback.sh --restore <backup_dir> --apply
```

## 回滚验证

回滚后必须运行：

```bash
bash scripts/verify.sh
```

如果 verify 失败，检查 backup 完整性或回退到更早的 backup。

## 不可回滚的操作

以下操作不受 rollback 保护：

- 插件缓存目录 (`~/.codex/plugins/cache/`) 的变更 — 由运行时插件机制管理
- 系统级包安装 (`brew install`, `pip install`) — 需单独手动回滚
- Git bare repo 的 push/force-push — 见 git reflog

## 备份保留策略

- 保留最近 5 次 bootstrap backup
- 超过 5 次的旧备份在下次 bootstrap --apply 时自动提示清理
- 与重大版本升级关联的备份建议永久保留
