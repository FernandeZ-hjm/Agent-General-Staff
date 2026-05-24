## 任务卡

读取并遵守：
- AGENTS.md
- CLAUDE.md
- docs/agent-workflow/agent-task-protocol.md
- docs/agent-workflow/task-routing.md
- docs/agent-workflow/cursor-skill-index.md

执行者：Claude Code

任务级别：Heavy

任务：
<一句话任务描述>

背景：
<只写本次任务差异，不重复长期协议。说明涉及的数据、历史产物和风险范围。>

相关路径：
- <path_1>
- <path_2>

本次任务相关文件：
- <file_1>
- <file_2>

适用治理文档：
- <governance doc>

目标：
1. <goal_1>
2. <goal_2>

非目标：
- <non-goal_1>
- <non-goal_2>

实施流程：
1. 阅读与诊断 → 输出 root cause / 设计 / 计划 → 等待确认
2. 确认后执行
3. 验证与交付

基线保护：
- 不修改、删除、覆盖: <受保护数据/目录>

实施要求：
- 先输出 root cause / design / implementation plan / verification plan
- 等待用户确认后再改代码
- 数据操作必须 dry-run 先行
- 保持旧基线不动

验证：
    <verification command>

交付：
按 docs/agent-workflow/agent-task-protocol.md 输出 delivery report。报告必须包含：
- root cause / 设计摘要
- 改动内容
- 验证结果
- 是否触碰基线数据
- 风险提示
- 下一步建议

[skill: diagnose]
[skill: zoom-out]
[skill: verify]
