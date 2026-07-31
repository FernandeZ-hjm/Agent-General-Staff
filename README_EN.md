# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)

[中文](README.md) | [English](README_EN.md)

Agent Governance Suite (AGS) is a **multi-Agent development governance control
plane**. It owns admission, authorization, policy, capability snapshots,
verification, receipts, and memory closure. It does not schedule Agent teams
and is not a task queue, parallel executor, or multi-Agent negotiation runtime.

This repository is the public AGS distribution, licensed **GPL-3.0-only**. The
current source candidate is **v0.4.1**; the latest published release is
**v0.4.0**.

## v0.4.1 governance flow

```text
human request
  -> host interprets the full conversation
  -> typed HostRouteProposal
  -> AGS validates closed fields
  -> read-only ags_route_request
  -> host-native action or explicit ags_apply_action
  -> evidence / delivery closure
```

- Natural language is interpreted only by hosts such as Codex, Claude Code,
  Cursor, CodeBuddy-Code, and OMP.
- `ags_route_request` is strictly read-only. It rejects raw requests and has no
  keyword or similarity fallback.
- `ags_apply_action` is the only effectful MCP tool and consumes one fixed,
  server-held action exactly once.
- DirectResponse is exclusive. Otherwise, a proposal may contain at most one
  exact SkillTarget and one closed MachineCliTarget.
- Skill Resolver, Compiler, Policy, Gate, and Runner consume structured
  contracts only.
- Task-card generation requires both an explicit handoff instruction and a
  confirmed handoff contract.

## One AGS per workspace

Each canonical workspace has one long-lived AGS daemon:

```text
canonical workspace
  -> AGS daemon
       -> Codex client session
       -> Claude Code client session
       -> Cursor client session
       -> CodeBuddy client session
       -> OMP client session
```

The stdio process is only a `connect-or-start` forwarding adapter. The workspace
daemon loads one static snapshot per host, while `session_id`,
preflight binding, and DecisionLease remain isolated per client session.
Disconnecting one client does not stop the daemon. Idle recycling and
stop-before-restart executable upgrades are internal service behavior.

This architecture adds **no new user command**. Hosts still launch:

```bash
ags mcp serve --transport stdio
```

## Twelve major modules

The v0.4.1 runtime workspace exposes exactly twelve authoritative Cargo
packages:

| Module | Responsibility |
|---|---|
| `ags-platform` | cross-platform paths, files, processes, hashing, and atomic writes |
| `ags-workspace-facts` | canonical workspace facts, discovery, protocol audit, and preflight facts |
| `ags-host-integration` | Codex, Claude Code, Cursor, CodeBuddy-Code, and OMP integration facts |
| `ags-capability-governance` | capability catalog, skill-body governance, exact resolution, and snapshots |
| `ags-task-contract` | task-card compile/validate, handoff contracts, and non-executing launch preparation |
| `ags-governance-decision` | typed proposals, policy, route, and decision contracts |
| `ags-session` | workspace daemon, client sessions, bindings, and one-shot action storage |
| `ags-evidence` | receipts, delivery closure, and evidence integrity |
| `ags-verification` | bootstrap readiness, doctor, projection, and local/promotion/release verification |
| `ags-lifecycle` | setup, init, onboarding, and update |
| `ags-cli` | current human and Machine CLI adapter |
| `ags-mcp` | thin MCP wire, session connection, and error-mapping adapter |

The former `bootstrap-dry-run`, `capability-registry`,
`delivery-report-validator`, `execution-policy`, `runner`, `skill-governance`,
`suite-doctor`, `task-card-validator`, and `workflow-sync-check`
implementations have moved under their authoritative modules. v0.4.1 retains
only commands, wire/schema types, and re-exports with current callers, not old
aliases or a second package authority. See [WORKSPACE.md](WORKSPACE.md) and
[docs/architecture.md](docs/architecture.md).

## Host support matrix

| Host | MCP / daemon | Skill or command entry | Native memory closure | Current verification |
|---|---|---|---|---|
| Codex | supported | global/project skills | SessionStart / Stop Guard / SessionEnd adapter | native MCP registration probe + lifecycle/MCP process E2E |
| Claude Code | supported | `/ags` and skills | SessionStart / Stop Guard / SessionEnd adapter | native MCP connection probe + lifecycle/MCP process E2E |
| OMP | supported; may reuse Codex config | native/shared skills | three-event OMP lifecycle extension | native RPC discoverability probe + lifecycle/MCP process E2E |
| Cursor | supported | host/project skill projection | native sessionStart / sessionEnd / stop hooks | native `cursor-agent mcp list` read-only probe + lifecycle/MCP process E2E |
| CodeBuddy-Code | supported | setup-generated configuration snippets | SessionStart / Stop Guard / SessionEnd adapter | native hook schema + lifecycle/MCP process E2E |
| WorkBuddy | MCP onboarding | setup-generated configuration snippets | lifecycle support is not declared | initialization and static/visibility verification |

