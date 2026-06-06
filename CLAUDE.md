# Agent Governance Suite — Public Edition

This is the **public distributable edition** of the Agent Governance Suite (`ags`).
It provides a Rust-native CLI toolchain for task-card validation, execution policy
resolution, protocol drift checking, suite health diagnostics, bootstrap simulation,
project discovery, agent instructions, session preflight, and scoped verification.

## Quick Start

```bash
# Install from source
git clone <repo-url>
cd <repo-dir>
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
CLAUDE.md               # This file — Agent execution protocol
AGENT_SUITE_PROTOCOL.md  # Suite protocol overview
WORKSPACE.md             # Repository role map

protocol/               # Canonical protocol files
  agent-task-protocol.md
  task-card-template.md
  runtime-adapters.md
  task-routing.md
  skill-governance.md
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

templates/              # Task card templates
tests/                  # Test fixtures
```

## Default: DIY Installation

The default install is **DIY** — you get the Rust `ags` core, public protocols, templates,
and basic governance commands. No third-party skills are installed by default.

After installation, run `ags doctor` for a health check. The report will include
recommendations for third-party development skills that can enhance the full
development experience. These are **recommendations only** — you must install
them manually if you want them.

## Third-Party Skill Recommendations

See `docs/skill-recommendations.md` for a curated list of recommended
third-party development skills with installation instructions, source URLs,
and risk assessments.

Run `cat docs/skill-recommendations.md` after installation to review.

## Verification

```bash
# Local checks: fmt, test, build, fixtures, YAML, preflight
ags verify --scope local

# Full checks: local + drift (requires AGS_STABLE_ROOT env var)
ags verify --scope full

# Release checks: boundary verification (requires AGS_PUBLIC_ROOT env var)
ags verify --scope release
```

## License

MIT
