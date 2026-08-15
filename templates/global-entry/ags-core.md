# AGS Core Agent Rules

## First-principles priority

Derive decisions from goals, facts, constraints, invariants, cost, and
verifiable evidence. Skills and MCP servers provide methods and execution
surfaces; they do not expand authority or replace host judgment.

## Contract-v2 entry

The host retains conversation context, performs the only natural-language
interpretation, and creates one typed Operation. Use the human/Machine CLI or
the two-tool MCP surface (`ags_decide`, `ags_apply`). Do not send raw user text
to AGS. Read-only Operations return bounded structured results. Effectful
Operations first return a sealed, binding-specific plan and `action_ref`.

Workspace identity is request-scoped. Prefer an explicit canonical workspace;
otherwise one unique MCP root or adapter cwd may resolve it. Never infer from
HOME, recent projects, fuzzy matching, or managed-projects.

## Execution authority

- Validate an existing `## 任务卡` before raw-request classification.
- A task card's execution mode, topology, delegation, mutation boundaries, and
  commit rules override playbook defaults.
- A playbook may narrow authority but cannot require commits, serialize an
  authorized parallel topology, or add external writes.
- Heavy work requires an independent reviewer who did not implement the diff.
- Releases, remotes, credentials, destructive actions, and protected paths are
  independent stop conditions.

Before claiming completion, use the `verification-before-completion` playbook
or equivalent fresh evidence. Preserve unrelated working-tree changes.
