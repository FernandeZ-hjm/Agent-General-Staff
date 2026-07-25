# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)]()

[中文](README.md) | [English](README.en.md)

AGS is a governance control plane for multi-agent software development. It
governs admission, authorization, policy, verification, receipts, capability
snapshots, and memory closure. It does not assemble agent teams or provide a
task queue, scheduler, parallel executor, or agent negotiation runtime.

The current latest product release is **v0.3.1**, licensed
**GPL-3.0-only**.

## What AGS governs

Codex, Claude Code, Cursor, and OMP can all work in one repository. Strong host
capabilities do not by themselves create clear engineering boundaries. AGS
turns a drifting natural-language execution process into a verifiable chain:

```text
project preflight
  → host submits a typed proposal
  → AGS validates authority, policy, and exact capabilities
  → host executes
  → verification, receipt, and delivery closure
```

The boundary is deliberate:

- The host interprets user intent. AGS does not parse raw language to guess a
  skill or permission.
- AGS validates a closed `HostRouteProposal` with exact capability identifiers
  and closed machine actions.
- `ags_route_request` is read-only. The only effectful MCP tool is
  `ags_apply_action`, which consumes a one-shot, session-bound lease.
- The Runner prepares a structured LaunchPlan; it never claims execution or
  verification.
- Missing contracts, capability state, authentication, snapshots, or evidence
  fail closed.

## Quick start

### Install from source

```bash
git clone https://github.com/FernandeZ-hjm/Agent-General-Staff.git
cd Agent-General-Staff
bash scripts/install.sh
```

Or build directly:

```bash
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
ags --version
```

### MCP launcher

Release binaries are available through the npm launcher:

```bash
npx -y @agent-governance-suite/mcp
```

The launcher selects the matching OS/architecture asset, verifies
`SHA256SUMS`, and starts the MCP stdio adapter without a shell. npm publication
uses GitHub OIDC trusted publishing and stores no long-lived npm token.

### Recommended flow

```bash
ags setup --yes --force
ags agents govern --agent codex --apply
ags agents govern --agent claude-code --apply
ags agents govern --agent omp --apply
ags init --target .
ags doctor --target .
ags verify --scope local
```

Mutating lifecycle commands continue to require dry-run review or explicit
`--apply` / `--yes`.

## Stable human CLI

v0.3.1 preserves the complete v0.3.0 Clap command tree, arguments, aliases,
defaults, stdout/stderr behavior, exit codes, and JSON schemas. The only
expected change is:

```text
ags --version
0.3.0 → 0.3.1
```

The human-facing top-level commands remain:

| Command | Purpose |
|---|---|
| `ags setup` | Install or upgrade the local governance kernel |
| `ags onboarding` | Assess and confirm public onboarding one item at a time |
| `ags init` | Project AGS protocol and entrypoint onboarding |
| `ags doctor` | Diagnose runtime, host, project, and capability state |
| `ags agents` | Scan, govern, and verify agent hosts |
| `ags capability` | Capability inventory, snapshots, and host visibility |
| `ags skill` | Skill body, entrypoint, update, and rollback governance |
| `ags update` | Update kernel, runtime, hosts, skills, and project projections |

Internal MCP/Machine CLI surfaces are not promoted into new human commands.
Existing scripts and project entry files require no migration.

## v0.3.1 Workspace Service

MCP stdio is now a thin adapter:

```text
MCP stdio adapter
        ↓ connect-or-start
canonical workspace path → one AGS workspace daemon
        ├── Codex session
        ├── Claude Code session
        ├── Cursor session
        └── OMP session
```

- The daemon key is only the canonical workspace path, never the host.
- One atomic capability bundle is shared inside a workspace, avoiding repeated
  full-directory scans.
- Every conversation keeps an independent `session_id`, preflight binding, and
  DecisionLease.
- Snapshot refresh validates a candidate bundle, atomically replaces it, and
  publishes a new hash. A new preflight accepts the new hash immediately.
- Cross-host, cross-session, cross-workspace, and replayed leases are rejected.
- A disconnected client does not kill the daemon; an idle daemon with no
  sessions is recycled after a fixed TTL.
- Binary upgrades are stop-before-restart. Two versions cannot serve one
  workspace concurrently.

This is an internal runtime improvement and introduces no new user command.

## Twelve primary boundaries

