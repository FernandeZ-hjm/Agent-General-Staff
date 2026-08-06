# Agent Governance Suite Workspace

This checkout is the self-contained public distributable edition of Agent
Governance Suite (AGS), version **v0.4.13**.

Current source candidate: **v0.4.13**. Latest published release: **v0.4.13**.

## Role and authority

This repository is the reviewed public-safe projection. It contains the public
Rust workspace, dual npm entrypoints, protocols, typed manifests, generated
command Skills, and release workflows. It builds and runs without private
infrastructure.

Private A is the development/protocol authority; stable S is its exact
fast-forward integration point; public B is produced by one hash-bound
projection transaction. B-owned entry documents and workflows are fixed by
reviewed digests. No directory-name convention substitutes for those proofs.

## Twelve authoritative modules

| Module | Owns |
|---|---|
| `ags-platform` | paths, containment, hashing, atomic files, process facts |
| `ags-workspace-facts` | canonical workspace discovery and preflight facts |
| `ags-host-integration` | Host identity, roots, probes, and native adapter facts |
| `ags-capability-governance` | catalog, installed Skill state, exact routes, snapshots |
| `ags-task-contract` | task-card compile/validate and launch preparation |
| `ags-governance-decision` | typed proposals, policy, route, and decision contracts |
| `ags-session` | workspace daemon, client binding, one-shot action store |
| `ags-evidence` | receipts, delivery closure, evidence integrity |
| `ags-verification` | doctor, projections, release boundary, VerificationBundle |
| `ags-lifecycle` | setup, maintenance transactions, activation, recovery |
| `ags-cli` | human and Machine CLI adaptation |
| `ags-mcp` | typed MCP wire and session adaptation |

Dependencies point inward. CLI and MCP do not own domain rules. Each fact has
one authority: the third-party catalog describes discoverable sources,
InstalledSkillRecord describes installed Skills, Host observations describe
activation, and signed release state describes AGS updates.

## Closed maintenance model

All mutating maintenance uses `Intent -> Plan -> Apply -> Verify -> Receipt`,
with automatic recovery on failed activation or verification. Plan hashes bind
the source and target bytes. Required suite Skills are projected transactionally
to five supported Skill Hosts; renamed and retired AGS-owned links are handled
inside the same rollback boundary.

Retired bootstrap/update/registry implementations and old JSON read models are
not compatibility authorities. Migration is one-way into the current typed
state.

## Public projection boundary

The public edition includes source, protocols, empty templates, canonical
catalog metadata, generated suite/registry views, and release automation. It
excludes private or personal Skill bodies, real memory/task archives/receipts,
credentials, machine paths, Host config, build output, and runtime state.

The public capability projection generates exactly eight outputs: suite,
Skill registry, MCP registry, and five command Skill bodies. Recommendations
never enter installed or routable sets by declaration.

## Release proof reuse

The exact A candidate runs one full local gate. The exact B commit runs one full
public CI gate. A content-addressed `VerificationBundle` binds commit, tree,
toolchain, lockfiles, policy, commands, inventory, artifacts, and results.
Promotion, tag, GitHub Release, and npm consume the bundle and run only their
boundary smokes; they do not replay a full workspace suite.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo run -q -p ags-cli -- verify --scope local --target .
./target/release/ags verify --scope release --target .
git diff --check
```
