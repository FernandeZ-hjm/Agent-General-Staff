# AGS v3 Architecture

Thin AGS (contract v3): governance is a thin shell at the enforcement point,
not a process bus. Five crates; one deep module.

## One kernel, three projections

```text
ags (CLI)        ags mcp (2 tools)        ags-policy (hook decisions)
        \              |              /
         +--- ags-kernel (the only deep module) ---+
                     /         |         \
              sealed decide/apply | evidence log | permission matrix
              capabilities.lock   | memory closure | projection
```

- `ags-kernel` owns all policy: the `ags.toml` matrix (allow/ask/deny/sealed),
  sealed transactions (5 states), the append-only content-addressed evidence
  log, the hash-pinned capability lock, evidence-chain memory closure, and the
  ownership-safe projection (byte-preserving, atomic no-replace, tamper
  detection). It also ships the `ags-policy` hook binary.
- `ags-task-contract` owns the ≤13-field canonical card, its validator
  (closure mapping G/AC/V/EV, protected paths), the structured command runner
  (no shell interpolation), and `ags run` prepare/verify/close with the review
  escalation matrix.
- `ags` CLI / internal MCP companion translate interfaces only. They never own policy,
  workspace identity, sealing, verification or receipts (lark-cli convention:
  one typed core, thin adapters, risk labels per command).
- `ags-release` owns the public projection boundary: the typed public spec,
  sensitivity scan, content-addressed plan and transactional apply from A
  into B, plus promotion verify (S==A, B integrity). The ags CLI keeps
  `release:*` blocked for workspace-local tasks.

## Control flow

```text
task card (≤13 fields)
  -> ags run --task <card>          # prepare: validate + matrix + review level
  -> host executes (ags-policy PreToolUse decides every tool call)
  -> ags run --verify               # structured verify commands + governance
  -> review (Light/Medium/Heavy escalation)
  -> ags run --close --report ...   # evidence-chain closure + memory pointer

sealed operations (init/upgrade/update/govern.*)
  -> decide seals a single-use action_ref
  -> ags apply consumes it once     # replay/tamper/cross-binding fail closed
```

`upgrade` binds the full machine/cache/target environment. Native five-binary
activation is journaled, switches the user-facing `ags` executable last, and
restores the previous verified set when an unreceipted journal is observed.

## Workspace identity

Single-machine: nearest ancestor `ags.toml`. Multi-root MCP: explicit
workspace context → unique MCP root → unique bound workspace containing the
adapter cwd → otherwise `workspace_required` / `workspace_ambiguous`. HOME,
recent projects and fuzzy matches are never identity authorities. The binding
hash is embedded in every action_ref.

## Evidence

`.ags/evidence/events.jsonl` — append-only, one JSON object per line,
content-addressed (sha256 over canonical fields) and chained (prev_sha256).
Rotation by day and at 10 MiB (gzip archives kept). `ags log` / `ags status`
derive reports; `ags run --close` verifies the chain interval for the task and
appends the closure event, whose id is the memory pointer. Sealed receipts
carry the same hashes, so both chains cross-check.

## Capability integrity

`.ags/capabilities.lock` pins each body to a content hash. Exact routing
(id + hash), no fuzzy fallback, no staleness machine. Only `ags update` /
skill install / remove refresh the lock; drift is a hard finding in
`ags check`, never a silent refresh.

## Workspace boundaries

Workspace A is the private development authority. Stable, public, remotes,
installed runtimes, tags, packages, and releases change only through
separately authorized promotion or release work; `release:*`/`promotion:*`
cannot be consumed from a workspace-local task. A local verification pass
never implies promotion or release completion.
