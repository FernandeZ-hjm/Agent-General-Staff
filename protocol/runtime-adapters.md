# Runtime adapters — contract v2

AGS v0.4.20 admits any normalized Generic Agent. Official integrations are
optional protocol probes and lifecycle codecs, not an allowlist.

```text
Generic Agent (any normalized HostId)
  + surface: cli | mcp | hybrid
  -> CLI adapter (`ags`) and/or MCP adapter (`ags-mcp`)
  -> authenticated workspace control plane
```

The host keeps conversation context and performs all natural-language
interpretation. AGS accepts one typed Operation. A host adapter may translate a
native envelope, but it must not implement policy, workspace identity, plan
sealing, verification, recovery, or receipts.

## Executables

- `ags`: the human and Machine CLI. `--format json` changes presentation only;
  it does not select another implementation.
- `ags-mcp stdio`: global or project-scoped MCP transport.
- `ags-mcp daemon --workspace <path>`: private per-workspace child mode.
- `ags-host lifecycle ...`: native lifecycle callback adapter.

Transport and lifecycle are not product subcommands. There is no `ags mcp` or
`ags host` command tree.

## Generic Agent admission

`ags agent register --host <id> --surface cli|mcp|hybrid` accepts any HostId
that normalizes to the canonical lowercase dash form. Registration is an
AGS-owned Transaction: decide seals the exact metadata write set, and apply
performs verification, receipt, and recover/risk handling in the authenticated
workspace binding. It does not require a host outcome. `ags agent probe` is
read-only.

Codex, Claude Code, Cursor, OMP, and CodeBuddy may have incremental probe or
lifecycle metadata. Hermes and unfamiliar agents use the same Generic Agent
contract; they are not downgraded to an unofficial security model.

Capability snapshot absence or staleness blocks only exact Skill/MCP selection.
It does not block setup, init, doctor, check, test, schema, or Generic Agent
admission.

## External ReadOnly execution

An external ReadOnly child uses the same fail-closed runner as LocalExecution
but receives zero declared workspace or host-state writable roots. `zero-write`
means no mutation of the canonical workspace, Git state, AGS state, project
configuration, host configuration, credentials, registry, cache, or any other
pre-existing filesystem object. The runner may provide one fresh isolated
scratch directory for process-internal temporary files; it is not an authorized
workspace/host root, is never projected into a result, and is destroyed before
closure. Complete before/after snapshots are audit evidence only and never the
enforcement mechanism.

If the platform cannot enforce filesystem containment and non-evadable process
membership, the adapter returns structured `sandbox_unavailable` and directs
the caller to a policy-approved HostDelegated outcome. It never runs the child
unsandboxed and never converts a selected-path digest comparison into authority.
On macOS, ReadOnly remains single-process: the Seatbelt profile permits the
direct exec but denies `process-fork`, closing the double-fork escape. A
ReadOnly command that requires child processes must use a policy-approved
HostDelegated outcome instead.

## Task-card authority and playbooks

The validated task card is the execution authority. Its `Execution mode`,
`Execution topology`, `Delegation planning`, mutation boundary, and explicit
commit prohibition override playbook workflow defaults. A playbook may narrow
authority but cannot serialize an authorized parallel topology, require lane
commits, or add external writes.

For `fanout-in-card` plus `worktree`, independent lanes may run concurrently in
separate worktrees and return uncommitted diffs and evidence to the main
executor. Only the main executor integrates. A task-card no-commit rule means
no lane commit, integration commit, or temporary commit. Heavy work still
requires an independent reviewer who did not implement the reviewed diff.

## Lifecycle adapter

Official host lifecycle codecs call the standalone executable:

```bash
ags-host lifecycle \
  --event session-start|session-end|stop-guard \
  --host <normalized-host-id> \
  --workspace <canonical-path> \
  --input <json-path|->
```

Generic hosts receive the canonical response. Known host codecs may translate
only response shape. Lifecycle state remains workspace-scoped and does not
create a second governance kernel.

## Conformance

Every adapter is tested for normalized HostId, workspace resolution, daemon
authentication, A→B→A session isolation, cross-binding action rejection,
task-card authority precedence, no-commit delegation, and bounded output.
