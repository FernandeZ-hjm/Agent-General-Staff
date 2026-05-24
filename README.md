# Dongmenlaohu Multi-Agent Engineering Kit

V1.0 public release.

面向 Codex、Cursor 和 Claude Code 的可迁移多 Agent 工程化开发套件。它把角色分工、任务卡协议、开发技能、验证流程、交付报告和本地技能治理机制打包成一套可下载、可验证、可安装的工作流骨架。

## What It Is

This kit is for developers who want a more disciplined multi-agent workflow:

- Codex / Cursor handle diagnosis, design, task routing, and review.
- Claude Code executes bounded task cards and reports delivery status.
- Skills enforce repeatable behavior for brainstorming, debugging, verification, review, commits, and project onboarding.
- Governance scripts keep local skills and upstream changes under human-reviewed control.

It is not tied to one application codebase. It is a portable workflow layer that can be installed into a local development environment and then integrated into individual projects.

## Download

Clone with SSH:

```bash
git clone git@github.com:FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit.git
cd Dongmenlaohu-multi-agent-engineering-kit
```

Clone with HTTPS:

```bash
git clone https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit.git
cd Dongmenlaohu-multi-agent-engineering-kit
```

Download ZIP:

```text
https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit/archive/refs/heads/main.zip
```

V1.0 source ZIP:

```text
https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit/archive/refs/tags/V1.0.zip
```

V1.0 source TAR:

```text
https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit/archive/refs/tags/V1.0.tar.gz
```

## Quick Start

Preview installation first. The installer defaults to dry-run and does not write files unless `--apply` is used.

```bash
bash scripts/bootstrap.sh --dry-run
```

Verify the suite:

```bash
bash scripts/verify.sh
```

Install only after reviewing the dry-run output:

```bash
bash scripts/bootstrap.sh --apply
```

## What Gets Installed

Required global rules:

- `global-rules/SOUL.md` -> `$HOME/.agents/rules/SOUL.md`
- `global-rules/core.md` -> `$HOME/.agents/rules/core.md`
- `global-rules/RTK.md` -> `$HOME/.codex/RTK.md`

Required skills are installed under:

```text
$HOME/.agents/skills/
```

The required skill set includes:

- `auto-brainstorm`
- `auto-debug`
- `auto-verify`
- `claude-execution-prompt-maker`
- `claude-delivery-report`
- `tdd`
- `diagnose`
- `zoom-out`
- `caveman-commit`
- `caveman-review`
- `finishing-a-development-branch`
- `using-git-worktrees`
- `webapp-testing`
- `grill-with-docs`
- `improve-codebase-architecture`
- `prototype`
- `database-migration`
- `supply-chain-risk-auditor`
- `skill-creator`
- `graphify-project-map`
- `superpowers`

See `manifests/suite.yaml` for the machine-readable install manifest.

## Repository Layout

```text
.
├── README.md
├── AGENT_SUITE_PROTOCOL.md
├── manifests/
│   ├── suite.yaml
│   └── skills.lock.example.yaml
├── protocol/
├── governance/
├── task-modules/
├── templates/fallback-task-cards/
├── project-integration/
├── scripts/
├── global-rules/
└── global-skills/
```

Key files:

- `AGENT_SUITE_PROTOCOL.md`: suite-level role model, task routing, safety rules, and delivery report contract.
- `protocol/task-card-template.md`: project task-card skeleton.
- `templates/fallback-task-cards/`: fallback task cards for projects without local protocol files.
- `project-integration/`: templates for adding `AGENTS.md`, `CLAUDE.md`, and agent workflow docs to a project.
- `governance/`: skill/plugin synchronization rules and adoption logs.
- `scripts/bootstrap.sh`: dry-run first installer.
- `scripts/verify.sh`: suite integrity verification.
- `scripts/diff-local.sh`: compare installed local files with this suite.
- `scripts/rollback.sh`: rollback helper.
- `scripts/govern-new-skills.sh`: scan/adopt/ignore workflow for new skills.

## Safety Model

The kit is intentionally conservative:

- `bootstrap.sh` defaults to dry-run.
- `--apply` creates backups before writing.
- No dependency installation is performed automatically.
- Secrets, tokens, `.env` files, SSH private keys, and Keychain data are not copied.
- Upstream skill changes are not auto-applied. They must go through check, diff, review, and explicit adoption.
- Destructive commands such as force push, broad deletion, and blind overwrite are forbidden by the suite protocol.

## Project Integration

For a new project, start from:

```text
project-integration/AGENTS.md.template
project-integration/CLAUDE.md.template
protocol/
```

The intended pattern is:

1. Put project-specific commands, paths, and safety constraints in the project repo.
2. Keep reusable multi-agent workflow rules in this suite.
3. Use task cards to hand execution work from Codex/Cursor to Claude Code.
4. Require verification and a concise delivery report before treating work as complete.

## Verification

Run:

```bash
bash scripts/verify.sh
```

Expected result:

```text
Status  : PASSED
Errors  : 0
Warnings: 0
```

## Version

Current release: `V1.0`

See `CHANGELOG.md` for release notes.
