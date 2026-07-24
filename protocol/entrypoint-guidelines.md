# Agent Entrypoint Guidelines

`AGENTS.md` and `CLAUDE.md` are startup maps, not protocol encyclopedias. They
consume context on every relevant session, so recurring invariants stay inline
and conditional procedures live in canonical documents or skills.

## Required shape

- Keep the repository `AGENTS.md` near 100 lines when practical.
- Keep every `CLAUDE.md` below 200 lines.
- State repository identity, mandatory startup gate, hard safety boundaries,
  primary verification commands, and a short “read when relevant” map.
- Put multi-step task-card, release, migration, host-operation, or subsystem
  procedures in focused documents or skills.
- Prefer one ownership direction: `CLAUDE.md` may import a concise `AGENTS.md`;
  `AGENTS.md` must not import `CLAUDE.md`.
- Avoid duplicating details already owned by `WORKSPACE.md`,
  `AGENT_SUITE_PROTOCOL.md`, or `protocol/`.
- Imports are not a context-saving mechanism: use them only for rules that must
  always load. Plain links are the default for conditional material.
- Use nested or path-scoped instructions when a rule applies only to one
  subtree. Avoid conflicting instructions across host entry files.

## Mechanical checks

- Entry files remain below their line budgets.
- No `AGENTS.md` ↔ `CLAUDE.md` import cycle exists.
- The entry map points to existing canonical files.
- `ags setup` installs the shared `ags-core.md`, `ags-task-handoff.md`, and
  `host-operations.md` modules under `$HOME/.agents/rules`; host-global entry
  files remain operator-controlled and reference these modules explicitly.
- Detailed command tables and historical release procedures do not live in
  startup entry files.
- A new host-specific rule belongs in the host entry, a path-scoped rule, or the
  relevant protocol document—not all three.

## Sources

- OpenAI, [Harness engineering: leveraging Codex in an agent-first
  world](https://openai.com/index/harness-engineering/) — use a short
  `AGENTS.md` as a map to deeper sources of truth.
- Anthropic, [How Claude remembers your
  project](https://code.claude.com/docs/en/memory) — concise, specific
  `CLAUDE.md`; target under 200 lines; move conditional procedures elsewhere.
- GitHub, [Adding custom instructions for GitHub Copilot
  CLI](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions)
  — scoped discovery, relative references, and avoiding conflicting merged
  instructions.
- GitHub, [Using custom instructions to unlock the power of Copilot code
  review](https://docs.github.com/en/copilot/tutorials/customize-code-review)
  — short, focused, structured instruction files and path-specific splits.
