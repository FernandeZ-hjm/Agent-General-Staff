# Context Memory

AGS context memory is a derived, local view of verified task closure. It helps a
host resume work; it never grants authority and never replaces the task card,
LaunchPlan, delivery report, or receipt.

## Store

```text
$HOME/.agents/memory/projects/<project-slug>/
├── context-capsule.md
├── task-memory.md
└── task-archive/<receipt-id>/
    ├── task-card.md
    ├── launch-plan.json
    ├── delivery-report.md
    └── receipt.json
```

- `context-capsule.md` is a manual project charter. Automation must not rewrite
  `## 项目设计目的` or other human-owned boundaries.
- `task-memory.md` is explicitly non-authoritative. It contains only a compact
  view derived from verified receipts.
- `task-archive/` preserves the raw hash-bound artifacts.
- The local store is never part of a public release payload.

## Rust Ownership

The Rust kernel owns all lifecycle and archive decisions:

```bash
ags memory status
ags memory init
ags memory archive <receipt.json>

ags host lifecycle --event session-start --host <host> --target <repo>
ags host lifecycle --event session-end --host <host> --target <repo>
ags host lifecycle --event stop-guard --host <host> --target <repo>
```

`ags memory archive` first verifies the receipt and the task-card,
LaunchPlan, and delivery-report hashes. A failed or incomplete receipt is never
archived.

## Host Adapter Contract

Codex, Claude Code, Cursor, and OMP use one Rust lifecycle contract. Their
protocol descriptions only map native events to it:

| Native host event | Rust lifecycle event |
|---|---|
| Session start | `session-start` |
| Session end or settled task | `session-end` |
| Output/tool-call guard | `stop-guard` |

Cursor maps these to its lowercase `sessionStart`, `sessionEnd`, and `stop`
command hooks. Its native response fields are `additional_context` and
`followup_message`; the Rust lifecycle kernel selects that envelope from the
platform protocol table.

Host adapters do not parse task cards, infer completion from transcripts,
compare hashes, generate receipts, or implement authority policy.

OMP retains one required JavaScript extension. It may only register OMP events,
pass their JSON envelope to `ags host lifecycle`, and map the returned host
protocol value. OMP itself is not modified.

## Session Start

`session-start` reads bounded `context-capsule.md` and `task-memory.md` content
and returns the host-specific protocol envelope. The read is local and
non-mutating. Missing memory returns an empty result.

Memory can provide context but never upgrades:

- `Execution mode`
- `Execution topology`
- `Delegation planning`
- protected-operation authorization

## Session End

Successful closure is created only by:

```bash
ags task close <task-card> <launch-plan> <delivery-report> \
  --receipt-out <receipt.json>
```

That command atomically writes a session closure pointer after all bindings and
authority checks pass. `session-end` follows only this pointer.

- A valid pointer archives the verified receipt and its three source artifacts.
- Repeating `session-end` is idempotent.
- No pointer produces a `skipped` close receipt.
- Transcript, assistant messages, filenames, and conversation summaries are
  never searched to guess a task card or completed delivery.

## Stop Guard

`stop-guard` examines the host-provided final message envelope for leaked raw
tool-call markup. It does not inspect task authority and does not archive
memory.

## Resume

On continue, context compaction, or task notification:

1. Reread the exact task card and LaunchPlan.
2. Read the named capsule and derived task memory when present.
3. Run `git status --short`.
4. Continue only within the sealed execution mode and topology.
5. Close through `ags task close`; do not construct memory evidence manually.

Context memory owns project continuity. Evolver may propose reusable methods,
but it cannot write project truth or override receipt-bound facts.
