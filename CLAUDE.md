# Agent Governance Suite 2.0 — Public Edition

This is the **2.0 public distributable edition** of the Agent Governance Suite (`ags`).
It provides a Rust-native CLI toolchain, an AGS MCP stdio server, Claude Code
`/ags` entry commands, and Codex-visible AGS command skills for task-card
validation, execution policy resolution, protocol drift checking, suite health
diagnostics, bootstrap simulation, project discovery, agent instructions, session
preflight, and scoped verification.

## Execution Protocol

This file is an agent execution entry point, not only a command reference.
Agents working in this repository must follow the canonical protocol files under
`protocol/` before changing code, generating task cards, installing hooks, or
declaring completion.

AGS is a standing engineering hub for development work. When a development
request arrives, governance engages automatically:

```text
ambient preflight
  -> solution formation
  -> user confirmation ("方案 OK")
  -> explicit task-card instruction ("生成任务卡")
  -> execution contract
  -> task routing
  -> gate / execution / receipt
```

Do not classify raw user requests as Light / Medium / Heavy. Classification
happens only after preflight, solution formation, user confirmation, and a
separate task-card instruction. `方案 OK` is not authorization to generate or
execute a task card.

## Required Reads

Before development, debugging, review, commit, task-card generation, or handoff,
read:

1. `AGENTS.md`
2. `CLAUDE.md`
3. `AGENT_SUITE_PROTOCOL.md`
4. `protocol/agent-task-protocol.md`
5. `protocol/task-routing.md`
6. `protocol/runtime-adapters.md`
7. `protocol/task-card-template.md`
8. `protocol/skill-governance.md` when skills, hooks, or local agent capability
   changes are involved

Then run or equivalently complete:

```bash
ags session preflight --for codex --target .
ags session preflight --for claude-code --target .
ags session preflight --for cursor --target .
```

Use the command matching the current agent runtime. The report is read-only and
aggregates project identity, protocol status, agent instructions, memory paths,
stop conditions, warnings, failures, and next steps.

When AGS MCP is available, AGS-related tasks must call the MCP `ags_preflight`
tool first and treat CLI preflight as a fallback path only. The public MCP server
entry point is:

```bash
ags mcp serve --transport stdio
```

## Role Boundaries

Codex and Cursor own preflight, diagnosis, solution formation, user confirmation,
execution-contract formation, task routing, task-card generation, and final
review.

Claude Code executes bounded task cards that already exist. Claude Code must not
derive task level, permission mode, or task-card authorization from raw user
requests or from `方案 OK` alone.

## Safety Gates

- Do not install hooks, dependencies, runner adapters, or production wiring
  without explicit task-card authorization.
- Do not modify protocol files, task-card skeletons, public release boundaries,
  or execution-policy rules unless the current task explicitly targets them.
- Heavy tasks start plan-only and wait for explicit human approval before file
  mutation.
- Resume / `继续` is not mutation approval. Reread the task card, run
  `git status --short`, and stop if approval is unclear.
- Do not run destructive git commands, touch secrets, overwrite user files, or
  replace user-owned entry files unless explicitly authorized.
- Before claiming completion, run the narrowest relevant verification and report
  the evidence.

## Project Entry Integration

User projects usually already have their own `AGENTS.md` and `CLAUDE.md`.
Do not replace them with suite copies. Use the managed-block integration command:

```bash
ags project integrate --target /path/to/repo --dry-run
ags project integrate --target /path/to/repo --confirm
```

This preserves user-authored content, updates only the marked AGS block, creates
backups on confirmed writes, and stops on conflicting entry-file rules.

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
/ags setup
ags setup --yes --force
ags doctor
ags verify --scope local
```

## Commands

| Command | Description |
|---|---|
| `ags setup` | Write public-safe local AGS runtime snippets, MCP config snippets, Claude `/ags`, and Codex AGS command skills |
| `ags init` | Integrate AGS managed blocks into a target project |
| `ags mcp serve` | Start the public AGS MCP stdio server |
| `ags task validate` | Validate task cards against the canonical format |
| `ags policy resolve` | Resolve execution policy from a task card |
| `ags policy explain` | Explain each policy decision with rule IDs |
| `ags policy check` | Validate + resolve, exit with decision |
| `ags sync check` | Multi-project protocol drift checker |
| `ags doctor` | Suite health diagnostics |
| `ags bootstrap --dry-run` | Bootstrap dry-run simulation |
| `ags bootstrap --apply` | Bootstrap a target directory |
| `ags project detect` | Detect project identity and AGS integration |
| `ags project integrate` | Incrementally merge AGS managed blocks into project entry files |
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

## Memory Capsule Protocol

`protocol/project-profile.md` and `protocol/context-memory.md` are public-safe
protocol skeletons. Real project profiles, task archives, receipts, and memory
capsules are user-grown state and must not be bundled into the public suite.

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

MIT License. See `LICENSE`, `NOTICE.md`, and `THIRD_PARTY_NOTICES.md`.
