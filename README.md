# Dongmenlaohu Multi-Agent Engineering Kit

Public full installer and DIY/Core governance kit for Codex, Cursor, and
Claude Code collaboration.

This repository has two public profiles:

- **DIY/Core**: the original governance framework, task-card protocol, runtime
  adapter rules, verification gates, project profile templates, and safe
  installer scaffolding.
- **Full Installer**: the Core profile plus a curated skill suite and optional
  capability packs that recreate the high-performance development workflow.

The public kit is profile-driven. It does not ship private project identities,
private sync targets, or machine-specific release paths.

## Quick Start

Preview a full local runtime install:

```bash
bash scripts/bootstrap.sh --dry-run
```

Preview installing the workflow into a target project:

```bash
bash scripts/install-suite-to-project.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

Apply only after reviewing the dry-run:

```bash
bash scripts/install-suite-to-project.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

Verify the suite:

```bash
bash scripts/verify.sh
bash scripts/security-doctor.sh
```

## Profiles

### DIY/Core

Use this when you want the governance framework without adopting the full skill
stack. Core focuses on cache-stable task cards, light/medium/heavy routing,
review and verification gates, project profiles, runtime adapter conventions,
local memory capsules, dry-run, rollback, diff, and doctor tools.

Core does not assume any third-party skill implementation exists. Rules should
refer to capability slots and degrade when a slot is not installed.

### Full Installer

Use this when you want a ready-to-run development environment. Full includes all
Core assets, required global skills, optional curated skill packs, hook
normalization for Claude Code and Codex, a project workflow installer,
verification, and rollback receipts.

Full Installer still avoids private project bindings. Target-project data is
generated from CLI arguments and `config/agent-project-profile.yaml`.

## Repository Layout

```text
├── AGENT_SUITE_PROTOCOL.md
├── README.md
├── docs/
├── global-rules/
├── global-skills/
├── governance/
├── manifests/
├── project-integration/
├── protocol/
├── scripts/
├── skill-packs/
└── templates/
```

## Project Install

`scripts/install-suite-to-project.sh` installs project-level workflow files. It
does not overwrite silently.

It writes or updates:

- `AGENTS.md`
- `CLAUDE.md`
- `docs/agent-workflow/`
- `config/agent-project-profile.yaml`

It creates a timestamped backup before `--apply` and prints a receipt with the
rollback location.

## Runtime Install

`scripts/bootstrap.sh` installs global rules and skills into a target home.
Default mode is dry-run.

```bash
bash scripts/bootstrap.sh --dry-run
bash scripts/bootstrap.sh --apply
bash scripts/diff-local.sh
```

The installer backs up replaced files under:

```text
$HOME/.agents/backups/
```

## Safety

Automation must not run destructive or external dependency commands without
explicit user approval. The manifest keeps these commands forbidden by default:

- `git push --force`
- `curl | bash`
- `npx skills add/remove/update`
- unreviewed dependency installation
- deleting the user's global skill directory

## Public Boundary

This public repository must not include private project names, private release
topology, personal persona profiles, private sync registries, real secrets,
tokens, or API keys.

Run the boundary scan before publishing:

```bash
bash scripts/security-doctor.sh
```
