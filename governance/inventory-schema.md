# Agent Toolchain Inventory Schema

The current governance kit stores observed local state and source-resolution decisions as YAML under `tools/agent-toolchain/data/`.

These files are operational artifacts, not universal truth across machines. A target machine should regenerate or review them from its own local filesystem before accepting updates.

## Runtime Path Resolution

Data YAML files under `tools/agent-toolchain/data/` record paths as captured on the machine where the inventory was originally generated (e.g., `/Users/a92550/.agents/skills/...`). These are **inventory record paths**, not runtime paths.

The three governance scripts (`check`, `propose`, `accept`) resolve all `local_canonical_path` and file-access paths at runtime via a `resolve_local_path()` function. This function remaps any `/Users/<username>/` prefix to the current machine's `$HOME`, so the same YAML data works across different machines without modification.

**Rule**: YAML data files store inventory paths as-is. Scripts resolve them to `$HOME` at runtime. Never hardcode a specific username path in script logic.

## Files

- `agent-toolchain-inventory.yaml`: observed local skills, plugins, plugin-cache skills, and control-plane config fingerprints.
- `agent-toolchain-canonical-map.yaml`: local canonical identity choices and duplicate/shadow relationships.
- `agent-toolchain-source-resolution.yaml`: upstream/source evidence, confidence, and recommended action.
- `agent-toolchain-source-resolution-summary.md`: human-readable source-resolution summary.
- `agent-toolchain-sync-groups.yaml`: grouping of entries by update/control model.
- `agent-toolchain-update-plan.yaml`: allowed operations, forbidden operations, and next gate per canonical entry.
- `agent-toolchain-acceptance-log.yaml`: human decisions after proposal/dry-run/patch review.

## Canonical ID Rules

- `skill/<name>`: local canonical skill entry.
- `plugin/<name>`: plugin parent, managed through plugin mechanism.
- `skill/plugin-cache/<plugin>/<skill>`: child skill inside plugin cache; never sync independently.

## Required Safety Fields

Each entry that can be checked or proposed should include:

- `canonical_id`
- `local_canonical_path`
- `source_status`
- `update_strategy`
- `upstream_candidates`
- `confidence`
- `allowed_operations`
- `forbidden_operations`
- `next_gate`

## Conservative Defaults

When in doubt:

- Use `local_canonical_source_unknown` for local skills.
- Use `local_overlay` for wrappers, shortcuts, or routing skills.
- Use `plugin_cache_managed` for plugin parents.
- Use `plugin_parent_managed` for plugin-cache child skills.
- Use `system_local_canonical` for Codex/system-managed skills.

Unknown source is not a blocker for governance. It only blocks `propose` and `accept`.

## Record-Only Plugin Strategies

Use `plugin_runtime_record_only` for plugin parents whose upstream source is private, authentication-gated, or otherwise not useful for anonymous `git ls-remote` checks.

Use `plugin_cache_record_only` for child skills inside those plugin caches.

Record-only plugin entries remain in the inventory for local visibility and version tracking, but they do not perform remote Git/HTTP checks and should not appear in the human-review section unless another explicit error is present. Updates must go through the owning runtime plugin mechanism, not direct cache edits.
