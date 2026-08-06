# AGS v0.4.14 Architecture

AGS is a multi-Agent development governance control plane. It admits typed
requests, binds authority and policy, validates evidence, and preserves
capability/session state. It is not a task queue, Agent scheduler, parallel
executor, or multi-Agent negotiation runtime.

## Authority flow

```text
human request
  -> host semantic interpretation
  -> typed HostRouteProposal
  -> governance decision
  -> host-native action or one-shot server-held action
  -> verified evidence / delivery closure
```

Natural language remains in the host. AGS accepts closed typed fields and never
reconstructs user intent from keywords.

## Twelve authoritative modules

| Module | Owns | Must not own |
|---|---|---|
| `ags-platform` | paths, filesystem, process lookup, hashes, atomic writes | governance policy |
| `ags-workspace-facts` | canonical workspace identity, discovery, protocol audit, preflight facts | host mutation |
| `ags-host-integration` | host identity and native integration facts | workspace instance identity |
| `ags-capability-governance` | catalog, skill-body governance, exact resolution, static snapshots | session leases |
| `ags-task-contract` | task-card compile/validate, handoff contract, non-executing preparation | Agent execution |
| `ags-governance-decision` | typed proposals, policy resolution, route and decision contracts | I/O effects |
| `ags-session` | workspace daemon, client sessions, preflight bindings, one-shot action store | MCP JSON-RPC |
| `ags-evidence` | receipts, delivery closure, evidence integrity | decision authority |
| `ags-verification` | doctor, typed public projection, local/promotion/release gates and verification bundles | lifecycle writes |
| `ags-lifecycle` | setup, init, onboarding, update | CLI parsing |
| `ags-cli` | current human/Machine CLI and application dispatch | duplicated domain rules |
| `ags-mcp` | JSON-RPC conversion, session connection, error mapping | workspace-global state |

`cargo metadata` must expose exactly these packages. The former
The former `bootstrap-dry-run`, `capability-registry`, `delivery-report-validator`,
`execution-policy`, `runner`, `skill-governance`, `suite-doctor`,
`task-card-validator`, and `workflow-sync-check` package manifests are absent.
Their implementations live under the owning modules:

```text
ags-capability-governance/skill_body
ags-evidence/delivery_report
ags-governance-decision/policy
ags-task-contract/runner
ags-task-contract/validator
ags-verification/doctor
ags-verification/release_manifest
```

Only current commands, wire/schema identifiers, and necessary Rust re-exports
remain. Legacy aliases and packages are not compatibility mechanisms because
they preserve dead behavior or a second authority.

## Dependency direction

```text
human CLI / MCP wire
       |
       v
ags-cli / ags-mcp                 adapters
       |
       v
lifecycle / verification          orchestration
       |
       v
capability / task / decision /
session / evidence / workspace    domain modules
       |
       v
platform / host integration       machine facts
```

Dependencies point inward. Adapters do not own governance state, and machine
fact modules do not import product adapters.

Host facts are table-driven in `ags-host-integration`: native and shared skill
roots, Codex plugin visibility, MCP probe format/source, live-runtime evidence,
official registrar, and native memory-adapter identity are declared once.
Consumers may add domain behavior, but may not rebuild host lists with string
matches. In particular, OMP's inherited Codex registration source is not live
OMP runtime evidence.

The CLI has one outer output seam for the closed `text | json` choice and JSON
serialization failures. Domain modules still own their human-readable
renderers; the adapter no longer reimplements format selection per command.

## Workspace service

```text
canonical workspace path
  -> one workspace daemon
       -> Codex client session
       -> Claude Code client session
       -> Cursor client session
       -> CodeBuddy client session
       -> OMP client session
```

- The instance key is derived from the canonical workspace path only.
- Host identity is a client attribute and never participates in the key.
- Each host has one persisted static capability snapshot. The daemon loads and
  validates it once for its lifetime. Explicit snapshot refresh followed by a
  daemon restart publishes changed canonical sources; request-time scanning or
  automatic cache invalidation is deliberately absent.
- Lifecycle and reusable preflight state are keyed by workspace, host, and host
  session. DecisionLease state remains connection-bound and is never reused.
- Workspace-owned host adapters translate SessionStart, per-turn Stop Guard,
  and true SessionEnd into one daemon lifecycle envelope.
- A new preflight or route invalidates earlier actions in that session.
- Shape-invalid apply input is rejected before consumption. Failure after the
  effect boundary still consumes the lease.
- Disconnect does not stop the daemon. An empty daemon exits after its idle TTL.
- Executable mismatch triggers authenticated stop-before-restart.
- Hosts keep using `ags mcp serve --transport stdio`; no daemon command is added
  to the public command surface.

## Evidence

The host MCP E2E suite launches real `ags` adapter and daemon processes. It
tests same-workspace sharing, cross-project separation, reconnects, foreign
lease rejection, snapshot rebind, idle recycle, and executable replacement for
Codex, Claude Code, Cursor, CodeBuddy, and OMP identities. It does not automate
host GUIs.

The release comparison runs the current private build against the installed
stable build with fixed samples for startup, preflight, and E2E behavior. It
reports median and p95; the stable checkout remains read-only.

## Versions

The supported CLI and MCP contracts are the current release surfaces. Historical
hidden commands, loose compiler inputs, and captured executable snapshots are
not compatibility authorities.

Version classes remain separate:

- current source candidate: `0.4.14`; latest published release: `0.4.14`;
- existing governance wire schemas remain on the 0.3.6 contract;
- maintenance, runtime install/migration, suite activation and update notice
  schemas introduced or changed in this release use 0.4.13 identifiers;
- unchanged runtime-stage and release-plan structures retain their 0.4.0 identifiers;
- MCP `protocolVersion` remains `2024-11-05`;
- historical releases remain only in release notes, not runtime fixtures.

## Public completion

Public completion requires more than an exact release-manifest comparison. The guard
also proves:

- the exact twelve-package module set and source topology;
- bilingual current documentation and GPL-3.0-only/latest wording;
- real workspace-service E2E and performance-contract evidence;
- absence of every retired package and authority marker;
- public-safe runtime exclusions and exact-version release surfaces.
