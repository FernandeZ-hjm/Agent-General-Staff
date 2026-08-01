# `@agent-governance-suite/mcp`

Run the local Agent Governance Suite MCP host adapter without a Rust toolchain:

```bash
npx -y @agent-governance-suite/mcp
```

The launcher maps the current OS/architecture to the matching `v0.4.11` GitHub
Release asset, verifies it against `SHA256SUMS`, caches the verified binary, and
executes `ags mcp serve --transport stdio` directly (`shell: false`). A cached
same-version binary continues to work offline. Rust and Cargo are needed only
for AGS source development.

The stdio process is a thin proxy to the unique daemon keyed by the canonical
workspace path. Codex, Claude Code, Cursor, CodeBuddy-Code, and OMP clients
share workspace capability state while retaining independent sessions,
preflight bindings, and DecisionLeases.

Publication is an explicit post-Release workflow protected by the npm
environment and npm trusted publishing (GitHub OIDC); no long-lived npm token is
stored in the repository.

This launcher is licensed under `GPL-3.0-only`; see `LICENSE`.
