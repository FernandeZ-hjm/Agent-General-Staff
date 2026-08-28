# Agent Governance Suite

[中文](README.md)

Agent Governance Suite (AGS) v3 is a **thin-shell governance suite for coding
agents**: governance lives at the enforcement point — invisible day to day,
visible exactly when a dangerous action appears.

## In one line

```text
ags run --task card.md          # validate + matrix decision + review level
   (host executes; ags-policy hooks decide every tool call silently)
ags run --task card.md --verify # structured verification commands (no shell)
ags run --task card.md --close --report report.json --effective heavy
                                # evidence-chain closure + memory pointer
```

Dangerous operations stay sealed: `ags upgrade`, `ags update`, and the other sealed commands
emit a single-use `action_ref`; `ags apply <ref>` consumes it exactly once
(replay / tamper / cross-binding fail closed).

## Three files to know AGS

- **`ags.toml`** — the single policy file per workspace: permission matrix
  (`surface:action` → allow/ask/deny), write boundaries, sealed list, verify
  commands, review escalation, host registrations, capability sources.
- **`.ags/evidence/events.jsonl`** — append-only evidence log; every event is
  content-addressed and chained (prev_sha256); rotates by day + 10 MiB.
- **`.ags/capabilities.lock`** — hash-pinned capabilities; exact routing
  (id + hash), no staleness machine; only `ags update` / install / remove
  refresh it.

## Command surface (lark-cli three layers, risk class per command)

| Layer | Commands |
|---|---|
| Shortcuts | `ags run` (the one task command), `ags init`, `ags doctor` |
| Typed commands | `ags check`, `ags test`, `ags log`, `ags status`, `ags setup`, `ags upgrade`, `ags update`, `ags govern skill install/remove`, `ags schema` |
| Sealed escape hatch | `ags apply <ACTION_REF>` (the only mutation surface) |

Risk labels: `read` (zero writes) / `write` (sealed plan + apply) /
`high-risk-write` (sealed + boundary).

## Hosts

| Host | Policy channel |
|---|---|
| Claude Code / Codex / Cursor / CodeBuddy | full hooks (PreToolUse / PermissionRequest / PostToolUse → `ags-policy`) |
| OMP / DSH | degraded mode via MCP `ags_decide` / `ags_apply` |

Hook failure is fail-open to the host default (D5); `ags doctor` surfaces the
degradation.

## Five crates

`ags-kernel` (the only deep module) · `ags-task-contract` (card + `ags run`)
· `ags-cli` · `ags-mcp` · `ags-release` (the public projection boundary).
Users enter through `ags` / `ags mcp`; adapters are thin projections of the kernel and own
no parallel domain logic.

## Quick start

```bash
cargo build --release
ags setup --source-root .                   # verify five binaries + initialize v3 Skills/lock
ags upgrade check                           # signed stable release check
ags upgrade plan --workspace .              # sealed runtime plan
ags apply <ACTION_REF> --workspace .         # atomic activation
ags upgrade verify <ACTION_REF> --workspace .
ags init --workspace . --slug my-project     # sealed plan
ags apply <ACTION_REF> --workspace .         # consume once
ags govern host-register --id my-host --surface cli --workspace .
ags apply <ACTION_REF> --workspace .         # consume host registration
ags doctor --workspace .                     # health
```

The host-Skill migration sequence after A-to-S promotion is captured in
[`docs/ags-public-private-update-handoff.md`](docs/ags-public-private-update-handoff.md).

## Public contract

Architecture and boundaries: `docs/architecture.md` · commands and sealed
operations: `ags --help` / `ags schema` · official host Skills:
`ags-skills/` · release changes: `RELEASE_NOTES.md`.

Private protocols, proposals, and promotion runbooks remain only in authority
workspace A. They are neither projected nor an implicit dependency of the
public installation.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTFLAGS="-D warnings" cargo test --workspace
cargo build --release
ags check governance --workspace . --format json
git diff --check
```

## License

GPL-3.0-only. See `LICENSE`.
