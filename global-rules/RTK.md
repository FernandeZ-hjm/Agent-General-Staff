# RTK - Rust Token Killer (Codex)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

In Codex, automatically choose RTK for high-token shell commands. Do not blindly prefix every small command.

Codex currently uses this rule for automatic selection. Do not add a fake hook unless Codex exposes a supported pre-tool hook and RTK adds a Codex hook backend.

Use `rtk` for:
- broad search and recursive file discovery
- long file reads or large diffs
- tests, builds, lint, and noisy verification commands
- commands expected to print more than a few dozen lines

Skip `rtk` for short commands such as `pwd`, `date`, `which`, `git status --short`, small `sed -n` reads, and syntax checks with minimal output.

When a command must show exact raw output, run:

```bash
rtk proxy <cmd>
```

## Meta Commands

```bash
rtk gain
rtk gain --history
rtk proxy <cmd>
```

## Verification

```bash
rtk --version
rtk gain
which rtk
```
