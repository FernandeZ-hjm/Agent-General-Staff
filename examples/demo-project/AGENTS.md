# AGENTS.md — Demo Project

This is a lightweight synthetic demo project used by AGS task-card examples.
Before responding or executing tasks in this repository, also read and follow:

- `CLAUDE.md`

## AGS Example Scope

This project is not a standalone AGS suite. It is a safe Rust target used by
examples in the parent AGS repository. When using it with AGS task cards, run
governance commands from the AGS repository root.

```
ambient preflight → solution formation → user confirmation → task card → execution
```

## Kernel Activation

```bash
ags session preflight --for claude-code --target .
```

## Task Card Validation

Before executing any task:

```bash
bash scripts/validate.sh examples/task-cards/light-demo-task.md
```

This is a demo file. In a real project, AGENTS.md would contain project-specific
rules in addition to AGS managed blocks.
