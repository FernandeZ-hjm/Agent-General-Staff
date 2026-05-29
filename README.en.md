# Dongmenlaohu Multi-Agent Engineering Kit

[中文版](README.md)

An open, source-available development kit for multi-agent engineering collaboration. It helps build an AI coding environment that is governable, reproducible, verifiable, and rollback-ready.

## Personal Background

Hi, I am Dongmen Laohu.

I used to work as an auditor. Later, I became a tech media writer out of personal interest. I now lead public relations at a hard-tech company. As this project becomes public, I have one more identity: a vibe-coding architect, beginner edition.

I did not arrive here by accident.

In the second half of 2025, I was still writing as a tech media columnist. Several friends in the AI industry repeatedly recommended Gemini 3.0 Pro to me. Their reason was simple: at that time, its text generation quality was clearly ahead of other top models. Within a quarter, I had become a heavy Gemini user. I also started using Gem, Gemini's built-in skill mechanism, to rebuild my writing and research workflow.

By early 2026, that methodology had already helped me earn my first real return from AI-assisted work.

After joining a company, the problem became more concrete.

I was facing a fast-growing hard-tech business. The team was small. The workload was not. Marketing, public relations, external announcements, and internal strategy support all required higher-frequency and more stable output. Without AI to multiply my productivity, I could not realistically carry that workload alone.

So, about a month ago, I installed Claude Code plus DeepSeek. About half a month ago, I added Codex and Cursor to develop my own projects.

A new problem appeared quickly: each agent can be strong on its own, but how do several agents collaborate stably inside the same engineering system? How can human intent stay intact across multiple agent tools instead of drifting or being distorted?

I did not come from a large tech company. I also do not have an AI lab's resource pool behind me. My budget is limited. I cannot run Opus or GPT with unlimited usage forever.

So I had to choose a more cost-effective path: let DeepSeek perform at its best inside clear coding conventions, behavioral boundaries, and workflow rules.

Only then can my projects become truly engineered products, not just toys.

I also went through a stage many vibe-coding beginners know well: installing all kinds of skills, hooks, and MCP servers without much restraint.

But I soon found that more tools also meant more problems. Some had messy versions. Some conflicted with each other. Some triggered at the wrong time. Some had unclear update paths. They were supposed to improve productivity. Instead, they could drag attention away from development and back into tool management.

So I started doing something else: putting these capabilities into a governable order. Things that should trigger automatically should do so. Things that require human confirmation should stop and ask. Things that should update must have an update path. Things that can go wrong must have a rollback path.

This project borrows experience from several directions.

One part comes from popular open-source projects on GitHub, including many agent skills. They strengthen the Claude Code framework and help DeepSeek work inside clearer boundaries.

Another part comes from the latest Claude Code workflow practices. Their real value is not a beautiful prompt. Their value is strong boundaries, traceability, and explicit delivery scope. How a task starts, who executes it, who reviews it, what counts as done, and how to roll back after failure all need rules.

After a month, this engineering kit is finally ready to be shown publicly.

I have to be honest: many capabilities inside it were not originally invented by me. What I did was assemble scattered skills, rules, task cards, hooks, and verification gates into an engineering kit that can support individual developers and small teams.

For vibe-coding beginners, this kit does not let you skip engineering. It does the opposite. It moves the engineering order you will eventually need into your AI coding environment earlier.

Once installed, it should make two things easier for small and medium projects: higher development efficiency and more stable delivery boundaries.

## Project Versions

This repository has two public versions:

- **DIY version**: for mature AI practitioners. It keeps only my governance framework, task-card protocol, runtime adapter rules, verification gates, project templates, and installation validators.
- **Full version**: for beginners. A single command can reproduce my complete development workflow.

## Quick Start

Preview first. Do not write into your environment too early.

Full Installer installs the complete development kit into a target project and connects it to the local agent runtime. For the first run, read the dry-run output only:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

If you only want the governance framework and project workflow, without installing the full skill stack by default, use DIY/Core:

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

After reviewing the dry-run output, apply only when the result matches your expectation:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

Check local conflicts, installed runtime drift, target project state, and available public updates:

```bash
bash scripts/kit-doctor.sh doctor --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
```

Third-party skill sources are listed here:

```text
docs/third-party-skills.md
```

Supported MCP servers are listed here:

```text
docs/mcp-servers.md
```

## Version Details

### DIY/Core

DIY/Core is for people who already have their own toolchain.

It does not decide which third-party skills you should use. It provides engineering order: cache-stable task cards, light/medium/heavy routing, review and verification gates, project profiles, runtime adapter conventions, local memory capsules, dry-run, rollback, diff, and doctor tools.

Core does not assume any third-party skill implementation exists. The rule layer refers only to capability slots. If a capability is not installed, the system should degrade instead of breaking.

### Full Installer

Full Installer is for people who want to quickly reproduce a complete development environment.

It includes everything in Core, plus required global skills, hook normalization for Claude Code and Codex, a project workflow installer, verification flow, and rollback receipts.

Optional third-party skill packs are not bundled by default. `skill-packs/optional/` is only a reserved extension point. Current third-party upstreams and candidate skills are listed by GitHub author in `docs/third-party-skills.md`.

CodeGraph MCP is documented as an installable MCP server in `docs/mcp-servers.md`. `bootstrap.sh` does not install it silently.

Full Installer also avoids private project bindings. Target-project data comes from CLI arguments and `config/agent-project-profile.yaml`.

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

## Project Integration

`scripts/kit-install.sh` is the public installation entrypoint.

It coordinates two layers. Project-level workflow writes are handled by `scripts/install-suite-to-project.sh`. Runtime writes for the Full profile are handled by `scripts/bootstrap.sh`. It does not overwrite files silently.

It writes or updates:

- `AGENTS.md`
- `CLAUDE.md`
- `docs/agent-workflow/`
- `config/agent-project-profile.yaml`

Before `--apply`, it creates a timestamped backup and prints a receipt containing the rollback location.

## Runtime Install

The Full profile runtime install places global rules and skills into the target home. The default mode is still dry-run.

```bash
bash scripts/kit-install.sh --profile full --scope runtime --dry-run
bash scripts/kit-install.sh --profile full --scope runtime --apply
bash scripts/diff-local.sh
```

DIY/Core intentionally skips bundled global skill runtime installation. Unless downstream users provide their own capability implementations, it only provides the governance framework.

## Environment Doctor

`scripts/kit-doctor.sh` is the public environment checker.

It runs the suite doctor, security doctor, optional target-project conflict checks, and the update gate.

```bash
bash scripts/kit-doctor.sh doctor --profile full --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
bash scripts/kit-doctor.sh update --diff
bash scripts/kit-doctor.sh update --apply
```

`update --check` is read-only. `update --diff` fetches into `FETCH_HEAD` and prints a diff summary. `update --apply` requires a clean suite worktree, uses fast-forward only, then runs verification and the security doctor.

The installer backs up replaced files under:

```text
$HOME/.agents/backups/
```

## Safety Model

Automation must not run destructive commands or external dependency installation commands without explicit approval.

The manifest forbids these behaviors by default:

- `git push --force`
- `curl | bash`
- `npx skills add/remove/update`
- unreviewed dependency installation
- deleting the user's global skill directory

## Public Boundary

This public repository must not include private project names, private release topology, personal persona profiles, private sync registries, real secrets, tokens, or API keys.

Run the boundary scan before publishing:

```bash
bash scripts/security-doctor.sh
```
