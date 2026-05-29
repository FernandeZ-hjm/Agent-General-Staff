# Dongmenlaohu Multi-Agent Engineering Kit

Public full installer and DIY/Core governance kit for Codex, Cursor, and
Claude Code collaboration.

This repository has two public profiles:

- **DIY/Core**: the original governance framework, task-card protocol, runtime
  adapter rules, verification gates, project profile templates, and safe
  installer scaffolding.
- **Full Installer**: the Core profile plus the required curated skill suite
  that recreates the high-performance development workflow.

The public kit is profile-driven. It does not ship private project identities,
private sync targets, or machine-specific release paths.

## Quick Start

Preview a full one-click install into a target project and local runtime:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

DIY/Core installs only the governance framework and project workflow by
default:

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

Apply only after reviewing the dry-run:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

Check local conflicts, installed runtime drift, target project state, and
available public updates:

```bash
bash scripts/kit-doctor.sh doctor --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
```

Review third-party skill sources:

```text
docs/third-party-skills.md
```

Review supported MCP servers:

```text
docs/mcp-servers.md
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
Core assets, required global skills, hook normalization for Claude Code and
Codex, a project workflow installer, verification, and rollback receipts.

Optional third-party skill packs are not bundled by default. The
`skill-packs/optional/` directory is kept as a reserved extension point. Current
third-party upstreams and candidate skills are listed by GitHub author in
`docs/third-party-skills.md`.

CodeGraph MCP is documented as a supported installable MCP server in
`docs/mcp-servers.md`. It is not installed silently by `bootstrap.sh`.

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

`scripts/kit-install.sh` is the public installer entrypoint. It delegates
project-level workflow writes to `scripts/install-suite-to-project.sh` and, for
the Full profile, runtime writes to `scripts/bootstrap.sh`. It does not
overwrite silently.

It writes or updates:

- `AGENTS.md`
- `CLAUDE.md`
- `docs/agent-workflow/`
- `config/agent-project-profile.yaml`

It creates a timestamped backup before `--apply` and prints a receipt with the
rollback location.

## Runtime Install

Full profile runtime install places global rules and skills into a target home.
Default mode is dry-run.

```bash
bash scripts/kit-install.sh --profile full --scope runtime --dry-run
bash scripts/kit-install.sh --profile full --scope runtime --apply
bash scripts/diff-local.sh
```

DIY/Core intentionally skips bundled global skill runtime installation unless a
downstream user supplies their own capability implementations.

## Environment Doctor

`scripts/kit-doctor.sh` is the public environment checker. It runs the suite
doctor, security doctor, optional target-project conflict checks, and the update
gate.

```bash
bash scripts/kit-doctor.sh doctor --profile full --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
bash scripts/kit-doctor.sh update --diff
bash scripts/kit-doctor.sh update --apply
```

`update --check` is read-only. `update --diff` fetches into `FETCH_HEAD` and
prints the diff summary. `update --apply` requires a clean suite worktree, uses
fast-forward only, then runs verification and the security doctor.

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
