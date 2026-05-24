---
name: improve-codebase-architecture
description: >
  Architecture improvement skill for tangled modules, repeated bugs, hard-to-test code, unclear boundaries,
  or requests like refactor/technical debt/simplify architecture. Use before broad refactors and whenever a bug
  shows missing seams, hidden coupling, or shallow modules.
---

# Improve Codebase Architecture

Use this to make a codebase easier to change without turning the task into a rewrite. Surface architectural friction and propose deepening opportunities — refactors that turn shallow modules into deep ones.

See [LANGUAGE.md](LANGUAGE.md) for the shared vocabulary (module, interface, depth, seam, adapter, leverage, locality). Use these terms exactly — don't drift into "component," "service," "API," or "boundary."

## Principles

- **The deletion test.** Imagine deleting the module. If complexity vanishes, it was a pass-through. If complexity reappears across N callers, the module was earning its keep.
- **The interface is the test surface.** Callers and tests cross the same seam. If you want to test past the interface, the module is probably the wrong shape.
- **One adapter means a hypothetical seam. Two adapters means a real one.** Don't introduce a seam unless something actually varies across it.

## Workflow

1. Understand the domain and decisions.
   - Read AGENTS.md/CLAUDE.md.
   - Read CONTEXT.md/docs/agents/domain.md and docs/adr/ if present.
   - Inspect callers, public interfaces, data ownership, and tests around the painful area.
   - Walk the codebase and note where you experience friction:
     - Where does understanding one concept require bouncing between many small modules?
     - Where are modules shallow — interface nearly as complex as the implementation?
     - Where have pure functions been extracted just for testability, but the real bugs hide in how they're called (no locality)?
     - Where do tightly-coupled modules leak across their seams?
   - Apply the deletion test to anything suspect: would deleting it concentrate complexity, or just move it?

2. Find change pain, not aesthetic flaws.
   - Repeated edits across many files for one concept.
   - Tests coupled to internals.
   - Boolean/config explosions.
   - Cross-layer knowledge leaks.
   - Duplicate business rules.
   - Pure functions extracted only for testability while real behavior remains tangled.

3. Propose small architecture moves.
   - Favor deep modules: simple interface, meaningful behavior inside.
   - Preserve existing external contracts unless the user approves a migration.
   - Prefer one vertical slice over a sweeping refactor.
   - Classify dependencies per [DEEPENING.md](DEEPENING.md) (in-process, local-substitutable, ports & adapters, mock) to determine test strategy.
   - Include the verification command for each proposed change.

4. Present options before editing.
   - Option name.
   - Files/modules touched.
   - Benefit.
   - Risk/blast radius.
   - Test strategy.
   - Recommended choice.

   Once the user picks a candidate, walk the design tree together:
   - Constraints, dependencies, the shape of the deepened module, what sits behind the seam.
   - Which tests survive, which become waste.
   - If the user wants to explore alternative interfaces, use the parallel sub-agent pattern in [INTERFACE-DESIGN.md](INTERFACE-DESIGN.md).

5. When approved, execute incrementally.
   - Use tdd for behavior changes.
   - Keep commits/patches scoped.
   - Run verification after every meaningful step.

## Side effects

- **Naming a deepened module after a concept not in CONTEXT.md?** Add the term — same discipline as grill-with-docs. See [CONTEXT-FORMAT.md](../grill-with-docs/CONTEXT-FORMAT.md).
- **User rejects a candidate with a load-bearing reason?** Offer an ADR so future reviews don't re-suggest it. See [ADR-FORMAT.md](../grill-with-docs/ADR-FORMAT.md). Only offer when the reason would actually be needed by a future explorer — skip ephemeral or self-evident reasons.

## Output shape

Use: Findings -> Options -> Recommendation -> Verification Plan. Keep it concrete with file paths and symbols.
