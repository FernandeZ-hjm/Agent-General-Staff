# AGS contract v2 Operation routing

AGS exposes one typed Operation registry. Every public Operation is declared
once with its request type, `deny_unknown_fields` schema, kind, policy metadata,
handler, help text, and verification contract. Human CLI, Machine CLI, MCP,
schema, help, and documentation projections consume that registry; adapters do
not maintain parallel routing tables.

## Control-plane flow

```text
open typed Operation
  -> resolve immutable WorkspaceBinding
  -> decide policy
  -> blocked | no-change | sealed plan
  -> explicit apply when effectful
  -> automatic verify
  -> receipt | recover | risk-escalated
```

Operation kinds are `ReadOnly`, `Transaction`, `LocalExecution`, and
`HostDelegated`. Canonical states are `blocked`, `no-change`, `planned`,
`applying`, `verifying`, `receipted`, `recovering`, and `risk-escalated`.

- `ReadOnly` performs zero protected writes and returns closed evidence
  directly. For an external child, zero-write means zero declared workspace or
  host-state writable roots: the canonical workspace, Git/AGS/project state,
  host configuration, credentials, registry, caches, and every other
  pre-existing filesystem object remain non-writable. A fresh isolated scratch
  directory may exist only for process-internal temporary bytes and is destroyed
  before closure. Before/after snapshots are audit evidence, not enforcement.
- `Transaction` reaches apply, verify, receipt, or recover with an explicit
  terminal state.
- `LocalExecution` executes one structured `CommandSpec`. A non-zero project
  test is a closed failure receipt and never triggers source rollback;
  unexpected writes escalate risk.
- `HostDelegated` closes only after a binding-valid typed outcome is submitted.

## Execution authority

A confirmed task card's explicit `Execution mode`, `Execution topology`, and
`Delegation planning` fields are authoritative. Task level only selects review
risk. Skills, adapters, defaults, and model effort cannot rewrite the tuple.
No-commit, no-push, write ownership, and protected-path constraints remain
binding through fanout, integration, review, and closure.

## Capability state

A missing or stale capability snapshot blocks only exact Skill or third-party
MCP selection. It does not block the core setup, init, doctor, check, or test
Operations. AGS core never interprets raw natural language and never brokers a
third-party MCP connection.

## Hard cut

Contract v2 has no aliases, redirects, deprecated parser branches, removed-
command handler, compatibility wire, or translation layer. Unknown commands,
options, fields, Operations, and schema identifiers receive the standard
structured unknown/invalid response.
