# AGS MCP: Host Initialization Adapter

AGS governance kernel exposed as a **host initialization adapter** over MCP
(Model Context Protocol). It is the mandatory governance interface for all MCP
hosts (Tencent Agent [WorkBuddy, CodeBuddy-Code], Codex, Cursor, Claude Code)
operating in AGS scenarios.

**AGS MCP is not a governed third-party MCP.** It is the suite's own host
adapter — the MCP transport layer for the AGS governance kernel. In
`manifests/mcp-registry.yaml`, `ags` resides under `suite_interfaces:`, not
under the governed `mcps:` list.

## Architecture

```
┌──────────────────────────────────────────────┐
│              MCP Host                        │
│ (Tencent Agent / Codex / Cursor / Claude Code)│
│                                              │
│  Step 0: ags_preflight ← MANDATORY FIRST │
│           (initialization gate)              │
│           ↓                                  │
│  Step 1-N: all other AGS tools               │
└───┬──────────────────────────────────────────┘
    │
    ▼
┌──────────┐
│ AGS MCP  │
│ (host    │
│ adapter, │
│ mandatory│
│ first)   │
│ stdio    │
└──────────┘
```

AGS MCP exposes AGS lifecycle and gate checks only. It does not proxy, wrap, or
broker unrelated tools.

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
- The user explicitly requests AGS protocol, task cards, review gates, or
  verification gates

### Legal invocation paths (in priority order)

1. **MCP path (preferred)**: call `ags_preflight` tool with `agent` parameter
   (`codex` / `claude-code` / `cursor` / `tencent-agent` / `workbuddy` /
   `codebuddy-code`) and optional `target`. WorkBuddy and CodeBuddy-Code are
   Tencent Agent host clients; unknown ids use the generic governed-host profile.
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
| `ags_agent_instructions` | Agent-specific instructions — for Codex/Claude Code/Cursor/Tencent Agent (WorkBuddy, CodeBuddy-Code) |
| `ags_task_validate` | Validate a task card against the canonical format gate |
| `ags_policy_resolve` | Resolve execution policy for a validated task card |
| `ags_verify_local` | Run local-scope verification (fmt, test, build, fixtures, YAML, preflight). Fixed-scope — not downgradable by caller input |
| `ags_solution_check` | Check whether solution formation phase allows an executable task card. Also runs the deterministic prompt-request classifier over `summary` and returns `detected_task_card_request` + `detected_triggers` (advisory — detection does NOT authorize a card), plus advisory-intent fields `detected_advisory_intent` / `mutation_allowed` / `advisory_block_reason`, and an advisory `value_route` block (效价比路由) |

> **Initialization gate rule**: `ags_preflight` is the mandatory first call.
> All other tools (including `ags_solution_check`, `ags_task_validate`, etc.) must only
> be called after preflight completes. Hosts should reject calls to other AGS
> tools before preflight is done.

### Entry intent recognition (deterministic)

`ags_solution_check` runs the deterministic `prompt-request-classifier` over the
`summary` argument and returns two advisory fields:

- `detected_task_card_request` (bool) — the summary matches a prompt/task-card
  request ("给我提示词", "生成任务卡", "交给 Claude Code", "给 CC 执行",
  "写个 prompt", "handoff", "让 Claude 做", …).
- `detected_triggers` (string array) — the matched trigger phrases.

This is **advisory only**: detection does NOT change `executable_allowed`. The
three-gate threshold (方案 OK → 任务卡指令 → 任务分级路由) still requires an
explicit user task-card instruction. The classifier exists so the host
recognizes prompt/task-card intent instead of treating it as prose — closing the
pre-entry bypass.

The frontstage **output-shape gate** and the `governance_miss` event live on the
CLI side (`ags gate output`), NOT in MCP — AGS MCP stays read-only and never
emits or persists `governance_miss`. See `protocol/agent-task-protocol.md` §3.6.

### Advisory intent no-mutation (deterministic)

`ags_solution_check` also classifies advisory/consultation intent and returns
three optional fields (present only when advisory intent is detected):

- `detected_advisory_intent` (bool) — the summary matches a consultation trigger
  ("你看看是否", "是否需要", "要不要", "评估一下", "你觉得", "should we",
  "evaluate", …).
- `mutation_allowed` (bool) — `false` blocks write-type tool calls; cleared to
  `true` only by an explicit execution authorization ("按这个改", "开始实现",
  "implement this", …). Bare "执行" is intentionally not an authorization.
- `advisory_block_reason` (string) — `advisory_intent_no_mutation` when blocked.

Advisory intent no-mutation, the task-card request gate, and the execution
permission gate are **three separate concerns** — none substitutes for another.
See `protocol/agent-task-protocol.md` §3.7.

### Value Route (效价比路由)

`ags_solution_check` also returns a `value_route` block — the minimal
execution-path *form* that still covers the task's risk, derived deterministically
from the same classification signals. Fields: `recommended_path` (one of
`read-only-advisory` / `direct-edit` / `plan-first` / `claude-code-route` /
`stop-for-scope`), `rationale`, `rejected_lighter` / `rejected_heavier`
(`{path, reason}`), `requires_user_confirmation`, `needs_planner_judgment`,
`advisory` (always `true`), and `authority_note`.

This is **advisory only**: Value Route shapes the path form and never changes the
Light / Medium / Heavy level, permission mode, Review gate, or Verification gate.
The same `value_route` block is exposed on the CLI side by `ags gate
prompt-request`. See `protocol/agent-task-protocol.md` §3.9.

### Quiet-by-default visible status

