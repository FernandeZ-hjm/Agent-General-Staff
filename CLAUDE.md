# Agent General Staff 0.3 — Public Edition

@AGENTS.md

Claude Code is an execution host for this public distributable edition. Consume
a validated task card or an explicitly bounded direct-edit request; do not infer
task level, permission mode, review gate, or verification gate from raw user
language.

For input beginning with `## 任务卡`, validate first and dispatch the exact card
without regeneration. A host Plan-mode final artifact is the canonical task
card; execution begins only after leaving Plan mode and preserving its
`task_card_hash`.

Use `ags_preflight` before AGS work. For implementation details, read only the
relevant documents linked from `AGENTS.md`; do not preload the full protocol.
