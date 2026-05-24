# Agent Governance Suite

可迁移 Agent 开发套件。将 Codex/Cursor/Claude Code 三方协作机制、开发技能套件、治理工具链打包为一键分发的工程化结构。

## 目录

```
agent-governance/
├── README.md                        # 本文件
├── AGENT_SUITE_PROTOCOL.md          # 套件级协议（角色、任务流、交付标准）
├── manifests/
│   ├── suite.yaml                   # 机器可读 manifest（required/optional/forbidden）
│   └── skills.lock.example.yaml     # 技能锁文件样例
├── protocol/                        # 项目接入协议模板
├── governance/                      # 治理机制
├── task-modules/                    # 专用任务卡填槽模块
├── templates/fallback-task-cards/   # 全局 fallback 任务卡模板
├── project-integration/             # 项目接入模板
├── scripts/                         # bootstrap/verify/diff/rollback
├── global-rules/                    # 全局 Agent 规则
└── global-skills/                   # 核心开发技能
```

## 安装

```bash
# 默认 dry-run，预览将要安装的内容
bash agent-governance/scripts/bootstrap.sh --dry-run

# 确认后正式安装
bash agent-governance/scripts/bootstrap.sh --apply

# 验证安装完整性
bash agent-governance/scripts/verify.sh
```

## 角色

- Codex / Cursor：负责设计、任务卡生成、复核
- Claude Code：负责执行任务卡
- 本项目只提供套件分发，不绑定任何具体项目的 agent 身份

## 约束

- 默认 dry-run，不静默覆盖 `$HOME` 下任何文件
- 安装前必须完成 backup
- 不接受未经人工 diff 的上游更新
- 不自动安装依赖