`ags_preflight`, `ags_solution_check`, and `ags_policy_resolve` carry an optional
`visible_status` field — the single foreground decision state (`OK`,
`NEEDS_USER_DECISION`, `BLOCKED_BY_POLICY`, `RISK_ESCALATED`,
`DONE_WITH_RECEIPT`, `ADVISORY_NO_MUTATION`). Full report detail stays in the
response as audit evidence; `visible_status` is the quiet summary. Quiet affects
only the foreground — trace, receipt, and archive writes are unchanged. All new
fields are optional (`skip_serializing_if`), so existing clients are unaffected.
See `protocol/agent-task-protocol.md` §3.8.

### Resources (5)

| URI | Content |
|-----|---------|
| `ags://global-kernel` | AGS global governance kernel — initialization gate, lifecycle, rules, host boundaries |
| `ags://protocol/agent-task-protocol` | Canonical agent task protocol |
| `ags://protocol/task-card-template` | Fixed task-card skeleton |
| `ags://protocol/runtime-adapters` | Runtime adapter definitions and rules |
| `ags://protocol/task-routing` | Light/Medium/Heavy routing criteria |

### Prompts (4)

| Prompt | Description |
|--------|-------------|
| `ags_global_kernel` | Load AGS governance kernel at session start — including mandatory initialization gate |
| `ags_solution_phase` | Guide through solution formation and context-backed proposal |
| `ags_task_card_request_gate` | Enforce the task-card instruction gate |
| `ags_delivery_report` | Produce a valid AGS delivery report |

## AGS vs Governed MCPs

AGS MCP is structurally distinct from third-party MCPs governed by AGS:

| Layer | Contains | Role |
|-------|----------|------|
| `suite_interfaces` (in `manifests/mcp-registry.yaml`) | `ags` | **Host initialization adapter** — mandatory governance interface; NOT a governed object |
| `mcps` (in `manifests/mcp-registry.yaml`) | registered external MCPs | **Governed third-party MCPs** — reviewed, registered, and managed by AGS |

AGS is not in the `mcps:` list. It does not have an adoption entry in the MCP
adoption log. It is the governance authority, not a governed entity.

## JSON-RPC Protocol

### Initialize

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"...","version":"..."}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false},"resources":{"listChanged":false,"subscribe":false},"prompts":{"listChanged":false}},"serverInfo":{"name":"ags-mcp","version":"2.7.0"}}}
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

## Tencent Agent Registration (WorkBuddy / CodeBuddy-Code)

Tencent Agent is the host family; WorkBuddy and CodeBuddy-Code are its host
clients. They are host platforms that enter AGS through MCP; this is distinct
from task-card `Runtime adapter` authority. `ags setup` emits three equivalent
platform MCP registration snippets:
`hosts/tencent-agent.mcp.snippet.json` (primary), `hosts/workbuddy.mcp.snippet.json`
(compatibility), and `hosts/codebuddy-code.mcp.snippet.json`.

# In a Tencent Agent host (WorkBuddy / CodeBuddy-Code) MCP configuration
# (do NOT write from this task):
```json
{
  "mcpServers": {
    "ags": {
      "role": "host_initialization_adapter",
      "command": "ags",
      "args": ["mcp", "serve", "--transport", "stdio"],
      "mandatory_first_tool": "ags_preflight",
      "_comment": "AGS MCP is a host initialization adapter."
    }
  }
}
```

> **Note**: This task does NOT write Tencent Agent (WorkBuddy / CodeBuddy-Code)
> configuration. Manual registration by the user is required.

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
- Write Tencent Agent (WorkBuddy / CodeBuddy-Code) host configuration
- Read real tokens, node-local secrets, or host secrets
- Modify user worktrees without explicit authorization
- Change AGS lifecycle/gate semantics
- Turn AGS MCP into a broker for unrelated tools

## Version History

| Version | Date | Changes |
|---------|------|---------|
| 2.6.0 | 2026-06-17 | Quiet governance release: advisory-intent no-mutation gate for consultation requests, `visible_status` quiet foreground summaries on MCP preflight/solution/policy responses, fixed-scope `ags_verify_local`, diff-aware `ags verify lane`, trusted shell MINIMAL/FULL push-lane routing, Tencent Agent host recognition, Value Route advisory output, expanded verification smoke tests, and copyable Markdown fenced-block delivery reports. |
| 2.5.1 | 2026-06-15 | Local ignored governance overlay: `ags init` defaults to a `local` overlay that adds AGS-managed files to `.git/info/exclude` (idempotent managed block), `--mode shared|tracked` opts into a committed overlay, and `--migrate-tracked-overlay` safely migrates already-tracked AGS-owned files via `git rm --cached`. Task-card template sources collapsed to the single canonical `protocol/task-card-template.md` (per-level fallback templates removed) |
| 2.5.0 | 2026-06-13 | Engineering self-consistency release: supply-chain gate (`deny.toml` + `cargo deny check`), Windows portability phase 1 (cross-platform home/temp/PATH-lookup helpers), skill asset inventory scanner, and `task-card-validator` module split (move-only, public API and validation messages unchanged) |
| 2.4.0 | 2026-06-12 | Added human command facade (`setup`, `init`, `doctor`, `skill`, `help`), one-command host initialization, visible Codex command skills (`AGS Setup`, `AGS Init`, `AGS Skill`, `AGS Doctor`) with Chinese descriptions, retired visible Codex hub/preflight/verify entries, project onboarding `.gitignore` management, soft-coded host agent compatibility, and schema-safe MCP tool names |
| 2.1.0 | 2026-06-10 | Added AGS Initialization Gate; repositioned as host initialization adapter; structural separation from governed MCPs |
| 1.0.0 | 2026-06-10 | Initial release — 7 tools, 7 resources, 4 prompts, stdio transport only |
