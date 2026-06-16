# AGS MCP: Host Initialization Adapter

AGS governance kernel exposed as a **host initialization adapter** over MCP
(Model Context Protocol). It is the mandatory governance interface for all MCP
hosts (WorkBuddy, Codex, Cursor, Claude Code) operating in AGS scenarios.

**AGS MCP is not a governed third-party MCP.** It is the suite's own host
adapter — the MCP transport layer for the AGS governance kernel. In
`manifests/mcp-registry.yaml`, `ags` resides under `suite_interfaces:`, not
alongside governed third-party MCPs like `context7` or `gep`.

## Architecture

```
┌──────────────────────────────────────────────┐
│              MCP Host                        │
│  (WorkBuddy / Codex / Cursor / Claude Code)  │
│                                              │
│  Step 0: ags_preflight ← MANDATORY FIRST │
│           (initialization gate)              │
│           ↓                                  │
│  Step 1-N: all other AGS tools               │
│           ↓                                  │
│  Parallel: EvoMap MCP (advisory only)        │
└───┬──────────────────┬───────────────────────┘
    │                  │
    ▼                  ▼
┌──────────┐   ┌──────────────┐
│ AGS MCP  │   │  EvoMap MCP  │
│ (host    │   │  (advisory   │
│ adapter, │   │   memory)    │
│ mandatory│   │              │
│ first)   │   │  Parallel    │
│          │   │  peer, not   │
│ stdio    │   │  brokered    │
└──────────┘   └──────────────┘
```

**AGS MCP and EvoMap MCP are parallel peers.** Each is called independently
by the host. AGS MCP does NOT proxy, wrap, or broker EvoMap MCP calls.

## AGS Initialization Gate

The AGS Initialization Gate is the mandatory first step for any host operating
in an AGS scenario. The host MUST complete `ags_preflight` (MCP) or its CLI
fallback before invoking any other AGS tool or entering solution formation.

### Trigger conditions (AGS scenarios)

An AGS scenario is active when any of the following is true:

- The repository is under AGS governance (`ags project detect` returns `suite`
  or `integrated`, or `AGENTS.md` + `CLAUDE.md` root entry files exist)
- The task involves task cards (generation, validation, execution, archival)
- AGS skill governance is involved (skill adopt/ignore/sync/proposal)
- AGS MCP governance is involved (MCP registry writes, adoption logs, trust
  level changes)
- Multi-agent routing is involved (task handoff between Codex / Claude Code /
  Cursor)
- AGS release boundary is involved (private → stable → public promotion)
- Execution policy is involved (permission downgrades, parallelism strategy,
  launch arg resolution)
- AGS Evolver boundary is involved (method capture, recall boundary, Gene/
  Capsule governance)
- The user explicitly requests AGS protocol, task cards, review gates, or
  verification gates

### Legal invocation paths (in priority order)

1. **MCP path (preferred)**: call `ags_preflight` tool with `agent` parameter
   (`codex` / `claude-code` / `cursor` / `workbuddy`) and optional `target`.
2. **CLI fallback**: when MCP is unavailable, run
   `ags session preflight --for <agent> [--target <path>]`.

Both paths are valid. CLI fallback requires the same evidence recording as MCP.

### Prohibition rules

- Do NOT read protocol documents and manually simulate preflight output.
- Do NOT skip preflight based on model memory, user oral description, or
  host built-in rules.
- Do NOT enter solution formation, task routing, execution, or delivery
  without recording preflight evidence.
- `ags_solution_check`, `ags_task_validate`, `ags_policy_resolve`, and
  `ags_verify_local` are NOT substitutes for preflight. The host MUST
  complete preflight first; it should reject calls to other AGS tools
  before preflight is done.

### Failure handling

- **MCP unavailable**: report `AGS MCP unavailable`, then execute CLI fallback
  (`ags session preflight --for <agent>`).
- **CLI fallback also unavailable**: stop. Do not continue AGS scenario tasks.
  Report the stop reason to the user.
- **CLI fallback succeeds but MCP unavailable**: subsequent AGS tool calls use
  CLI equivalents (`ags task validate` etc.); record `status=fallback` in
  evidence.

