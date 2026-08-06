# Agent General Staff 0.4.13 — Public Edition

@AGENTS.md

Claude Code is an execution Host for this public distribution. Start AGS work
with `ags_preflight`. Consume a validated task card or an explicitly bounded
direct-edit request; do not infer task level, permission mode, review gate, or
verification gate from raw user language.

For input beginning with `## 任务卡`, validate and dispatch the exact card
without regeneration. A Host Plan-mode final artifact is the canonical task
card; execution begins only after leaving Plan mode while preserving its
`task_card_hash`.

Read only the documents linked from `AGENTS.md` that are relevant to the current
request.
