# Agent task protocol — contract v2

The host interprets human language once. AGS consumes typed Operations and a
confirmed task card when the task uses handoff. It never reinterprets prose to
choose task level, execution authority, capabilities, or commands.

## Task authority

A task card is executable only after schema validation and an explicit handoff.
Its `Execution mode`, `Execution topology`, and `Delegation planning` tuple is
the authority ceiling. Light, Medium, and Heavy are risk/review tiers; Heavy
adds independent review and cannot alter the tuple.

The card binds goals (`G-*`), acceptance criteria (`AC-*`), verification
commands (`V-*`), evidence (`EV-*`), stop conditions, write ownership, and
commit/push/release constraints. Workflows and Skills must preserve those
fields verbatim.

## Execution lifecycle

```text
validate card
  -> open typed Operation
  -> immutable WorkspaceBinding
  -> policy decision
  -> sealed plan
  -> host-native execution within authority
  -> automatic verification and receipt
  -> independent review when required
  -> closure
```

Planning is not execution, and a plan reference is not completion. Effectful
Operations require explicit apply. Replay, tamper, and cross-binding attempts
fail closed.

## Fanout

`fanout-in-card` permits multiple writers only inside the same card.
`fanout-cross-card` is required for separately authoritative cards. Parallel
writers require `parallel` or `worktree` topology and non-overlapping owned
paths. The main executor alone integrates lane diffs, runs combined checks and
tests, owns final delivery, and closes the card.

Under no-commit, lanes leave changes uncommitted and report tracked diff hashes
plus exact untracked inventories. Integration and review cover the complete
working-tree bytes; commit ranges alone are insufficient.

## Check and test

`ags check` evaluates governance, evidence, change, release, or promotion
boundaries and always reports `project_tests_run=false`. It never executes a
project test command.

`ags test smoke|standard|full` consumes one structured `CommandSpec` containing
`program`, `argv`, `cwd`, `env`, `timeout_ms`, and
`allowed_write_paths`. It does not use shell interpolation. The closed
`TestReceipt` binds canonical workspace, commit, tree, argv hash, exit code,
duration, output digest, and observed write set. Test failure never rolls back
source; unexpected writes produce `risk-escalated`.

## Review and closure

- Light: risk-matched review.
- Medium: complete diff review; independent review for high-impact modules.
- Heavy: an independent non-author reviews the complete integrated diff,
  interfaces, workspace isolation, failure semantics, and verification evidence.

Blocking findings are fixed and re-reviewed. If the gate cannot be satisfied,
delivery is partial or blocked. Closure records card and plan identity, actual
downscoped authority, G/AC/V/EV results, review evidence, diff summary, preserved
state, and deliberately unperformed external actions. Only the contract v2
kernel may issue the terminal receipt.

A hash-bound canonical task card is immutable evidence, not a compatibility
surface. If such a card was authored before the contract-v2 hard cut and its
literal close command conflicts with that same card's hard-cut acceptance
criteria, the executor must not edit the card or restore the removed command.
The contract-v2 registry is authoritative: prepare closure with
`ags govern task close`, then consume its sealed `action_ref` with `ags apply`.
The delivery report records this protocol resolution and preserves the original
task-card hash.
