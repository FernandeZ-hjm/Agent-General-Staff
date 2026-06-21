# Agent General Staff 2.0 — Public Edition Workspace

This is the **public distributable edition** of the Agent General Staff.
It contains the public-safe Rust `ags` CLI core, AGS MCP host initialization
adapter, canonical protocols, templates, and documentation.

## Default: DIY

The default installation is DIY — only the Rust `ags` core and public protocols.
Third-party development skills are **recommended but not installed automatically**.

After `bash scripts/install.sh`, run `ags doctor` for a health check and review
`docs/skill-recommendations.md` for suggested third-party skills.

## Repository Roles

| Code | Role | Path |
|---|---|---|
| P | Public distributable edition | (auto-detected from WORKSPACE.md) |

The public edition is self-contained. It does not require any private
infrastructure, private repositories, or internal services to build and run.

## Structure

```
Cargo.toml                  # Rust workspace — public crates only
AGENTS.md                   # Agent entry point
CLAUDE.md                   # Agent execution protocol
AGENT_SUITE_PROTOCOL.md     # Suite protocol overview
WORKSPACE.md                # This file

protocol/                   # Canonical protocol files
  2.0-baseline.md
  2.0-roadmap.md
  agent-task-protocol.md
  context-memory.md
  cursor-skill-index.md
  mcp-server.md
  project-profile.md
  runtime-adapters.md
  skill-governance.md
  task-card-template.md
  task-routing.md

manifests/                  # Suite manifests
  mcp-registry.yaml
  skills-registry.yaml
  suite.yaml
  skill-recommendations.yaml

scripts/                    # Public-safe scripts
  claude-stop-memory-capture.py
  context-memory.sh
  install.sh
  lane-decision.sh
  run-task-card.sh
  stop-archive-hook.sh
  update.sh
  validate-task-card.sh
  validate.sh
  verify.sh

crates/                     # Rust crates (public-safe core)
  ags-cli/                  # Unified CLI entry point
  ags-mcp/                  # AGS MCP host initialization adapter
  task-card-validator/      # Task-card validation
  execution-policy/         # Execution policy resolver
  suite-doctor/             # Suite health diagnostics
  bootstrap-dry-run/        # Bootstrap simulation
  workflow-sync-check/      # Protocol drift checker
  ags-verify/               # Scoped verification
  project-discovery/        # Project/agent detection
  receipt/                  # Execution receipt generation and verification
  runner/                   # Policy-resolved executor launch
  task-compiler/            # Execution contract to task-card compiler
  skill-governance/         # Skill recommendation and governance
  capability-registry/      # Local capability detection

docs/                       # Documentation
  skill-recommendations.md

templates/                  # Task card templates
tests/                      # Test fixtures
```

## Release Identity

Release line: AGS 2.0 Public Edition.

Repository name: `agent-governance-suite`.

Repository URL: `https://github.com/FernandeZ-hjm/agent-governance-suite`.

## Standard Checks

```bash
# Core Rust checks
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release

# AGS governance checks
bash scripts/verify.sh
ags verify --scope local
```

## Third-Party Skill Recommendations

See `docs/skill-recommendations.md` and `manifests/skill-recommendations.yaml`
for recommended third-party development skills that enhance the full development
experience. None are installed by default — each must be installed manually by
the user.