The E2E suite launches the real `ags` stdio adapter and workspace daemon. It
covers same-workspace sharing, cross-project isolation, reconnects, foreign
lease rejection, snapshot rebind, idle recycling, and upgrades. It is not GUI
automation for each host product.
Cursor memory closure and external MCP registration are independent facts. AGS
can install and verify native lifecycle hooks and inspect registration through
`cursor-agent mcp list`; writing Cursor MCP configuration remains
operator-controlled.

## Current limitations

- AGS can prove that a typed proposal matches governance state; it cannot prove
  that the host understood the user's intent correctly.
- Runner returns a validated LaunchPlan / host handoff only. It does not dispatch
  Agents or claim execution or verification occurred.
- AGS has no task queue, Agent scheduler, resource quota system, or multi-Agent
  negotiation runtime.
- External MCP/CLI registration is usually advice-only. AGS does not run
  third-party installers for the user.
- Codex, Claude Code, Cursor, CodeBuddy-Code, and OMP share one Rust lifecycle
  contract; host files only map native events and output schemas.
- The public edition does not carry private skill bodies, real
  memory/receipt/archive data, or machine-private runtime state.

`ags doctor` checks both runtime health and local current-version conformance.
Drift in an enabled host's runtime, daemon, MCP registration, snapshot, or
workspace lifecycle projection exits 1. The remote latest check is advisory and
offline operation is non-blocking. Migration first verifies a complete
workspace adapter and then removes only AGS-owned user-level lifecycle hooks;
user hooks and MCP configuration are preserved.
`ags setup --lifecycle-hosts` records only explicitly approved hosts, and
`ags init` consumes that set for the explicitly selected current workspace.
The public edition never enumerates or writes the private managed-project fleet.

## Installation

Ordinary MCP users do not need Rust or Cargo:

```bash
npx -y @agent-governance-suite/mcp
```

The npm launcher downloads the matching prebuilt `ags` binary for the current
OS/architecture, verifies `SHA256SUMS`, caches the verified binary, and starts
`ags mcp serve --transport stdio` without a shell.

To install from source:

```bash
cargo install --path crates/ags-cli --locked --force
ags setup --yes --force
```

## First-time onboarding

```bash
ags onboarding plan --host codex
ags onboarding apply --item project-init --plan-hash <HASH_FROM_PLAN> --host codex --yes
ags onboarding verify --host codex
```

`apply` accepts one plan item at a time. The third-party capability manifest is
fixed in the release package; normal setup, preflight, resource reads, routing,
and apply never refresh it over the network.

## Stable command surface

v0.4.1 supports only the current command surface below. Removed legacy
commands, aliases, and plan-only fake actions are not compatibility contracts.

```bash
ags setup --help
ags onboarding --help
ags init --help
ags doctor --help
ags agents --help
ags capability --help
ags skill --help
ags update --help
ags mcp --help
ags memory --help
ags host lifecycle --help
ags task close --help
```

Mutating actions still require explicit `--apply` or `--yes` and remain subject
to existing confirmation, policy, and lease gates.

## Host-internal contracts and execution boundary

`policy`, `project`, `session`, and `run` primarily serve host/MCP Machine CLI
contracts. `task validate/close`, `memory`, `mcp`, and `verify` are also
explicit operator interfaces.

Without an explicit handoff instruction or confirmed handoff contract, the
compiler may return diagnostics but cannot emit an executable card. `ags run`
remains a non-executing preparation surface: it validates the card, resolves
policy, evaluates the gate, and returns a structured
`host_execution_required` plan.

## Verification

```bash
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
ags verify --scope release
git diff --check
```

Public completion cannot be inferred from the exact release manifest
alone. It must also verify twelve-module source topology, bilingual docs, real
MCP E2E, the performance benchmark contract, retired-authority absence, release
assets, and remote CI for the exact public commit.

## License and release

- License: **GPL-3.0-only**
- Source candidate: **v0.4.1**
- Latest published: **v0.4.0**
- Current contract: v0.4.1 human/Machine CLI
- History: v0.3.1 release notes remain historical, not current

Release ordering is fixed:

1. Push the public-safe source to GitHub `main` and wait for exact-commit CI.
2. Align Cargo, npm, manifests, docs, and release notes to `0.4.1`.
3. The maintainer explicitly pushes the annotated `v0.4.1` tag.
4. The tag workflow builds five platform assets, `SHA256SUMS`, and provenance.
5. After the Release assets are complete, manually dispatch the npm OIDC
   trusted-publisher workflow and publish
   `@agent-governance-suite/mcp@0.4.1` as latest.

Daily CI, the synchronization guard, and the npm workflow never create tags.

## Public boundary

See [WORKSPACE.md](WORKSPACE.md) for module ownership and support-package
migration status. This is a complete public Rust edition, but it excludes
maintainer-local skill bodies, real memory/receipt/archive data, credentials,
machine configuration, and `workspace-services/` state.
