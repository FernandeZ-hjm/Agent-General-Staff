# MCP Servers

The public Full Installer can document and verify MCP server setup, but it must
not silently install MCP servers. MCP installation changes agent runtime config,
so it requires an explicit user action after dry-run review.

## CodeGraph

CodeGraph is the first supported MCP server for the public Full Installer.

Local command shape:

```bash
codegraph serve --mcp
```

Current install helper:

```bash
codegraph install
```

Useful read-only preview:

```bash
codegraph install --print-config codex
```

Non-interactive install, only after review:

```bash
codegraph install --target auto --location global --yes
```

The MCP server exposes code intelligence operations such as symbol search,
context building, callers, callees, impact analysis, node lookup, file listing,
and index status.

## Public Installer Policy

- `scripts/bootstrap.sh` does not install MCP servers silently.
- MCP install commands must stay explicit and reviewable.
- Full Installer docs may list supported MCP servers and their dry-run or
  print-config commands.
- Future automation should add an MCP-specific dry-run first, then an explicit
  apply step.
