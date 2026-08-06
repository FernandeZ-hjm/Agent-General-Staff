# `@agent-governance-suite/mcp`

Run the local Agent Governance Suite MCP host adapter without a Rust toolchain:

```bash
npx -y @agent-governance-suite/mcp
```

First-time setup uses the same signed Rust transaction as the CLI package:

```bash
npx -y @agent-governance-suite/mcp setup --lifecycle-hosts detected --yes
```

Run without arguments to serve MCP over stdio. Skill lifecycle operations then
use the typed `ags_maintenance_*` tools; there is no separate JavaScript
installer or updater.

The launcher maps the current OS/architecture to the matching versioned GitHub
Release asset, verifies the pinned Ed25519-signed release index and exact asset
hash before a fresh install is activated, and executes
`ags mcp serve --transport stdio` directly (`shell: false`). The MCP and CLI
packages share the immutable `~/.ags/versions/<version>/<triple>/` cache and
atomic `~/.ags/launcher-state/current.json` pointer (or the corresponding
`AGS_CACHE_DIR` root), so installing both does not redownload the artifact.
A cached same-version binary continues to work offline. Rust and Cargo are
needed only for AGS source development.

Update checks are lazy and at most once every seven days. They use
`launcher-state/update-check.json`; an unsigned release index is recorded as
`unavailable` and is never treated as a verified update.

The CLI and MCP packages depend on the same interface-neutral launcher package;
installing one product entrance does not install the other entrance. Release
signatures are verified with the public key shipped by that shared core.

The stdio process is a thin proxy to the unique daemon keyed by the canonical
workspace path. Codex, Claude Code, Cursor, CodeBuddy-Code, and OMP clients
share workspace capability state while retaining independent sessions,
preflight bindings, and DecisionLeases.

Publication is an explicit post-Release workflow protected by the npm
environment and npm trusted publishing (GitHub OIDC); no long-lived npm token is
stored in the repository.

This launcher is licensed under `GPL-3.0-only`; see `LICENSE`.
