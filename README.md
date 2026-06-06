# Agent Governance Suite 2.0 — Public Edition

Agent Governance Suite (AGS) 2.0 public distributable edition. Provides a
Rust-native CLI toolchain for task-card validation, execution policy
resolution, protocol drift checking, suite health diagnostics, bootstrap
simulation, project discovery, agent instructions, session preflight, and
scoped verification.

AGS is a **standing engineering hub** for development work — not a CLI
toolbox you invoke separately. When a development request arrives, governance
engages automatically through the lifecycle defined in `protocol/agent-task-protocol.md`:
ambient preflight → solution formation → user confirmation → user task-card
instruction → execution contract → task routing → gate / execution / receipt.
"方案 OK" only ends the solution phase — a separate user task-card instruction
is required before routing. `ags task compile` enforces this with
`--task-card-requested`.

## Quick Start

```bash
# Install from source
git clone https://github.com/FernandeZ-hjm/agent-governance-suite.git
cd agent-governance-suite
bash scripts/install.sh

# Or DIY: build and add to PATH
cargo build --release
export PATH="$PWD/target/release:$PATH"

# Verify installation
ags doctor
ags verify --scope local
```

## Commands

| Command | Description |
|---|---|
| `ags task validate` | Validate task cards against the canonical format |
| `ags policy resolve` | Resolve execution policy from a task card |
| `ags policy explain` | Explain each policy decision with rule IDs |
| `ags policy check` | Validate + resolve, exit with decision |
| `ags sync check` | Multi-project protocol drift checker |
| `ags doctor` | Suite health diagnostics |
| `ags bootstrap --dry-run` | Bootstrap dry-run simulation |
| `ags bootstrap --apply` | Bootstrap a target directory |
| `ags project detect` | Detect project identity and AGS integration |
| `ags protocol status` | Check protocol file status |
| `ags agent instructions` | Export agent-specific project instructions |
| `ags session preflight` | Aggregated agent wake-up check |
| `ags verify` | Scoped verification checks |

## Directory Structure

```
Cargo.toml              # Rust workspace manifest
AGENTS.md               # Agent entry point
CLAUDE.md               # Agent execution protocol
AGENT_SUITE_PROTOCOL.md  # Suite protocol overview
WORKSPACE.md             # Repository role map

protocol/               # Canonical protocol files
  agent-task-protocol.md
  task-card-template.md
  runtime-adapters.md
  task-routing.md
  skill-governance.md
  project-profile.md
  context-memory.md
  2.0-baseline.md
  2.0-roadmap.md

manifests/              # Suite manifests
  suite.yaml
  skill-recommendations.yaml  # Third-party skill recommendations

scripts/                # Public-safe scripts
  install.sh            # DIY install
  validate.sh           # Task card validation wrapper
  run-task-card.sh      # Task card execution wrapper
  verify.sh             # Verification wrapper
  context-memory.sh     # Public-safe archive/capture wrapper
  stop-archive-hook.sh  # Public-safe Stop archive helper

crates/                 # Rust crates (public-safe core)
  ags-cli/              # Unified CLI entry point
  task-card-validator/  # Task-card validation
  execution-policy/     # Execution policy resolver
  suite-doctor/         # Suite health diagnostics
  bootstrap-dry-run/    # Bootstrap simulation
  workflow-sync-check/  # Protocol drift checker
  ags-verify/           # Scoped verification
  project-discovery/    # Project/agent detection

docs/                   # Documentation
  skill-recommendations.md

templates/              # Task card and memory templates
  memory/               # Blank context capsule / task memory / archive templates
tests/                  # Test fixtures
```

## Public-Full Sanitized Boundary

This public edition ships the full AGS runtime and governance framework, but
not private user state. It includes the Rust workspace, root agent entry files,
protocols, scripts, memory/archive templates, empty governance audit skeletons,
and confirmed skill-governance commands.

It must not include build output (`target/`, release/debug binaries), installed
third-party skills, local skill packs, real task archives, real receipts, private
memory, local agent config, secrets, or machine-specific private paths.

## Memory Capsule Protocol

AGS ships the memory-capsule protocol and blank templates, not real user
memory. `protocol/project-profile.md` and `protocol/context-memory.md` define
how integrated projects can grow their own project profile, context capsule,
task-memory index, and task archives after installation.

The public suite must keep these files generic. User-specific memory belongs in
the user's chosen memory root, such as `$AGS_MEMORY_DIR`, not in the public
distribution.

## Default: DIY Installation

The default install is **DIY** — you get the Rust `ags` core, public protocols,
templates, and basic governance commands. No third-party skills are installed
by default.

After installation, run `ags doctor` for a health check. The report will
include recommendations for third-party development skills that can enhance
the full development experience. These are **recommendations only** — you
must install them manually if you want them.

## Third-Party Skill Recommendations

See `docs/skill-recommendations.md` for a curated list of recommended
third-party development skills with installation instructions, source URLs,
and risk assessments.

Run `cat docs/skill-recommendations.md` after installation to review.

## Release Notes

See `RELEASE_NOTES.md` for the AGS 2.0 Rust and CLI conversion summary.

## Verification

```bash
# Local checks: fmt, test, build, fixtures, YAML, preflight
ags verify --scope local

# Full checks: local + drift
ags verify --scope full

# Release checks: boundary verification
ags verify --scope release
```

## Build from Source

```bash
# Build
cargo build --release

# Test
RUSTFLAGS="-D warnings" cargo test

# Validate task cards
ags task validate path/to/task-card.md
ags task validate - < task-card.md

# Convenience wrapper (delegates to Rust validator)
bash scripts/validate.sh path/to/task-card.md
```

## CLI Reference

| Command (M2 primary) | M1 alias | Description |
|---|---|---|
| `ags task validate <paths>` | `ags task-card-validator <paths>` | Validate task cards |
| `ags policy resolve <path>` | `ags resolve-policy <path>` | Resolve execution policy |
| `ags sync check` | `ags workflow-sync-check` | Protocol drift check |
| `ags doctor` | `ags suite-doctor` | Suite health diagnostics |
| `ags bootstrap --dry-run` | `ags bootstrap-dry-run` | Bootstrap dry-run simulation |
| `ags project detect` | — | Detect project identity and AGS integration |
| `ags protocol status` | — | Check protocol file status and governance requirements |
| `ags agent instructions` | — | Export agent-specific project instructions |
| `ags session preflight` | — | Aggregated agent wake-up check — kernel activation entry point |
| `ags verify` | — | Scoped verification entry point with structured `CheckItem` model |

All commands except `task validate`/`task-card-validator` support `--format text|json`. Illegal format values are rejected by clap (exit 2).

## License

AGS is source-available under the Agent Governance Suite Public License 2.0.
You may use it for personal and internal engineering work, including inside a
commercial company, but you may not repackage, rebrand, host, or resell AGS as a
paid wrapper product without prior written permission.

Superpowers-related workflow inspiration and optional skill references are
credited under `THIRD_PARTY_NOTICES.md`.
