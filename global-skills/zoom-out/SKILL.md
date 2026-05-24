---
name: zoom-out
description: >
  High-level codebase/context explanation skill. Use when the user asks for the bigger picture, architecture context,
  how a module fits into the system, risk assessment before a change, or when local edits need broader orientation.
---

# Zoom Out

Use this to explain the system around a piece of code before deciding what to change.

## Workflow

1. Anchor on the requested file, symbol, behavior, or plan.
2. Inspect its callers, callees, data flow, config, tests, and runtime entrypoints.
3. Read AGENTS.md/CLAUDE.md and domain/ADR docs if present.
4. Explain at the right altitude:
   - What this part is responsible for.
   - What it depends on.
   - Who depends on it.
   - What invariants or contracts matter.
   - What would be risky to change.
5. Finish with practical next steps, not a full rewrite proposal unless the user asked for one.

Keep the answer concise but grounded in file paths and symbols.
