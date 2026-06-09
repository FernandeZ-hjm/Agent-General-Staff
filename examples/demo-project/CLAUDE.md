# CLAUDE.md — Demo Project

This is a lightweight synthetic demo project used by AGS task-card examples.

## Execution Protocol

Before development, debugging, review, commit, or task-card execution, read:

1. `AGENTS.md`
2. `CLAUDE.md`
3. Protocol files under `protocol/` in the parent AGS suite repository

## Quick Start

```bash
cargo build
cargo test
```

## Safety Gates

- Do not install hooks, dependencies, or production wiring without explicit
  task-card authorization.
- Before claiming completion, run verification and report evidence.

This is a demo file. In a real project, CLAUDE.md would contain project-specific
build commands, directory structure, and safety rules.
