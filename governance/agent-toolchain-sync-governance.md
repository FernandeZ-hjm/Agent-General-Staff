# Agent Toolchain Sync Governance

This document defines the public governance rules for refreshing bundled skills,
runtime hooks, and project workflow files.

## Principles

1. Local suite files are canonical until a human accepts an update.
2. Upstream repositories are comparison sources, not automatic overwrite
   sources.
3. Any update must be reviewed as a diff before it changes tracked files.
4. Dependency installation is never implicit.
5. Public releases must pass the boundary scan before publishing.

## Allowed Flow

```text
discover upstream or local candidate
-> generate inventory or diff proposal
-> human review
-> apply selected changes
-> run verify and security doctor
-> commit with clear scope
```

## Skill Updates

Use `manifests/skills-registry.yaml` to track source relationships. For
third-party skills, keep source URL, relationship, and update policy explicit.

Do not run `npx skills add/remove/update` from automation. If a dependency must
be installed, present the command and wait for explicit approval.

## Project Workflow Updates

Registered project workflow sync is optional. Public default has no registered
projects in `governance/project-sync-registry.yaml`.

When a user opts in, `scripts/sync-project-workflow.sh --check` compares
`protocol/` with each target project's `docs/agent-workflow/`. `--apply` may
copy protocol files only when the target policy allows it.

## Release Gate

Before publishing:

```bash
bash scripts/verify.sh
bash scripts/security-doctor.sh
git diff --check
```

The public release must not include machine-local paths, private project names,
personal memory archives, real tokens, or private sync registries.
