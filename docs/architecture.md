# AGS v0.3.6 Architecture

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

## Twelve major boundaries

| Boundary | Package | Owns | Must not own |
|---|---|---|---|
| Platform | `ags-platform` | paths, filesystem, process lookup, hashes, atomic writes | governance policy |
| Workspace facts | `ags-workspace-facts` | canonical project identity, discovery, configuration facts | host mutation |
| Host integration | `ags-host-integration` | canonical host identity, skill roots, MCP probes/registrars, and native memory-adapter facts | workspace instance identity |
| Capability governance | `ags-capability-governance` | inventory, exact resolution, static snapshot publication | session leases |
| Task contract | `ags-task-contract` | task-card compile, validation, handoff contract | execution |
| Governance decision | `ags-governance-decision` | proposal validation, policy and route contracts | I/O effects |
| Session | `ags-session` | workspace daemon, client session identity, preflight binding source, one-shot action store | MCP JSON-RPC |
| Evidence | `ags-evidence` | receipts, delivery evidence and integrity | decision authority |
| Verification | `ags-verification` | doctor, exact release manifest, local/release/promotion checks and version-surface gates | lifecycle writes |
| Lifecycle | `ags-lifecycle` | setup, init, onboarding and explicit update | CLI parsing |
| CLI | `ags-cli` | current Clap surface, application-service dispatch, and one text/JSON output seam | duplicated governance rules |
| MCP | `ags-mcp` | JSON-RPC conversion, session connection and error mapping | workspace-global state |

Source directories and Cargo package names use the same boundary names. The
former `bootstrap-dry-run`, `capability-registry`,
`delivery-report-validator`, `execution-policy`, `runner`, `skill-governance`,
`suite-doctor`, `task-card-validator`, and `workflow-sync-check` package
manifests are retired. Their implementations live inside the owning modules
above, so the workspace has no alternate authority or second product surface.

Host facts are table-driven in `ags-host-integration`: native and shared skill
roots, Codex plugin visibility, MCP probe format/source, live-runtime evidence,
official registrar, and native memory-adapter identity are declared once.
Consumers may add domain behavior, but may not rebuild host lists with string
matches. OMP's inherited Codex registration source is configuration evidence,
not live OMP runtime evidence.

The CLI has one outer output seam for the closed `text | json` choice and JSON
serialization failures. Domain modules still own their human-readable
renderers; adapters do not reimplement format selection per command.

## Workspace service

```text
canonical workspace path
  -> one workspace daemon
       -> Codex client session
       -> Claude Code client session
       -> Cursor client session
       -> OMP client session
```

- The instance key is the SHA-256 of the canonical workspace path only.
- Host identity is a client attribute and never participates in the key.
- Each host has one persisted static capability snapshot. The daemon loads and
  validates it once; request paths never rebuild or compare live capability state.
- Preflight binding and DecisionLease state are per client session.
- A new preflight or route invalidates earlier actions in that session.
- Shape-invalid apply input is rejected before consumption; after the governed
  effect boundary is crossed, failure still consumes the lease.
- Disconnect does not stop the daemon. An empty daemon exits after its idle TTL.
- Executable mismatch triggers authenticated stop-before-restart.

## Public completion boundary

Public completion requires more than an exact release-manifest comparison. Promotion
must validate the canonical payload authority, exact tracked inventory and
content hashes, public-safe source topology, compatibility contracts, release
workflows, and absence of private runtime or third-party skill bodies.

## Current surface boundary

The current 0.3.6 CLI, MCP schema and typed contracts are authoritative.
Removed aliases, dynamic capability lifecycle commands and historical
behavior fixtures are not hidden compatibility interfaces. Product version,
wire/schema version and historical release notes remain separate version
classes.
