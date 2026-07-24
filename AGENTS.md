# AGENTS.md

## Repository contract

This checkout is the public distributable edition of Agent General Staff 0.3.
It is self-contained and must not depend on private repositories, machine-local
state, credentials, or unpublished skill bodies.

Before AGS-governed work, call MCP `ags_preflight` or run:

```bash
ags session preflight --for <agent> --target .
```

The host reads `ags://capabilities/current-host`, interprets the complete user
context, and submits one typed `HostRouteProposal` to read-only
`ags_route_request`. `ags_apply_action` may consume only the returned
connection-bound action. Existing `## 任务卡` input is validated before request
classification.

## Hard boundaries

- Do not publish local memory, receipts, credentials, build output, host config,
  or machine-specific paths.
- Do not modify protocol, release, protected, destructive, external-write, or
  credential boundaries without matching authorization.
- AGS MCP is the suite host adapter, not a governed third-party MCP.
- `agents govern --apply` may install only AGS-owned host memory adapters;
  external MCP registration remains advice-only.
- Task-card permission is only `plan-only` or `execute-and-verify`; Heavy adds an
  independent review gate.
- Preserve unrelated working-tree changes and user-owned entry-file content.

## Read when relevant

- Repository and publication boundary: `WORKSPACE.md`
- Entrypoint size and ownership: `protocol/entrypoint-guidelines.md`
- Governance overview: `AGENT_SUITE_PROTOCOL.md`
- Task lifecycle and cards: `protocol/agent-task-protocol.md`,
  `protocol/task-card-template.md`, `protocol/task-routing.md`
- Host adapters and memory closure: `protocol/runtime-adapters.md`,
  `protocol/context-memory.md`
- MCP contract: `protocol/mcp-server.md`
- Skills and capabilities: `protocol/skill-governance.md`

## Verification

Use the narrowest relevant check during development. Before delivery:

```bash
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
bash scripts/verify.sh
git diff --check
```

After context compaction, re-check the current request, repository root, and
`git status --short` before editing.
