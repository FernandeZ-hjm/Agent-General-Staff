# `@agent-governance-suite/mcp`

Run the local AGS MCP host adapter without a Rust toolchain:

```bash
npx -y @agent-governance-suite/mcp
```

The launcher maps the current OS/architecture to the matching `v0.3.0` GitHub
Release asset, verifies it against `SHA256SUMS`, caches the verified binary, and
executes `ags mcp serve --transport stdio` directly (`shell: false`). A cached
same-version binary continues to work offline. Rust and Cargo are needed only
for AGS source development.

No npm publication happens automatically from this repository. The npm scope
must be created and publication must be explicitly authorized.
