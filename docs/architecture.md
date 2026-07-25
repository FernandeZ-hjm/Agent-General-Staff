# AGS v0.3.1 Architecture

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
| Host integration | `ags-host-integration` | Codex, Claude Code, Cursor, OMP projections and probes | workspace instance identity |
| Capability governance | `ags-capability-governance` | inventory, exact resolution, lifecycle overlay, atomic snapshot publication | session leases |
| Task contract | `ags-task-contract` | task-card compile, validation, handoff contract | execution |
| Governance decision | `ags-governance-decision` | proposal validation, policy and route contracts | I/O effects |
| Session | `ags-session` | workspace daemon, client session identity, preflight binding source, one-shot action store | MCP JSON-RPC |
| Evidence | `ags-evidence` | receipts, delivery evidence and integrity | decision authority |
| Verification | `ags-verification` | doctor, local/release checks, sync and version-surface gates | lifecycle writes |
| Lifecycle | `ags-lifecycle` | setup, init, onboarding, update and rollback contracts | CLI parsing |
| CLI | `ags-cli` | unchanged Clap surface and application-service dispatch | duplicated governance rules |
| MCP | `ags-mcp` | JSON-RPC conversion, session connection and error mapping | workspace-global state |

Source directories and Cargo package names use the same boundary names.
Smaller crates such as `task-card-validator`, `execution-policy`,
`skill-governance`, and `workflow-sync-check` are subordinate implementation
modules. They are not alternate authorities and do not expose a second product
surface.

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
- Capability bundles are validated and atomically replaced by the daemon.
- Preflight binding and DecisionLease state are per client session.
- A new preflight or route invalidates earlier actions in that session.
- Shape-invalid apply input is rejected before consumption; after the governed
  effect boundary is crossed, failure still consumes the lease.
- Disconnect does not stop the daemon. An empty daemon exits after its idle TTL.
- Executable mismatch triggers authenticated stop-before-restart.

## Compatibility boundary

The complete 36-node visible public v0.3.0 Clap help tree and canonical Machine
CLI paths are captured in
`crates/ags-cli/tests/fixtures/human-cli-v0.3.0.json`. v0.3.1 matches those
surfaces byte-for-byte. Only `ags --version` changes.

Product version, wire/schema version, and historical release version are
separate classes:

- Product metadata is aligned to `0.3.1`.
- v0.3.0 wire/schema identifiers remain stable for compatibility.
- v0.3.0 release notes and fixtures remain historical evidence.
