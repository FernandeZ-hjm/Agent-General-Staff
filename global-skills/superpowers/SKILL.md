---
name: superpowers
description: >
  Bundled playbook dependency for auto-brainstorm, auto-debug, auto-verify,
  diagnose, and grill-with-docs. Not a user-facing skill — installed to satisfy
  @../superpowers/playbooks/... references from required skills.
disable-model-invocation: true
---

# Superpowers (Bundled Dependency)

This directory is a bundled playbook dependency installed alongside the agent-governance suite.
It provides the playbooks referenced via `@../superpowers/playbooks/...` by the following
required skills:

- `auto-brainstorm` → `playbooks/brainstorming/SKILL.md`
- `auto-debug` → `playbooks/systematic-debugging/SKILL.md`
- `auto-verify` → `playbooks/verification-before-completion/SKILL.md`
- `diagnose` → `playbooks/systematic-debugging/SKILL.md`
- `grill-with-docs` → `playbooks/brainstorming/SKILL.md`

This SKILL.md is a minimal wrapper. The playbook files under `playbooks/` are the canonical
references. Do not invoke `/superpowers` directly unless the full Superpowers skill with all
playbooks is installed.