### Evidence format

In solutions, task cards, or delivery reports, record preflight evidence as:

```
AGS preflight: MCP|CLI, agent=<agent-id>, status=<ok|failed|fallback>
```

| Value | Meaning |
|-------|---------|
| `ok` | MCP preflight or CLI fallback succeeded with `exit_code=0` |
| `fallback` | MCP unavailable, CLI fallback succeeded |
| `failed` | Both MCP and CLI fallback failed, execution stopped |

## Installation

```bash
# Build from source
cargo build -p ags-cli --release

# Run as MCP server
./target/release/ags mcp serve --transport stdio
```

## Transport

V1 supports `--transport stdio` only. The server reads line-delimited
JSON-RPC 2.0 messages from stdin and writes responses to stdout.
Stderr is reserved for server logging.

Future transports (SSE, WebSocket) may be added in later versions.

## MCP Capabilities

### Tools (7)

| Tool | Description |
|------|-------------|
| `ags_preflight` | **Mandatory first call.** Aggregated session preflight — project identity, protocol status, agent instructions, memory paths, stop conditions, warnings, failures, next steps. Must be called before any other AGS tool in AGS scenarios. |
| `ags_protocol_status` | Protocol file inventory — which files are present/missing, validator entry, risk boundaries |
| `ags_agent_instructions` | Agent-specific instructions — for Codex/Claude Code/Cursor/WorkBuddy |
| `ags_task_validate` | Validate a task card against the canonical format gate |
| `ags_policy_resolve` | Resolve execution policy for a validated task card |
| `ags_verify_local` | Run local-scope verification (fmt, test, build, fixtures, YAML, preflight) |
| `ags_solution_check` | Check whether solution formation phase allows an executable task card |

> **Initialization gate rule**: `ags_preflight` is the mandatory first call.
> All other tools (including `ags_solution_check`, `ags_task_validate`, etc.) must only
> be called after preflight completes. Hosts should reject calls to other AGS
> tools before preflight is done.

### Resources (7)

| URI | Content |
|-----|---------|
| `ags://global-kernel` | AGS global governance kernel — initialization gate, lifecycle, rules, EvoMap boundary |
| `ags://protocol/agent-task-protocol` | Canonical agent task protocol |
| `ags://protocol/task-card-template` | Fixed task-card skeleton |
| `ags://protocol/runtime-adapters` | Runtime adapter definitions and rules |
| `ags://protocol/task-routing` | Light/Medium/Heavy routing criteria |

### Prompts (4)

| Prompt | Description |
|--------|-------------|
| `ags_global_kernel` | Load AGS governance kernel at session start — including mandatory initialization gate |
| `ags_solution_phase` | Guide through solution formation with EvoMap recall |
| `ags_task_card_request_gate` | Enforce the task-card instruction gate |
| `ags_delivery_report` | Produce a valid AGS delivery report |

## EvoMap Parallel-Call Boundary

AGS MCP and EvoMap MCP are **parallel peers**:

- **AGS MCP** decides: lifecycle, task level, permission mode, review gate,
  verification gate, release boundary, stop conditions.
- **EvoMap MCP** advises: design patterns, reusable methods, risk flags,
  edge cases — during solution formation only.

AGS MCP does NOT:
- Proxy, wrap, or broker EvoMap MCP calls
- Return EvoMap Gene/Capsule/EvolutionEvent data
- Install or configure EvoMap MCP
- Require EvoMap MCP to be present

The `ags_solution_check` tool recommends EvoMap recall for non-trivial tasks
but records `recall_status: unavailable_or_not_called` — hosts must call
EvoMap MCP in parallel.

### Authority precedence

When EvoMap output conflicts with AGS protocol, project memory, task cards,
or gates — AGS always wins. EvoMap output is advisory input to solution
formation; it must not override governance decisions.

## AGS vs Governed MCPs

AGS MCP is structurally distinct from third-party MCPs governed by AGS:

