# Agent Governance Suite Workspace

This checkout is the self-contained public distributable edition of Agent
Governance Suite (AGS). AGS is a multi-Agent development governance control
plane. It governs admission, authorization, policy, verification, receipts,
capability state, and memory closure; it is not an Agent scheduler or execution
platform.

Current source candidate: **v0.4.0**. Latest published release: **v0.3.8**.

The normal request path is:

```text
preflight
  -> current-host capability catalog
  -> host semantic interpretation
  -> typed HostRouteProposal
  -> read-only ags_route_request
  -> host-native action or explicit ags_apply_action
```

Natural language stays in the host. AGS validates closed typed fields. Each
canonical workspace has one long-lived daemon; stdio integrations are thin
clients, while session identity, preflight binding, and DecisionLease remain
client-local.

## Repository role

This repository is the reviewed public-safe projection. It contains the public
Rust workspace, protocols, templates, release workflows, and empty governance
skeletons. It does not require private infrastructure to build or run. Product
version, wire/schema version, and historical release version remain separate
version classes.

| Code | Role | Path |
|---|---|---|
| P | Public distributable edition | (repository root) |

## Twelve authoritative modules

The current runtime workspace exposes exactly twelve Cargo packages. Package
names are the architectural module names; there is no second routing,
snapshot, lease, validation, or verification authority behind a legacy
package.

| Module | Owns | Must not own |
|---|---|---|
| `ags-platform` | cross-platform paths, filesystem operations, process lookup, hashing, atomic files | governance policy |
| `ags-workspace-facts` | canonical workspace facts, discovery, protocol audit, preflight projections | host mutation |
| `ags-host-integration` | canonical host identity, skill roots, MCP probes/registrars, and native memory-adapter facts | workspace instance identity |
| `ags-capability-governance` | capability catalog, skill-body governance, exact resolution, overlay lifecycle, snapshot publication | session leases |
| `ags-task-contract` | task-card compile/validate, handoff contract, non-executing launch preparation | Agent execution |
| `ags-governance-decision` | typed proposals, policy resolution, route and decision contracts | filesystem or process effects |
| `ags-session` | canonical-workspace daemon, client sessions, preflight bindings, one-shot action store | MCP JSON-RPC conversion |
| `ags-evidence` | receipts, delivery closure, evidence integrity | decision authority |
| `ags-verification` | bootstrap readiness, doctor, exact release manifest, local/promotion/release checks, version and module gates | lifecycle writes |
| `ags-lifecycle` | setup, init, onboarding, and update contracts | CLI parsing |
| `ags-cli` | current human and Machine CLI adapter with one text/JSON output seam | duplicated domain rules |
| `ags-mcp` | thin MCP wire adapter, workspace connection, error mapping | workspace-global governance state |

The dependency direction is inward: `ags-cli` and `ags-mcp` adapt external
protocols to the domain modules; lifecycle and verification orchestrate
closed operations; domain modules depend on platform facts, never on the
adapters.

## Support-package migration status

The current architecture retires the nine former support packages as Cargo package boundaries.
Their implementation moves under the owning authoritative module. Compatibility
is preserved at the human CLI, Machine CLI, wire/schema, and selected Rust
re-export surfaces—not by keeping a second package alive.

| Former package | Owning module | Status required for release |
|---|---|---|
| `bootstrap-dry-run` | `ags-verification::bootstrap` | implementation relocated; old package manifest removed |
| `capability-registry` | `ags-capability-governance::project_registry` | implementation relocated; old package manifest removed |
| `delivery-report-validator` | `ags-evidence::delivery_report` | implementation relocated; old package manifest removed |
| `execution-policy` | `ags-governance-decision::policy` | implementation relocated; old package manifest removed |
| `runner` | `ags-task-contract::runner` | non-executing preparation retained; old package manifest removed |
| `skill-governance` | `ags-capability-governance::skill_body` | implementation relocated; old package manifest removed |
| `suite-doctor` | `ags-verification::doctor` | implementation relocated; old package manifest removed |
| `task-card-validator` | `ags-task-contract::validator` | canonical validator retained; old package manifest removed |
| `workflow-sync-check` | `ags-verification::release_manifest` | exact release manifest retained; section-drift engine removed |

Release verification keeps the completed migration observable:

1. reconnect imports and compatibility re-exports to the owning module;
2. remove every retired package manifest and workspace dependency;
3. prove `cargo metadata` exposes exactly the twelve authoritative packages;
4. promote the same public-safe source/module topology to B;
5. reject retired package and command surfaces instead of keeping hidden compatibility paths.

`crates/ags-cli/tests/module_boundary_contract.rs` is the executable source of
truth for the package set and retired-package absence.

## Public promotion boundary

The sanitized public edition may include:

- the twelve-package public Rust workspace and lockfile;
- public protocol files, templates, install/doctor/validation scripts;
- public documentation, GPL-3.0-only license, release notes, manifests, and
  empty governance audit skeletons;
- public-safe capability metadata and release workflows.

It must not include:

- build outputs or preinstalled binaries;
- private/personal skill bodies or local Agent configuration;
- real task archives, receipts, memory capsules, credentials, or machine paths;
- private runtime state, including overlays, usage ledgers, auth state,
  capability snapshots, leases, or `workspace-services/` registry/token/bundle
  files.

Promotion follows A → S → B. A green exact release-manifest comparison is necessary
but not sufficient: source/module topology, documentation, E2E contracts,
performance evidence, version surfaces, and retired-authority absence must also
pass the public guard.

## Current interfaces

The current human and Machine CLI surfaces are authoritative. Retired aliases
and captured legacy behavior fixtures are not hidden compatibility contracts.
The daemon architecture requires no separate user command:

```text
ags mcp serve --transport stdio
  -> connect-or-start
  -> one daemon for the canonical workspace
```

Current wire/schema identifiers are release-gated together with their owners.

## Verification

Use the narrowest relevant check while developing. Before promotion:

```bash
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
ags verify --scope release
git diff --check
```

The public release additionally requires `ags verify --scope release`, the
repository release wrapper, and remote CI for the exact public commit.
