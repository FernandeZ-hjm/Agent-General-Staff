---
name: auto-verify
description: >
  当用户或AI声称工作完成、已修复、通过测试、准备提交/PR时自动触发。
  触发场景："做完了"、"好了"、"完成了"、"done"、"fixed"、"搞定了"、
  commit前、PR前。不触发：一般对话中的"好的"、"OK"表示同意。
---

# auto-verify

当声称工作完成时，自动验证：确定验证命令 → 执行 → 读取完整输出 → 确认通过后才允许声称完成。

验证通过后，按全局临时文件清理规则，清理本次任务产生的临时文件。

$ARGUMENTS

@../superpowers/playbooks/verification-before-completion/SKILL.md
