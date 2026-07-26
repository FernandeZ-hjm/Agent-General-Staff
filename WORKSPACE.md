# Agent General Staff 0.3 — Public Edition Workspace

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
  raw-tool-call-stop-guard.js
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

crates/                     # Exactly twelve authoritative Rust packages
  ags-platform/               # Paths, filesystem, hashes, atomic writes
  ags-workspace-facts/        # Canonical workspace and project facts
  ags-host-integration/       # Codex, Claude Code, Cursor, OMP adapters
  ags-capability-governance/  # Capability inventory, resolution, snapshots
  ags-task-contract/          # Task-card compile and handoff contracts
  ags-governance-decision/    # Typed proposals, policy, route decisions
  ags-session/                # Workspace service and client session state
  ags-evidence/               # Receipts and delivery evidence
  ags-verification/           # Doctor, release, sync, version checks
  ags-lifecycle/              # Setup, init, onboarding, update, rollback
  ags-cli/                    # Stable human and Machine CLI adapters
  ags-mcp/                    # Thin MCP/stdio protocol adapter

docs/                       # Documentation
  skill-recommendations.md

templates/                  # Task card templates
tests/                      # Test fixtures
```

## Twelve authoritative modules

The package list above is the complete runtime authority set. The adapters
(`ags-cli`, `ags-mcp`) call the domain modules; they do not retain duplicate
governance, snapshot, session, lease, validation, or verification rules.

## Support-package migration status

v0.3.2 retires the former `bootstrap-dry-run`, `capability-registry`,
`delivery-report-validator`, `execution-policy`, `runner`, `skill-governance`,
`suite-doctor`, `task-card-validator`, and `workflow-sync-check` Cargo package
boundaries. Their implementations live under the owning modules named above;
the old package manifests are absent. Compatibility remains at the human CLI,
Machine CLI, wire/schema, and required re-export surfaces—not through a second
authority.

## Release Identity

Latest release: AGS v0.3.2 Public Edition.

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
