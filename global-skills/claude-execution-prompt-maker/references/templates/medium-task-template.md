# Medium Claude Code Task Card Template

Use this global fallback template only when no project task-card protocol is available, the task is cross-repo, an external agent will execute it, or Claude Code cannot read project files.

---

~~~~markdown
## 任务卡

读取并遵守：
- 本任务卡内的约束
- 当前工作目录中的 AGENTS.md / CLAUDE.md / README / CONTRIBUTING，如存在
- 当前任务相关文件

执行者：Claude Code

任务级别：Medium

任务：
{one_sentence_task_summary}

背景：
{task_context}

相关路径：
- `{path_or_module_1}`
- `{path_or_module_2}`
- `{path_or_module_3}`

本次任务相关文件：
- `{task_file_or_doc_1}`
- `{task_file_or_doc_2}`

适用治理文档：
- 无 / `{governance_doc}`

目标：
1. {goal_1}
2. {goal_2}
3. {goal_3}

非目标：
- {non_goal_1}
- {non_goal_2}
- {non_goal_3}

实施要求：
- 先阅读相关代码、配置、测试和局部文档。
- 先简要说明 root cause / 当前行为 / 修改方案，再开始改代码。
- 只做与本任务直接相关的改动。
- 不安装新依赖，除非先说明必要性并等待确认。
- 不做破坏性删除。
- 如发现需求与现有代码事实不一致，先报告，不要自行扩大范围。

验证：
```bash
{verification_command_1}
{verification_command_2}
{verification_command_3}
```

交付：
完成后必须先读取并使用 `claude-delivery-report` skill，按其模板输出简洁交付报告。

{skill_tags}
~~~~
