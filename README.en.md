# Dongmenlaohu Multi-Agent Governance Suite

[中文版](README.md)

This repository is a governance suite for multi-agent Vibe-coding.

It is not a magic prompt pack. It is a set of rules, task-card contracts, runtime adapters, verification gates, rollback paths, and public-boundary checks for people who want Claude Code, Codex, Cursor, DeepSeek, and other Agent tools to work inside the same project without drifting all over the place.

The goal is simple: make Vibe-coding more stable for individual developers and small teams.

Release notes are available in [CHANGELOG.md](CHANGELOG.md).

## What It Solves

One Agent can be useful.

Several Agents working on the same repository can become messy very quickly.

Human intent drifts. Task boundaries expand. Verification gets skipped. Third-party skills pile up. Eventually, the work stops feeling like product development and starts feeling like toolchain management.

This governance suite tries to put order around that.

Before a task starts, it should have a clear goal. During execution, it should respect boundaries. Before anyone claims completion, verification should run. When different tools collaborate, they should share a stable task-card format instead of improvising their own prompt shape every time.

## Public Profiles

This repository has two public profiles.

### DIY/Core

DIY/Core is for people who already have their own toolchain.

It provides the governance framework only. You get the task-card protocol, runtime adapter rules, verification gates, project templates, doctor checks, local diff checks, rollback support, and local memory entrypoints.

It does not decide which third-party skills you should use.

### Full Installer

Full Installer is for people who want to reproduce the complete workflow quickly.

It includes everything in DIY/Core, plus suite-managed global rules and skills, hook normalization for Claude Code and Codex, the project workflow installer, verification flow, and rollback receipts.

If you are setting up a Vibe-coding environment for the first time, start with Full Installer. Get it running first, then inspect each layer.

## Step-by-Step Install

The most important rule is this:

Do not apply on the first run.

Use dry-run first.

Dry-run previews what the installer would do without writing files. Review that output before applying anything.

### 1. Prepare A Target Project

Assume your project lives here:

```bash
/Users/you/projects/my-project
```

If it does not exist yet:

```bash
mkdir -p /Users/you/projects/my-project
```

If it is a git repository, check its status first:

```bash
cd /Users/you/projects/my-project
git status
```

If the project already has many uncommitted changes, handle those first. The installer can still run, but it becomes harder to tell which files were changed by you and which files were written by the installer.

### 2. Enter This Governance Suite

If you already have the repository:

```bash
cd /path/to/Dongmenlaohu-multi-agent-engineering-kit
```

If not, clone it:

```bash
git clone https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit.git
cd Dongmenlaohu-multi-agent-engineering-kit
```

### 3. Check The Suite Itself

```bash
bash scripts/verify.sh
bash scripts/security-doctor.sh
```

This checks suite integrity and public-boundary safety. If these fail, do not install the suite into another project yet.

### 4. Choose A Profile

For the complete workflow, use Full Installer:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

For the governance framework only, use DIY/Core:

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

Parameter notes:

`--target-project` is the target project path.

`--project-name` is the human-readable project name.

`--project-slug` is the stable project identifier. Use lowercase ASCII letters, numbers, and hyphens.

### 5. Review Dry-Run Output

Dry-run usually previews writes like these:

```text
Project Agent entrypoint, AGENTS.md
Claude Code project protocol, CLAUDE.md
Agent workflow docs, docs/agent-workflow/
Project profile config, config/agent-project-profile.yaml
```

Full Installer may also touch local runtime rules, skills, and hooks.

If the dry-run output plans to overwrite something important, stop and inspect it first.

### 6. Apply After Review

Apply Full Installer:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

Apply DIY/Core:

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

Before writing, the installer creates backups under:

```text
$HOME/.agents/backups/
```

After installation, it prints a receipt with the written files and rollback location.

### 7. Run Post-Install Doctor

For Full Installer:

```bash
bash scripts/kit-doctor.sh doctor \
  --profile full \
  --target-project /Users/you/projects/my-project
```

For DIY/Core:

```bash
bash scripts/kit-doctor.sh doctor \
  --profile diy \
  --target-project /Users/you/projects/my-project
```

### 8. Runtime-Only Install

If the project workflow is already installed and you only want to update local runtime files, preview first:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --scope runtime \
  --dry-run
```

Then apply:

```bash
bash scripts/kit-install.sh \
  --profile full \
  --scope runtime \
  --apply
```

DIY/Core intentionally does not install the bundled full skill runtime. It provides the governance framework and leaves the toolchain to you.

### 9. Check Local Drift

```bash
bash scripts/diff-local.sh
```

### 10. Check For Updates

Check only:

```bash
bash scripts/kit-doctor.sh update --check
```

Inspect diff:

```bash
bash scripts/kit-doctor.sh update --diff
```

Apply after review:

```bash
bash scripts/kit-doctor.sh update --apply
```

`update --apply` requires a clean suite worktree, uses fast-forward only, then runs verification and the security doctor.

## Important Files

File paths stay in English for command-line, git, Claude Code, Codex, and runtime compatibility.

```text
Install entrypoint, scripts/kit-install.sh
Environment doctor, scripts/kit-doctor.sh
Suite verification, scripts/verify.sh
Public-boundary scanner, scripts/security-doctor.sh
Task-card validator, scripts/validate-task-card.sh
Agent workflow protocol, protocol/
Project templates, project-integration/
Global rules, global-rules/
Global skills, global-skills/
Task-card templates, templates/
Third-party skill notes, docs/third-party-skills.md
MCP server notes, docs/mcp-servers.md
```

## Safety Boundary

This public repository must not include private project names, private release topology, personal persona profiles, private sync registries, real secrets, tokens, or API keys.

It also does not silently:

- force-push git branches
- run `curl | bash`
- install third-party dependencies
- delete your global skills directory
- install optional skill packs

CodeGraph MCP is documented, but `bootstrap.sh` does not install it silently.

The point is not to make Vibe-coding look cooler.

The point is to make it less mystical and more governable inside real projects.
