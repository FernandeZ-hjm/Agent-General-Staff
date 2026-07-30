# Runtime Adapters

AGS has one Rust Host Adapter and a table of host protocol descriptions. Codex,
Claude Code, Cursor, and OMP are not separate policy implementations.

```text
host-native event or MCP probe
        │
        ▼
AgentPlatformSpec (data)
        │
        ▼
HostAdapter (one Rust implementation)
        │
        ├─ MCP status/probe normalization
        ├─ lifecycle event normalization
        └─ LaunchPlan handoff
```

Host-specific code may translate transport envelopes. It must not parse task
cards, calculate authority, compare artifact hashes, generate receipts, or
implement closure policy.

## Canonical Task Authority

Every task card declares three independent fields:

```text
Execution mode: plan-only | single-writer | fanout-in-card | fanout-cross-card
Execution topology: single | parallel | worktree
Delegation planning: no | yes
```

`Execution mode` is the writer-authority lattice:

```text
plan-only < single-writer < fanout-in-card < fanout-cross-card
```

- `plan-only`: no filesystem or external mutation.
- `single-writer`: one writer owns all mutation.
- `fanout-in-card`: multiple writers are allowed only within the current card.
- `fanout-cross-card`: the current card may coordinate separately closed cards.

`Execution topology` is the maximum execution shape:

- `single` permits only single.
- `parallel` permits single or parallel.
- `worktree` permits single, parallel, or worktree.

`Delegation planning: yes` permits designing a delegation plan. It does not
create writers and does not upgrade `Execution mode`.

The following combinations fail closed:

- `plan-only` with topology other than `single`.
- `plan-only` or `single-writer` with actual delegation.
- fanout requested by task text without a `fanout-*` execution mode.
- subtask orchestration without `parallel` or `worktree`.

`Execution effort` remains a reasoning-intensity field:

```text
low | normal | high | exhaustive
```

It never changes authority, topology, delegation, review, or launch arguments.
Task level (`Light`, `Medium`, `Heavy`) is a risk and review tier only.

## LaunchPlan

`ags run` validates the card and resolves policy, then emits a
`0.3.6-launch-plan`. It prepares execution but does not start a host.

Required binding fields:

- `task_card_hash`
- `launch_plan_hash`
- `effective_execution_mode`
- `effective_execution_topology`
- `delegation_planning`
- resolved launch arguments
- downgrade reasons

`launch_plan_hash` is SHA-256 over deterministic LaunchPlan body JSON excluding
the hash itself and any timestamp or random value. The same validated input and
policy produce the same plan and hash.

Launch arguments come only from the resolved policy. A non-writing mode must
not emit `--parallel`, `--worktree`, `--headless`, or another write-capable
flag.

## Downscope at Closure

The delivery report declares actual use:

```text
Closure schema: 1.1
task-card-hash:
launch-plan-hash:
execution-mode-used:
execution-topology-used:
delegation-used: none | in-card | cross-card
```

Actual authority may only shrink:

- execution mode used must be no greater than the effective mode;
- topology used must be no greater than the effective topology;
- `plan-only` and `single-writer` require `delegation-used: none`;
- `fanout-in-card` permits `none` or `in-card`;
- `fanout-cross-card` permits `none`, `in-card`, or `cross-card`.

Closure is performed only through:

```bash
ags task close <task-card> <launch-plan> <delivery-report> \
  --receipt-out <receipt.json> \
  --format text|json
```

This single Rust operation verifies all hashes and authority, generates the
`0.3.6-task-receipt`, and writes the session closure pointer.

## Host Protocol Table

The Rust `AgentPlatformSpec` table describes:

- canonical host ID and display name;
- CLI names and config locations;
- MCP probe command and output format;
- native skill roots;
- memory/lifecycle protocol;
- supported verification and registration operations.

The table currently includes:

| Host | Host ID | Runtime adapter | Lifecycle bridge |
|---|---|---|---|
| Codex | `codex` | `codex-local` | native hooks call Rust |
| Claude Code | `claude-code` | `claude-code` | native hooks call Rust |
| Cursor | `cursor` | `cursor` | native lowercase hooks call Rust |
| OMP | `omp` | `omp` | thin JS extension calls Rust |
| CodeBuddy-Code | `codebuddy-code` | `codebuddy-code` | native workspace hooks call Rust |

Adding a host means adding a protocol description and contract tests. It does
not mean copying policy or lifecycle implementations.

## Lifecycle Contract

All five hosts call:

```bash
ags host lifecycle --event session-start|session-end|stop-guard \
  --host codex|claude-code|cursor|codebuddy-code|omp \
  --target <repo>
```

The v0.4.0 generator writes only workspace-owned adapters and replaces
`<repo>` with the canonical absolute workspace path. Claude Code uses
`.claude/settings.local.json`, CodeBuddy-Code uses
`.codebuddy/settings.local.json`, Codex and Cursor use their project hook
files, and OMP uses `.omp/extensions/ags-memory-lifecycle.js`.

`ags host lifecycle` is a compatibility-preserving CLI facade. It sends a
`0.4.0-workspace-lifecycle` envelope to the canonical workspace daemon; it no
longer owns memory reads, archive writes, or host-session state.

- `session-start` returns bounded, read-only memory context.
- `session-end` archives only a verified closure pointer.
- `stop-guard` checks raw tool-call markup leakage.

Claude Code consumes `hookSpecificOutput.additionalContext`; its clear Stop
response omits an empty optional `hookSpecificOutput`. CodeBuddy-Code uses the
same SessionStart context shape but maps a blocked Stop to its native
`continue: false` plus `reason` response. Cursor consumes `additional_context`
on `sessionStart` and `followup_message` on `stop`. These response-envelope
differences are declared in `AgentPlatformSpec`, not implemented as additional
lifecycle kernels. Cursor registration evidence is read through
`cursor-agent mcp list` with its file credential-store mode so the probe does
not depend on the macOS login keychain.

OMP's JavaScript extension is intentionally thin: register events, pass JSON to
the command, map the result. OMP itself is not patched.

## MCP Process Ownership

The MCP server never restarts itself. Process management belongs at the
CLI/lifecycle seam:

```bash
ags mcp status
ags mcp restart
```

Restart stops the workspace service and lets the host reconnect to the current
binary. All old connection-bound preflight bindings, actions, and leases become
invalid.

## MCP Self-Integrity

Before serving a governed request, MCP hashes the complete current executable
content and compares it with the startup identity. It deliberately has no
inode/mtime/ctime shortcut because those metadata are not reliable change
signals on every filesystem.

## Review and Resume

Review gates are independent from execution authority:

- Light: focused self-review.
- Medium: integration and boundary review.
- Heavy: independent review.

On resume, reread the exact task card and LaunchPlan, inspect current workspace
state, and continue only within the sealed authority. Conversation text,
memory, task level, skill selection, and host product name cannot upgrade it.

## Adapter Conformance

Every supported host runs the same contract suite:

- task-card and policy input mapping;
- lifecycle start/end/stop behavior;
- no transcript inference;
- idempotent repeated SessionEnd;
- no closure pointer means safe skip;
- MCP probe result normalization;
- plan-only launch-argument gate.

Host-native integration tests may add transport checks, but they cannot weaken
the shared Rust semantics.
