# Sync Protocol

Agent 开发套件的技能/规则同步治理协议。

## 核心原则

1. **本地是 source of truth**。远端上游只是候选更新来源，不可直接覆盖本地文件。
2. **dry-run first**。任何变更必须先在 dry-run 模式下预览效果。
3. **diff before apply**。人工审阅 diff 后再决定是否接纳。
4. **backup before overwrite**。覆盖前必须备份原始文件到带时间戳的 backup 目录。
5. **acceptance log**。每次接纳/拒绝/部分接纳决策必须记录到 acceptance log，含理由和基线哈希。
6. **rollback ready**。每次更新前计算回滚路径；如果更新导致问题，必须可回退到 pre-update backup。
7. **禁止静默覆盖**。非交互模式下禁止覆盖 `$HOME/.agents/skills`、`$HOME/.agents/rules`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`。

## Source of Truth

| 资产类型 | Source of Truth | 更新方式 |
|---------|----------------|---------|
| 全局 rules (SOUL, core) | 套件库 `global-rules/` | 套件 release → manual diff → apply |
| 核心开发技能 | 套件库 `global-skills/` | 套件 release → manual diff → apply |
| 项目接入模板 | 套件库 `project-integration/` | 项目初始化时参考 |
| 插件技能 | 运行时插件机制 | 通过插件机制更新，不手改缓存 |
| 本地自定义技能 | 本机 `~/.agents/skills/` | 人工创作，不进套件 |

## Dry-Run 流程

```bash
# 1. 运行只读检查
bash scripts/diff-local.sh

# 2. 预览 bootstrap 将要安装的内容
bash scripts/bootstrap.sh --dry-run

# 3. 审阅 diff 输出
# diff-local.sh 会列出: local-only, suite-only, content-diff 三类文件
```

## Diff & Apply 流程

```bash
# 1. 备份当前状态
bash scripts/rollback.sh --backup

# 2. 预览变更
bash scripts/diff-local.sh

# 3. 确认后应用
bash scripts/bootstrap.sh --apply

# 4. 验证
bash scripts/verify.sh

# 5. 如果不满意，回滚
bash scripts/rollback.sh --restore <backup_dir>
```

## Acceptance Log

每次接受或拒绝套件更新后，在 `governance/skill-adoption-log.yaml` 中记录：

```yaml
- id: <decision_id>
  timestamp: <ISO8601>
  decision: accept | reject | partial_accept
  reason: <why>
  baseline:
    before_hash: <sha256>
    after_hash: <sha256>
  affected_paths:
    - <path>
  rollback_backup: <backup_dir or N/A>
```

## 禁止命令

在任何自动化或脚本流程中禁止：

- `rm -rf ~/.agents/skills/*`
- `cp -rf <suite> ~/.agents/skills/`  (无 dry-run 保护)
- `lark-cli update`
- `npx skills add/remove/update`
- `pip install` / `npm install -g`  (无确认)
- `git push --force` 到远端
- 任何未经 dry-run 验证的批量写入操作

## 回滚

详见 `governance/rollback.md`。
