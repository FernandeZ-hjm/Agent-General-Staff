# Agent Toolchain Sync Task Module

本文件不是独立完整任务卡模板。Cursor / Codex 在生成“本地 Agent 技能/插件与上游同步”任务卡时，先使用 `docs/agent-workflow/task-card-template.md` 的固定骨架，再按本文件填入动态槽位。Claude Code 只执行最终任务卡，不自行放宽治理规则。

---

## 固定槽位填法

```markdown
任务：
<一句话说明本次要检查、提案、接纳门禁或记录哪一个 agent-toolchain 条目>

背景：
<只写本次条目、上游、已有 proposal / patch / log 状态，不重复长期协议>

相关路径：
- `tools/agent-toolchain/`
- `tools/agent-toolchain/data/agent-toolchain-update-plan.yaml`
- `tools/agent-toolchain/data/agent-toolchain-source-resolution.yaml`
- `tools/agent-toolchain/proposals/`
- `tools/agent-toolchain/acceptance-patches/`
- `tools/agent-toolchain/agent-toolchain-acceptance-log.yaml`

本次任务相关文件：
- `docs/agent-workflow/agent-toolchain-sync-governance.md`
- `tools/agent-toolchain/README.md`
- `tools/agent-toolchain/reports/<report>.md`
- `tools/agent-toolchain/proposals/<proposal>.md`

适用治理文档：
- `docs/agent-workflow/agent-toolchain-sync-governance.md`

目标：
1. <例如：运行只读 no-network 检查>
2. <例如：为 skill/<name> 生成 proposal，或对既有 proposal 跑 dry-run>
3. <例如：只生成 review-only patch，不应用>

非目标：
- 不写 `/Users/a92550/.agents/skills`
- 不写 `/Users/a92550/.codex/skills`
- 不写 `/Users/a92550/.codex/plugins/cache`
- 不运行 `lark-cli update`
- 不运行 `npx skills add/remove/update`
- 不自动应用 patch
- 不把 plugin-cache 子 skill 当作独立同步对象

实施要求：
- 默认只允许 read-only check / diff proposal / accept dry-run。
- 只有任务卡明确授权时，才允许生成 proposal、报告或 review-only patch。
- 动态命令输出只放在验证结果或风险提示中，不加入“读取并遵守”清单。

允许命令：
~~~bash
tools/agent-toolchain/check-agent-toolchain-updates.sh --no-network --report /tmp/agent-toolchain-update-report.no-network.md
tools/agent-toolchain/propose-agent-toolchain-update.sh <canonical_id>
tools/agent-toolchain/accept-agent-toolchain-update.sh <proposal.md> --dry-run
tools/agent-toolchain/accept-agent-toolchain-update.sh <proposal.md> --confirm-generate-patch
bash -n tools/agent-toolchain/check-agent-toolchain-updates.sh
bash -n tools/agent-toolchain/propose-agent-toolchain-update.sh
bash -n tools/agent-toolchain/accept-agent-toolchain-update.sh
~~~

验证：
~~~bash
tools/agent-toolchain/check-agent-toolchain-updates.sh --no-network --report /tmp/agent-toolchain-update-report.no-network.md
bash -n tools/agent-toolchain/check-agent-toolchain-updates.sh
bash -n tools/agent-toolchain/propose-agent-toolchain-update.sh
bash -n tools/agent-toolchain/accept-agent-toolchain-update.sh
~~~

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。报告必须列出：
- 是否修改了 repo 内工具/文档
- 是否生成 proposal / patch / report
- 是否触碰本地 skill/plugin 目录（预期必须为否）
- 验证命令和结果
- 仍需人工确认的接纳事项

[skill: verify]
```