| Crate | Responsibility |
|---|---|
| `ags-platform` | Paths, filesystem, processes, hashing, atomic writes |
| `ags-workspace-facts` | Canonical workspace, discovery, configuration facts |
| `ags-host-integration` | Codex, Claude Code, Cursor, and OMP adapters |
| `ags-capability-governance` | Inventory, exact resolution, skill lifecycle, snapshots |
| `ags-task-contract` | Task cards, compilation, validation, handoff contracts |
| `ags-governance-decision` | Typed proposals, policy, authority, route decisions |
| `ags-session` | Workspace service, sessions, preflight, leases |
| `ags-evidence` | Receipts, delivery reports, evidence models |
| `ags-verification` | Doctor, local/release verification, sync checks |
| `ags-lifecycle` | Setup, init, onboarding, update, rollback |
| `ags-cli` | Compatibility-preserving human adapter |
| `ags-mcp` | MCP conversion, connection, and error mapping |

Governance rules do not live in CLI or MCP adapters, and the release does not
retain a second legacy routing implementation.

## Support matrix

| Capability | Codex | Claude Code | Cursor | OMP |
|---|---:|---:|---:|---:|
| Shared workspace daemon | Yes | Yes | Yes | Yes |
| Independent session/preflight/lease | Yes | Yes | Yes | Yes |
| Snapshot refresh and reconnect | Yes | Yes | Yes | Yes |
| Skill entry probing | Yes | Yes | Yes | Yes |
| Native host memory lifecycle | Yes | Yes | Host-limited | Yes |

Four-host E2E covers a single daemon per workspace, cross-project isolation,
snapshot refresh, stdio reconnect, lease replay and boundary rejection, idle
recycle, binary restart, and damaged-bundle diagnostics.

Source CI covers Linux, macOS, and Windows. Tagged releases build Apple Silicon
and Intel macOS, x86_64 and ARM64 Linux, and x86_64 Windows assets.

## Security and supply chain

- Fixed argv; governance actions do not use shell-string composition.
- Task contracts, policies, paths, symlinks, hashes, and one-shot leases are
  fail-closed.
- The onboarding manifest is pinned to an immutable Git commit and an expected
  SHA-256; the packaged reviewed manifest is only an offline fallback.
- Releases include `SHA256SUMS` and provenance; npm uses a trusted publisher.
- `cargo-deny`, strict Clippy, release-boundary checks, and the public/private
  guard are release gates.
- Public tracked payload excludes machine paths, runtime state, third-party
  skill bodies, and private implementation.

## Limits

AGS is not:

- an AutoGen/LangGraph-style execution orchestrator;
- an agent task queue, resource quota manager, or parallel scheduler;
- a natural-language proof that the host understood the user correctly;
- a substitute for real host execution and project verification.

A typed proposal proves that the proposal is internally legal. It cannot prove
that the host interpreted the user's intent correctly. Real delivery still
requires host execution, project checks, and evidence closure.

## Verification

For normal use:

```bash
ags doctor --target .
ags verify --scope local
```

For source contributions:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --release --locked
bash scripts/verify.sh
git diff --check
```

Release gates also verify the v0.3.0 human and Machine CLI fixtures,
workspace daemon/session/lease/snapshot E2E, product versus schema/history
version groups, public release boundaries, the npm launcher, and private marker
absence.

### 0.3.1 Release Order

1. The exact public-safe `main` commit passes every GitHub Action.
2. Cargo, npm, suite manifest, MCP `serverInfo`, README, and Release Notes all
   declare `0.3.1`; compatibility wire/schema identifiers remain unchanged.
3. Create annotated tag `v0.3.1` on that exact commit.
4. The tag workflow publishes five platform assets, `SHA256SUMS`, provenance,
   and the GitHub Release.
5. Dispatch the OIDC npm workflow and confirm registry version `0.3.1` with
   `latest` pointing to `0.3.1`.

Pushing `main` is not equivalent to completing the tag, Release, or npm
publication.

## Documentation

- [Architecture](docs/architecture.md)
- [MCP protocol](protocol/mcp-server.md)
- [Task protocol](protocol/agent-task-protocol.md)
- [Skill governance](protocol/skill-governance.md)
- [Release notes](RELEASE_NOTES.md)
- [Security policy](SECURITY.md)
- [Commercial and GPL notes](COMMERCIAL.md)

## License

AGS is licensed under the **GNU General Public License v3.0 only
(GPL-3.0-only)**.

You may use, study, modify, and distribute it. Distribution of AGS or a
derivative must provide the corresponding complete source under GPL-3.0-only.
Internal use alone is not distribution. Third-party licenses and attributions
are documented in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
