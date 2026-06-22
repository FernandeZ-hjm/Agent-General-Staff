# Third-Party Development Skill Recommendations

This document mirrors `manifests/skill-recommendations.yaml`, the single public
recommendation source consumed by `ags setup`. These are **recommendations
only** — each must be **manually installed** by the user.

## Important: Recommendations Only

- **No skills are installed automatically.** AGS never clones, installs,
  downloads, copies, or packages these bodies.
- **No repositories are cloned during AGS installation.**
- **No files are written** to `$HOME/.agents/skills/`, `$HOME/.codex/skills/`,
  or any host skill directory, and no host config or skill thin-index is written
  for a recommendation-only source.
- Every entry uses its **upstream canonical name** (no local aliases) and lists
  its source, tier, risk, and **manual installation steps**.

> **Recommendation-only ≠ AGS-hosted body.** An `external` source is an upstream
> pointer for manual install, not an AGS-locally-hosted canonical skill body.
> Only a real local `SKILL.md` under an AGS canonical store participates in
> thin-index writes.

## Excluded by policy

- `obsidian-vault` — user-personal skill; never a public core recommendation.
- `teach` — not recommended for the public core development surface.

Retired local aliases were removed with **no compatibility rows**; upstream
canonical names are used throughout. See `RELEASE_NOTES.md` for the rename map.

## Core Engineering Workflow

### Engineering Workflow (superpowers)
- **Tier**: core · **Source**: `obra/superpowers` — https://github.com/obra/superpowers · **License**: MIT (retain upstream notice)
- **Purpose**: brainstorm → plan → execute → review → verify → worktree isolation → parallel execution
- **Risk**: Low (read-only orchestration) · **Install**: `$HOME/.agents/skills/superpowers/`

## Matt Flow (mattpocock/skills)

Upstream: https://github.com/mattpocock/skills · each is `external`,
recommendation-only, Low risk unless noted, installed under
`$HOME/.agents/skills/<id>/`.

- **grill-me** — relentless plan/design interrogation before building — `skills/productivity/grill-me`
- **review** — review changes since a fixed point (standards + spec axes) — `skills/in-progress/review`
- **decision-mapping** — map decisions and trade-offs before committing — `skills/in-progress/decision-mapping`
- **resolving-merge-conflicts** — resolve an in-progress git merge/rebase conflict — `skills/engineering/resolving-merge-conflicts`
- **to-prd** — shape requirements into a PRD (optional task-card module; never replaces the task-card contract) — `skills/engineering/to-prd`
- **to-issues** — break work into GitHub issues — `skills/engineering/to-issues`
- **triage** — backlog and issue triage — `skills/engineering/triage`
- **handoff** — clean context handoff to another agent/session — `skills/productivity/handoff`

> `grill-me` (Matt), `grill-with-docs` (Matt, requirements alignment), and the
> Superpowers `grilling` playbook are **distinct** skills — keep their separate
> upstream names; do not alias one to another.

## Quality & Verification

### Test-Driven Development (test-driven-development)
- **Tier**: quality · **Source**: `obra/superpowers` — `skills/test-driven-development` · **License**: MIT
- **Purpose**: red-green-refactor, mocking patterns, interface design from tests
- **Risk**: Low (local development) · **Install**: `$HOME/.agents/skills/test-driven-development/`

## Debugging

### Diagnosing Bugs (diagnosing-bugs)
- **Tier**: debugging · **Source**: `mattpocock/skills` — `skills/engineering/diagnosing-bugs`
- **Purpose**: HITL diagnosis loop with evidence-chain tracing and root-cause isolation
- **Risk**: Low (read-only diagnosis) · **Install**: `$HOME/.agents/skills/diagnosing-bugs/`

## Architecture & Planning

### Requirements Clarification (grill-with-docs)
- **Tier**: architecture · **Source**: `mattpocock/skills` — `skills/engineering/grill-with-docs`
- **Purpose**: align requirements with project docs before implementation
- **Risk**: Low (read-only alignment) · **Install**: `$HOME/.agents/skills/grill-with-docs/`

### Improve Codebase Architecture (improve-codebase-architecture)
- **Tier**: architecture · **Source**: `mattpocock/skills` — `skills/engineering/improve-codebase-architecture`
- **Purpose**: architecture improvement patterns — boundary hardening, testability
- **Risk**: Medium (may propose structural refactors; always requires human approval) · **Install**: `$HOME/.agents/skills/improve-codebase-architecture/`

## Workflow

### Git Worktrees (using-git-worktrees)
- **Tier**: workflow · **Source**: `obra/superpowers` — `skills/using-git-worktrees` · **License**: MIT
- **Purpose**: isolated git worktree management for parallel feature branches
- **Risk**: Low (local git operations) · **Install**: `$HOME/.agents/skills/using-git-worktrees/`

## Supply Chain Security

### Supply Chain Risk Auditor (supply-chain-risk-auditor)
- **Tier**: security · **Source**: community-maintained (select a trusted source)
- **Purpose**: dependency supply-chain risk audit — license compliance, maintainer health, vulnerability scanning
- **Risk**: Low (read-only audit) · **Install**: `$HOME/.agents/skills/supply-chain-risk-auditor/`

---

## How to Install a Skill

For each skill above:

1. **Review the upstream source** at the URL listed (and its license).
2. **Copy or download** the skill directory to the install location.
3. **Verify the skill** — check SKILL.md for configuration requirements.
4. **Test** — run a simple task to confirm your agent loads the skill.

**AGS will never clone, install, or configure skills on your behalf.** Every
skill above requires explicit manual action; the recommendations here are
informational only. Inspect governance status with `ags skill inventory`.

## Risk Levels

| Level | Description |
|---|---|
| **Low** | Read-only, no production writes, no external network calls beyond documentation fetch |
| **Medium** | May propose structural changes; always requires human approval before execution |
| **High** | Not recommended without full security review |

All skills above are **Low** or **Medium** risk. No **High** risk skills are
recommended by default.
