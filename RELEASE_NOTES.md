# Agent Governance Suite Release Notes

## Release 0.4.21

0.4.21 is the contract-v3 hard cut — the "Thin AGS" restructure. Governance
moves from a process bus to a thin shell at the enforcement point; the core
invariants do not degrade. The private authority workspace keeps the D1–D7
design contract; it is not part of the public payload.

### What changed

- **Unified product surface**: CLI and MCP are both named `ags`; stdio starts
  with `ags mcp`, reports `v0.4.21`, and exposes only `ags_decide`/`ags_apply`.
- **Agent-driven Host lifecycle**: sealed host projection repairs known client
  registrations, merges AGS-owned hooks, aligns slugs, injects startup memory
  and projects verified closures without transcript inference.
- **Immutable Skill adoption**: local Skill sources are audited, hash-bound to
  the sealed plan, copied into a content-addressed machine store, registered,
  rollback-journaled and routed through the shared machine lock.
- **Public first-install closure**: every signed platform asset carries all
  five binaries plus the validated public `ags-skills/` runtime profile;
  `ags setup --source-root <runtime>` establishes the v3 install record and
  machine lock before workspace adoption.

- **5 crates instead of 12**: `ags-kernel` (sealed decide/apply, evidence
  log, permission matrix, capabilities.lock, memory closure, ownership-safe
  projection, `ags-policy` binary), `ags-task-contract` (≤13-field card,
  validator, structured runner, `ags run`), `ags-cli`, `ags-mcp`,
  `ags-release`.
- **`ags.toml`**: one policy file per workspace (matrix, boundaries, sealed
  list, verify commands, review escalation, hosts, capability sources).
- **`ags run`**: the one task command — prepare / `--verify` / `--close`;
  review level auto-derived (Light default; ask-hit/fanout/boundary-crossing
  → Medium; sealed/promotion/release → Heavy).
- **Evidence log**: append-only, content-addressed, chained events with
  day/10 MiB rotation; `ags log` / `ags status` derive reports; closure
  event = memory pointer.
- **Capabilities**: hash-pinned lock instead of per-host snapshots; no
  staleness machine; drift is a hard finding, never a silent refresh.
- **`ags-policy` hooks**: allow/ask/deny/sealed decisions for Claude Code /
  Codex / Cursor / CodeBuddy; OMP/DSH ride MCP; fail-open (D5).
- **Task cards**: ≤13 fields; Execution mode/topology/effort/delegation
  fields, review-target maintenance and multi-file receipts are gone.
- **CLI follows the lark-cli three-layer convention**: risk labels
  (read/write/high-risk-write) per command, quickstart help, single-op
  inspection via `ags schema OPERATION`.

### Removed (hard cut)

v2 setup saga / agent / govern task / capability snapshot / memory close / old
command aliases and compatibility handlers. Unknown commands, options and
fields return the standard structured unknown/invalid response.

### Preserved (red lines)

Sealed decide/apply with replay/tamper/cross-binding fail-closed; authority
ceiling via matrix + card tuple; content-addressed evidence; independent
Heavy review; exact capability routing; memory closure; A→S→B promotion
boundary and release scopes (independent authorization only).
