# Third-Party Development Skill Recommendations

This document lists **recommended** third-party development skills that enhance
the full AGS development experience. These are **not installed by default** —
each must be manually installed by the user.

## Important: Recommendations Only

- **No skills are installed automatically.**
- **No repositories are cloned during AGS installation.**
- **No files are written to `$HOME/.agents/skills/`, `$HOME/.codex/skills/`, or `$HOME/.codex/plugins/cache/`.**
- Each skill entry below lists its source, purpose, risk level, and **manual installation steps**.

## Core Development Skills

### Engineering Workflow (brainstorm/superpowers)
- **Purpose**: Structured engineering workflow — brainstorm → plan → execute → review → verify → worktree isolation → parallel execution
- **Source**: GitHub (repository varies by user; check community forks)
- **Risk**: Low (read-only orchestration, no production writes)
- **Install Location**: `$HOME/.agents/skills/superpowers/`
- **Manual Install**:
  ```bash
  # Step 1: Find a community-maintained copy of the superpowers skill pack
  # Step 2: Clone or copy to your skills directory
  mkdir -p ~/.agents/skills/superpowers
  # Step 3: Copy SKILL.md and playbooks/ into ~/.agents/skills/superpowers/
  ```

### Systematic Debugging (diagnose)
- **Purpose**: HITL debugging loop with evidence-chain tracing, root-cause isolation, reproduction-first discipline
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only diagnosis; no production writes)
- **Install Location**: `$HOME/.agents/skills/diagnose/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/diagnose
  # Copy SKILL.md, scripts/, references/ into ~/.agents/skills/diagnose/
  ```

### Test-Driven Development (tdd)
- **Purpose**: Red-green-refactor cycle, mocking patterns, interface design from tests
- **Source**: GitHub (community-maintained)
- **Risk**: Low (local development only)
- **Install Location**: `$HOME/.agents/skills/tdd/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/tdd
  # Copy SKILL.md, deep-modules.md, interface-design.md, mocking.md, tests.md, refactoring.md
  ```

### Code Review (code-review)
- **Purpose**: Code review across correctness bugs, simplification, efficiency cleanups
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only review; posts comments only when explicitly requested)
- **Install Location**: `$HOME/.agents/skills/code-review/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/code-review
  # Copy SKILL.md and references/
  ```

### Git Worktrees (using-git-worktrees)
- **Purpose**: Isolated git worktree management for parallel feature branches
- **Source**: GitHub (community-maintained)
- **Risk**: Low (local git operations only)
- **Install Location**: `$HOME/.agents/skills/using-git-worktrees/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/using-git-worktrees
  # Copy SKILL.md
  ```

## Quality & Verification

### Auto-Verify
- **Purpose**: Automatic behavior verification on task completion; confirms changes work before declaring done
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only verification)
- **Install Location**: `$HOME/.agents/skills/auto-verify/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/auto-verify
  # Copy SKILL.md
  ```

### Conventional Commits (caveman-commit)
- **Purpose**: Generate concise Conventional Commit messages from diff analysis
- **Source**: GitHub (community-maintained)
- **Risk**: Low (generates commit messages; does not commit without confirmation)
- **Install Location**: `$HOME/.agents/skills/caveman-commit/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/caveman-commit
  # Copy SKILL.md, README.md
  ```

### Code Review Lite (caveman-review)
- **Purpose**: Short, actionable code review feedback — correctness and simplification
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only review output)
- **Install Location**: `$HOME/.agents/skills/caveman-review/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/caveman-review
  # Copy SKILL.md, README.md
  ```

## Architecture & Planning

### Zoom Out
- **Purpose**: High-level architecture context, risk assessment before changes, module-in-system view
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only architecture analysis)
- **Install Location**: `$HOME/.agents/skills/zoom-out/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/zoom-out
  # Copy SKILL.md
  ```

### Improve Codebase Architecture
- **Purpose**: Architecture improvement patterns — boundary hardening, testability, dep cleanup
- **Source**: GitHub (community-maintained)
- **Risk**: Medium (may propose structural refactors; always requires human approval)
- **Install Location**: `$HOME/.agents/skills/improve-codebase-architecture/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/improve-codebase-architecture
  # Copy SKILL.md, DEEPENING.md, INTERFACE-DESIGN.md, LANGUAGE.md
  ```

### Requirements Clarification (grill-with-docs)
- **Purpose**: Align requirements with project docs before implementation; ask clarifying questions
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only alignment check)
- **Install Location**: `$HOME/.agents/skills/grill-with-docs/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/grill-with-docs
  # Copy SKILL.md, ADR-FORMAT.md, CONTEXT-FORMAT.md
  ```

## Supply Chain Security

### Supply Chain Risk Auditor
- **Purpose**: Dependency supply chain risk audit — license compliance, maintainer health, vulnerability scanning
- **Source**: GitHub (community-maintained)
- **Risk**: Low (read-only audit report)
- **Install Location**: `$HOME/.agents/skills/supply-chain-risk-auditor/`
- **Manual Install**:
  ```bash
  mkdir -p ~/.agents/skills/supply-chain-risk-auditor
  # Copy SKILL.md, resources/
  ```

---

## How to Install a Skill

For each skill above:

1. **Find the skill source** — community-maintained copies are available on GitHub
2. **Clone or download** the skill directory to the install location listed
3. **Verify the skill** — check SKILL.md for configuration requirements
4. **Test** — run a simple task to verify the skill is loaded by your agent

**AGS will never clone, install, or configure skills on your behalf.** Every skill
above requires explicit manual action. The recommendations here are informational only.

## Risk Levels

| Level | Description |
|---|---|
| **Low** | Read-only, no production writes, no external network calls beyond documentation fetch |
| **Medium** | May propose structural changes; always requires human approval before execution |
| **High** | Not recommended without full security review |

All skills listed above are **Low** or **Medium** risk. No **High** risk skills
are recommended by default.
