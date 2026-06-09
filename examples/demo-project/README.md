# Demo Project

This is a minimal synthetic Rust project used by AGS task-card examples.

## What This Shows

- How `AGENTS.md` and `CLAUDE.md` can point back to the suite-level protocol
- A minimal Rust project that can be used to test AGS commands
- A safe target for example task cards and receipt verification

## Try It

```bash
# From the AGS repository root, run AGS preflight against the suite itself
ags session preflight --for claude-code --target .

# Run structured verification
ags verify --scope local
```

## Structure

```
demo-project/
  AGENTS.md           # Lightweight synthetic agent entry point
  CLAUDE.md           # Lightweight synthetic execution note
  Cargo.toml          # Minimal Rust project manifest
  src/
    main.rs           # Simple CLI entry point
  tests/
    demo_test.rs      # Basic test
```

This is a synthetic example, not a standalone AGS suite. It contains no real
credentials, private paths, task memories, or production data.
