# Agent Governance Suite Protocol

Current product version: **v0.4.21**. The public control contract is
**contract v3** — the "Thin AGS" hard cut.

## Core model

```text
host interprets the request
  -> task card (≤13 fields) or typed sealed Operation
  -> ags run (prepare -> execute -> verify -> close)   [default path]
  -> decide / apply                                    [sealed path only]
  -> evidence log (append-only, content-addressed, chained)
```

AGS does not interpret raw natural language, schedule Agents, or proxy
third-party MCP servers. The host owns semantic interpretation. The kernel
(`ags-kernel`) owns the permission matrix, sealed transactions, workspace
identity, evidence and capability integrity. Everything else is a thin
adapter: `ags` (CLI), `ags-mcp` (two tools), `ags-policy` (hook decisions),
`ags-host` (lifecycle).

## Product commands

```text
mcp  init  run  apply  check  test  log  status  doctor  update  govern  schema
```

Risk classes are declared per command: `read` / `write` / `high-risk-write`.
Only `ags apply` mutates; every `write` command seals a plan first.

## Sealed registry (the only typed operations)

```text
init  update  govern.skill.install  govern.skill.remove  govern.host_projection
```

Five states: `blocked / planned / applying / receipted / risk-escalated`.
Replay, tamper, and cross-binding use fail closed.

## Permission matrix (`ags.toml`)

Three decisions only: `allow / ask / deny`, plus `sealed` for the registry.
Patterns are `surface:action` with `*` wildcards; most-specific match wins,
deny beats ask beats allow on ties, no match is deny (fail closed). Write
boundaries escalate: deny-path hits are hard deny; outside-allowed hits never
stay below ask.

## Capability and memory boundaries

`~/.agents/skills/` is the installed Skill source of truth; the machine lock
owns body readiness. Project `.ags/capabilities.lock` is audit-only and never
routes. Host-selected Skills win; `ags route` is consulted only when the host
has no clear match, rejects ties, and returns an unready match as `candidate`
rather than `skill`. There is no staleness state machine or per-host snapshot.
Third-party MCP registration remains advice-only. Project memory records
verified closure facts and never expands execution authority.

## Authority and release boundary

This private workspace A is the implementation and protocol authority. Stable,
public, remotes, tags, packages and installed runtimes change only through an
explicit downstream promotion or release task. `release:*` / `promotion:*`
require independent authorization and cannot be consumed from a
workspace-local task.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTFLAGS="-D warnings" cargo test --workspace
cargo build --release
ags check governance --workspace . --format json
git diff --check
```
