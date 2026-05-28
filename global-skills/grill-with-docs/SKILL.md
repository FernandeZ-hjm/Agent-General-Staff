---
name: grill-with-docs
description: >
  Alignment interview for feature, architecture, refactor, or product changes before implementation.
  Use when the request is ambiguous, has multiple valid designs, touches domain language, or needs a plan/PRD.
  Reads existing project docs and challenges assumptions before writing code.
---

# Grill With Docs

Use this before implementation when misunderstanding would be expensive. The goal is shared language and explicit tradeoffs.

## Domain awareness

During exploration, look for existing documentation:
- Single context (most repos): `CONTEXT.md` at root, `docs/adr/` for decisions.
- Multiple contexts: `CONTEXT-MAP.md` at root listing contexts, their locations, and relationships.
- Create files lazily — only when you have something to write.

See [ADR-FORMAT.md](ADR-FORMAT.md) for decision record format and [CONTEXT-FORMAT.md](CONTEXT-FORMAT.md) for domain glossary format.

## Workflow

1. Read the local context first.
   - AGENTS.md/CLAUDE.md.
   - CONTEXT.md or docs/agents/domain.md if present.
   - docs/adr/ or nearby decision records if present.
   - Existing specs/plans relevant to the change.

2. Summarize what you believe the user wants.
   - Name the domain objects using the repo's vocabulary.
   - Call out assumptions, constraints, non-goals, and affected modules.

3. Ask only high-leverage questions.
   - Prefer 3-7 pointed questions that change implementation choices.
   - Ask one question at a time, waiting for feedback before continuing.
   - If a question can be answered by exploring the codebase, explore instead.
   - If a reasonable default exists, propose it and ask for confirmation.
   - Push back on scope creep, over-design, or unclear success criteria.

   **During the session, apply these techniques:**

   - **Challenge against the glossary.** When the user uses a term that conflicts with existing CONTEXT.md language, call it out: "Your glossary defines X as A, but you seem to mean B — which is it?"
   - **Sharpen fuzzy language.** When terms are vague or overloaded, propose a precise canonical term: "You're saying 'account' — do you mean Customer or User?"
   - **Discuss concrete scenarios.** Invent edge-case scenarios that probe boundaries between concepts and force precision.
   - **Cross-reference with code.** When the user states how something works, check whether the code agrees. Surface contradictions.

4. Produce a decision-ready summary.
   - Goal.
   - Non-goals.
   - Chosen approach and alternatives rejected.
   - Acceptance criteria and verification commands.
   - Docs/ADR updates needed, if any.

5. Do not implement until the user confirms, unless the local AGENTS.md explicitly allows direct execution for this kind of task.

## Side effects

Capture decisions as they crystallize — don't batch them up:

- **When a term is resolved**, update `CONTEXT.md` inline using the format in [CONTEXT-FORMAT.md](CONTEXT-FORMAT.md). CONTEXT.md is a glossary, not a spec or scratch pad.
- **Offer ADRs sparingly** — only when all three are true:
  1. **Hard to reverse** — changing your mind later has meaningful cost.
  2. **Surprising without context** — a future reader would wonder why.
  3. **The result of a real trade-off** — there were genuine alternatives and you picked one for specific reasons.
  Use the format in [ADR-FORMAT.md](ADR-FORMAT.md).

@../superpowers/playbooks/brainstorming/SKILL.md
