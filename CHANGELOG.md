# 更新日志

## V1.5

这次更新主要解决一件事，任务卡不再只是 prompt，而是被治理套件正式校验的执行合约。

以前任务卡虽然有固定格式，但更多依赖 Agent 自觉。V1.5 之后，任务卡进入可验证阶段。格式不对，就不能被当成正式执行入口。

### 主要变化

- 新增任务卡格式校验脚本，`scripts/validate-task-card.sh`
- `scripts/run-task-card.sh` 在执行任务卡前会先做格式校验
- Prompt Maker 进一步收紧任务卡输出规范
- DIY/Core 同步纳入任务卡校验能力
- 安装清单和验证流程增加对应检查
- 清理被误跟踪的 Python cache 文件

### 为什么重要

多 Agent 协作最怕的不是某个 Agent 不会写代码。

最怕的是大家拿到的任务不是同一种东西。

一个把任务卡当执行合约，一个把它当普通 prompt，一个又在中途自己补规则，最后看起来都在干活，实际已经开始偏航。

V1.5 把任务卡校验放进治理链路里，是为了让 Claude Code、Codex、Cursor 之间的任务交接更稳定，也减少自由 prompt 带来的漂移。

### 升级建议

升级后建议先运行：

```bash
bash scripts/verify.sh
bash scripts/security-doctor.sh
```

如果你已经把治理套件接入项目，建议重新跑一次项目检查：

```bash
bash scripts/kit-doctor.sh doctor --target-project /path/to/project
```

如果你要用 runner 执行任务卡，可以先手动验证任务卡格式：

```bash
bash scripts/validate-task-card.sh path/to/task-card.md
```

### 公开边界

V1.5 保持公开仓库边界，不纳入私有项目名、私有发布拓扑、个人 persona profile、私有同步 registry、真实 secret、token 或 API key。
