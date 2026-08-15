---
name: ags-agent
description: Govern any normalized Generic Agent over cli, mcp, or hybrid without an admission allowlist.
---

# AGS Agent

AGS 产品版本：0.4.20

```bash
ags agent probe --host <HostId> --surface <cli|mcp|hybrid> --workspace . --format json
ags agent register --host <HostId> --surface <cli|mcp|hybrid> --workspace . --format json
ags apply <ACTION_REF> --outcome <outcome.json> --workspace .
```

Unknown normalized HostIds are valid Generic Agents. Official adapters add only optional probe and hook metadata.