| Layer | Contains | Role |
|-------|----------|------|
| `suite_interfaces` (in `manifests/mcp-registry.yaml`) | `ags` | **Host initialization adapter** — mandatory governance interface; NOT a governed object |
| `mcps` (in `manifests/mcp-registry.yaml`) | `context7`, `gep`, ... | **Governed third-party MCPs** — reviewed, registered, and managed by AGS |

AGS is not in the `mcps:` list. It does not have an adoption entry in the MCP
adoption log. It is the governance authority, not a governed entity.

## JSON-RPC Protocol

### Initialize

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"...","version":"..."}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false},"resources":{"listChanged":false,"subscribe":false},"prompts":{"listChanged":false}},"serverInfo":{"name":"ags-mcp","version":"2.5.1"}}}
```

### Tools

```json
→ {"jsonrpc":"2.0","id":2,"method":"tools/list"}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[...]}}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ags_preflight","arguments":{"agent":"codex"}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"..."}]}}
```

### Resources

```json
→ {"jsonrpc":"2.0","id":4,"method":"resources/list"}
← {"jsonrpc":"2.0","id":4,"result":{"resources":[...]}}

→ {"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"ags://global-kernel"}}
← {"jsonrpc":"2.0","id":5,"result":{"contents":[{"uri":"ags://global-kernel","mimeType":"text/markdown","text":"..."}]}}
```

### Prompts

```json
→ {"jsonrpc":"2.0","id":6,"method":"prompts/list"}
← {"jsonrpc":"2.0","id":6,"result":{"prompts":[...]}}

→ {"jsonrpc":"2.0","id":7,"method":"prompts/get","params":{"name":"ags_solution_phase","arguments":{"user_request":"..."}}}
← {"jsonrpc":"2.0","id":7,"result":{"description":"...","messages":[{"role":"user","content":{"type":"text","text":"..."}}]}}
```

## WorkBuddy Registration

To register AGS MCP in WorkBuddy as the mandatory host initialization adapter:

```yaml
# In WorkBuddy MCP configuration (do NOT write from this task):
mcps:
  - name: "ags"
    transport: "stdio"
    command: "ags"
    args: ["mcp", "serve", "--transport", "stdio"]
    role: "host_initialization_adapter"
    mandatory_first: true
    boundary: "does_not_proxy_evomap"
```

> **Note**: This task does NOT write WorkBuddy configuration.
> Manual registration by the user is required.

## Verification

```bash
# Build
cargo build -p ags-cli

# Run MCP server
cargo run -p ags-cli -- mcp serve --transport stdio

# Smoke test
printf '{"jsonrpc":"2.0","id":1,"method":"initialize",...}\n{"jsonrpc":"2.0","id":2,"method":"tools/list"}\n' | cargo run -p ags-cli -- mcp serve --transport stdio

# Full verification
ags verify --scope local
```

## Stop Conditions

The AGS MCP implementation must stop and report when asked to:
- Write WorkBuddy global configuration
- Install or enable EvoMap MCP
- Read real tokens, node_secret, or `~/.evolver/settings.json`
- Modify stable/public worktree
- Change AGS lifecycle/gate semantics
- Proxy EvoMap MCP into AGS MCP or build an MCP broker

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 2.5.0 | 2026-06-13 | Public edition no longer serves the two EvoMap boundary resources (`ags://evolver-boundary`, `ags://protocol/evolution-memory`) whose backing files the public release gate forbids; `serverInfo.version` bumped to 2.5.0. AGS tool / resource / prompt product surface otherwise unchanged |
| 2.4.0 | 2026-06-12 | Added human command facade (`setup`, `init`, `doctor`, `skill`, `help`), one-command host initialization, visible Codex command skills (`AGS Setup`, `AGS Init`, `AGS Skill`, `AGS Doctor`) with Chinese descriptions, retired visible Codex hub/preflight/verify entries, project onboarding `.gitignore` management, soft-coded host agent compatibility, and schema-safe MCP tool names |
| 2.1.0 | 2026-06-10 | Added AGS Initialization Gate; repositioned as host initialization adapter; structural separation from governed MCPs |
| 1.0.0 | 2026-06-10 | Initial release — 7 tools, 7 resources, 4 prompts, stdio transport only |
