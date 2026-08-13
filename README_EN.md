# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)

[中文](README.md) | [English](README_EN.md)

AGS is a governance control plane for multi-agent development. It owns
admission, exact capability routing, authorization, maintenance transactions,
verification, receipts, and recovery. It is not an agent scheduler, task queue,
or natural-language classifier.

The current source candidate is **v0.4.16**; the latest published release is
**v0.4.15**.

## Choose CLI, MCP, or both

The CLI and MCP packages are independent npm entrypoints backed by the same
signed Rust kernel, content-addressed cache, and machine state directory.
Installing both does not download a second kernel.

```bash
npx -y @agent-governance-suite/cli --help
npx -y @agent-governance-suite/mcp
```

Source installation remains available:

```bash
cargo install --path crates/ags-cli --locked --force
ags setup --yes
```

## One maintenance transaction

AGS core, Skill, setup, and Host activation changes share one closure:

```text
Intent -> hash-bound Plan -> user approval -> Apply
       -> Host activation -> Verify -> Receipt
                         \-> Recover on failure
```

Plans freeze source, version, content hashes, risks, writes, and rollback points.
Apply accepts only the exact unexpired `plan_hash`. Copying files is not success:
Host projections, snapshots, and a real RouteResolution must verify. Updates are
never applied silently.

## Third-party Skills

The recommendation catalog is discovery and review metadata, not an install
allowlist. Users may select a catalog ID or any GitHub repository/tree URL,
branch, tag, or commit. Apply always resolves remote content to an immutable
commit first.

```bash
ags skill recommend
ags skill inspect <catalog-id|github-url|local-path>
ags skill install <catalog-id|github-url>
ags skill adopt <local-path>
ags skill check [skill-id]
ags skill update <skill-id>
ags skill rollback <skill-id>
ags skill status [skill-id]
ags skill verify <skill-id>
```

The machine-local `InstalledSkillRecord` is the only installation truth.
Catalog, installation, Host activation, and update policy are independent
layers. Old JSON state is migrated once, never read as a compatibility model.
The default policy is `notify`; `manual` and `pinned` are also available. AGS
never executes scripts shipped by a third-party repository.

Traversal, symlink escape, special files, out-of-bound writes, ID overwrite,
and hash drift are blocked. Unknown licenses, scripts, binaries, external
dependencies, and elevated privileges are disclosed and require explicit risk
acknowledgement.

## setup and five Host projections

`ags setup` and maintenance updates project required suite Skills to Codex,
Claude Code, OMP, Cursor, and CodeBuddy-Code. Declared upstream renames are
migrated and AGS-owned retired symlinks are removed. The complete candidate
state is prepared before an atomic switch; failure restores bodies, indexes,
snapshots, and Host pointers.

Private maintainer tooling may additionally require every target to come from a
stable authority root. The public kernel exposes the policy seam without
hard-coding maintainer paths.

## Signed update notices

CLI startup, MCP preflight, or Doctor lazily checks a signed release index once
per seven calendar days by default. Offline or unreachable upstream state never
blocks installed capabilities. Users can snooze, ignore one version, or disable
checks.

```bash
ags update check
ags update plan
ags update status --plan-hash <HASH>
ags update apply --plan-hash <HASH>
ags update verify --plan-hash <HASH>
ags update recover
```

## Typed governance path

Natural language stays in the Host. AGS consumes closed typed contracts only:

```text
preflight
  -> ags://capabilities/current-host
  -> typed HostRouteProposal
  -> read-only ags_route_request
  -> Host-native action or one-shot ags_apply_action
  -> evidence / receipt closure
```

`ags_route_request` accepts no raw request and has no keyword or similarity
fallback. `ags_apply_action` consumes only a connection-bound one-shot action.
Task-card creation requires both an explicit handoff request and a confirmed
handoff contract. Heavy adds independent review; it never expands authority.

## Architecture and release proof

The public workspace contains exactly twelve authoritative Cargo modules. CLI
and MCP are adapters; capability, decision, session, lifecycle, evidence, and
verification modules own domain rules. Retired update/bootstrap/registry
implementations are not preserved as a second authority. See
[WORKSPACE.md](WORKSPACE.md) and [docs/architecture.md](docs/architecture.md).

The release chain permits two full gates: one exact A candidate and one exact B
public commit. Promotion, tag, Release, and npm consume a content-addressed
`VerificationBundle` instead of replaying the workspace suite.

## Public boundary

The public edition includes the complete public Rust kernel, dual npm
entrypoints, protocols, typed manifests, command Skills, and release workflows.
It excludes private Skill bodies, real memory/receipt/archive data, credentials,
machine paths, Host configuration, and runtime state. Typed projection generates
the public capability manifests; catalog membership alone never means installed
or routable.

License: **GPL-3.0-only**.
