# AGS v0.4.20 Architecture

AGS is a typed governance control plane. The host interprets natural language;
AGS accepts an `OperationRequest`, returns a decision or sealed plan, and
consumes the plan through one explicit apply boundary. AGS is not an Agent
scheduler, task queue, third-party MCP broker, or natural-language router.

## One operation authority

```text
human CLI / machine JSON / MCP
              |
              v
       Operation registry
              |
              v
     open -> decide -> apply
              |
              v
     verify -> receipt/recover
```

Each public operation has one typed request declared by the registry. The same
declaration supplies adapter routing, schema, help metadata, operation kind, and
handler selection. Adapters do not compose domain argv or run `ags` recursively.

Operation kinds are `ReadOnly`, `Transaction`, `LocalExecution`, and
`HostDelegated`. ReadOnly performs no write. Effectful operations first return a
sealed plan; `ags apply` and MCP `ags_apply` are the only product apply surfaces.
Transactions verify and either receipt or recover. LocalExecution preserves
source changes on test failure and escalates an unexpected write set; a host
without provable descendant containment is blocked before spawn. HostDelegated
work may close only after a content-addressed, binding-scoped outcome receipt is
verified. The current A-workspace candidate keeps production HostDelegated
apply blocked until that artifact verifier is implemented.

## Authoritative modules

| Module | Owns |
|---|---|
| `ags-platform` | paths, hashes, atomic filesystem/process primitives |
| `ags-workspace-facts` | canonical project facts and protocol audit |
| `ags-host-integration` | normalized Generic Agent identity and optional host probes |
| `ags-capability-governance` | capability inventory, exact Skill resolution and snapshots |
| `ags-task-contract` | typed task-card validation and launch contract |
| `ags-governance-decision` | execution policy vocabulary |
| `ags-session` | workspace resolver/router, authenticated daemon sessions |
| `ags-evidence` | receipts and closure evidence integrity |
| `ags-verification` | governance check and structured project-test execution |
| `ags-control-plane` | registry, sealed plans, state machine, domain handlers |
| `ags-cli` | ten-command human and machine adapter |
| `ags-mcp` | standalone stdio adapter and private daemon child mode |

Dependencies point inward. `ags-cli` and `ags-mcp` translate Interface values;
they do not own policy or lifecycle orchestration. `ags-host` is the separate
host lifecycle callback executable.

## Target-routed workspace service

One global stdio transport may serve many per-workspace daemons:

```text
Generic Agent connection
  -> request context (explicit workspace | unique MCP root | adapter cwd)
  -> canonical workspace + project facts
  -> authenticated workspace-service handshake
  -> immutable WorkspaceBinding
  -> per-workspace open/decide/apply
```

Resolution order is fixed. HOME, daemon cwd, recent-project state, fuzzy path
matching, and managed-project guesses are not fallbacks. Managed-project data
may assist discovery but is not identity authority. The router keeps isolated
authenticated sessions per canonical workspace and has no mutable global
current binding, so A -> B -> A traffic can reuse both sessions safely.

An action reference binds connection, normalized host, canonical workspace,
authenticated session, policy, plan, and payload. It cannot be replayed or used
across a connection, host, workspace, daemon restart, or plan mutation.

## Public Interface

The CLI exposes only `setup`, `init`, `agent`, `govern`, `update`, `doctor`,
`check`, `test`, `apply`, and `schema`. MCP exposes only `ags_decide` and
`ags_apply`. Contract v2 is a hard cut: older commands, aliases, tools, wires,
schema identifiers, and compatibility handlers are absent.

Default successful human output is at most five lines. Default JSON is at most
16 KiB and MCP tool schema is at most 8 KiB; larger evidence is returned through
a `details_uri`.

## Workspace boundaries

Workspace A is the private development authority. Stable, public, remotes,
installed runtimes, tags, packages, and releases change only through separately
authorized promotion or release work. A local verification pass never implies
promotion or release completion.
