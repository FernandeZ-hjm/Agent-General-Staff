# Heavy Claude Code Task Card Template

Use this global fallback template only when no project task-card protocol is available, the task is cross-repo, an external agent will execute it, or Claude Code cannot read project files.

---

~~~~markdown
## 任务卡

读取并遵守：
- 本任务卡内的约束
- 当前工作目录中的 AGENTS.md / CLAUDE.md / README / CONTRIBUTING，如存在
- 当前任务相关文件

执行者：Claude Code

任务级别：Heavy

任务：
{one_sentence_task_summary}

背景：
{why_this_task_exists}

相关路径：
- `{path_1}`
- `{path_2}`
- `{path_3}`

本次任务相关文件：
- `{task_file_or_doc_1}`
- `{task_file_or_doc_2}`
- `{task_file_or_doc_3}`

适用治理文档：
- 无 / `{governance_doc}`

目标：
1. {goal_1}
2. {goal_2}
3. {goal_3}
4. {goal_4}

非目标：
- {non_goal_1}
- {non_goal_2}
- {non_goal_3}

实施要求：
- 先阅读现有代码、目录结构、配置、现有测试和文档。
- 先输出 root cause / 当前结构理解 / 风险点 / 设计方案 / 实施计划 / 验证计划。
- 等待确认后再改代码或生成新产物。
- 不安装新依赖，除非先说明必要性并等待确认。
- 不做破坏性删除；所有删除类动作必须先实现为 quarantine / disable / exclude / dry-run。
- 涉及数据、向量库、历史产物、索引、collection 或 baseline 时，必须保证可回滚、可审计、可对比。
- 所有自动判断必须留下审计证据。
- 如发现需求与现有代码事实不一致，先报告，不要自行扩大范围。

验证：
```bash
{verification_command_1}
{verification_command_2}
{verification_command_3}
```

交付：
完成后必须先读取并使用 `claude-delivery-report` skill，按其模板输出简洁交付报告。

如果本任务生成了详细审计报告、统计结果、manifest 或长日志，不要把全部内容塞进主报告；在主报告的“新增文件 / 输出物”或“风险提示”中引用对应文件即可。

{skill_tags}
~~~~
