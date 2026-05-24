---
name: prototype
description: >
  Throwaway prototype skill for uncertain designs, state machines, parsing rules, APIs, workflows, or UI approaches.
  Use when a small experiment will answer design questions faster than debating or implementing production code.
---

# Prototype

Use this to learn quickly. A prototype is evidence, not production implementation.

## Pick the right shape

- **"Does this logic / state model feel right?"** → [LOGIC.md](LOGIC.md). Build a tiny interactive terminal app that pushes the state machine through cases hard to reason about on paper.
- **"What should this look like?"** → [UI.md](UI.md). Generate several radically different UI variations on a single route, switchable via a floating bottom bar.

The two branches produce very different artifacts. If the question is genuinely ambiguous and the user isn't reachable, default to whichever matches the surrounding code (backend module → logic; page/component → UI) and state the assumption.

## Workflow

1. State the question.
   - What decision will this prototype answer?
   - What must it ignore to stay cheap?
   - What result would make you abandon the idea?

2. Choose the smallest artifact.
   - Terminal script for business logic, state machines, parsing, ingestion, or APIs.
   - One temporary route/page for UI alternatives.
   - A focused fixture set when data shape is the uncertainty.

3. Isolate it.
   - Put temporary artifacts in an obvious scratch location.
   - Do not wire into production paths unless the user explicitly approves.
   - Do not install dependencies without confirmation.

4. Compare outcomes.
   - Prefer 2-3 materially different approaches when design space is unclear.
   - Record what was learned, what failed, and what should be carried into the real implementation.

5. Clean up or promote intentionally.
   - Remove throwaway files before final unless the user asks to keep them.
   - If promoting, rewrite to match project style and add normal verification/tests.

## Hard rules

1. **Throwaway from day one, and clearly marked as such.** Locate prototype code close to where it will be used, but name it so a casual reader knows it's a prototype.
2. **One command to run.** Use the project's existing task runner — the user must be able to start it without thinking.
3. **No persistence by default.** State lives in memory. If the question involves a database, hit a scratch DB or local file with a clear "PROTOTYPE — wipe me" name.
4. **Skip the polish.** No tests, no error handling beyond what makes the prototype runnable, no abstractions.
5. **Surface the state.** After every action (logic) or variant switch (UI), print or render the full relevant state.
6. **Delete or absorb when done.** Don't leave it rotting in the repo.

## When done

The answer is the only thing worth keeping. Capture it somewhere durable (commit message, ADR, issue, or `NOTES.md` next to the prototype) along with the question it was answering. Then either delete the prototype or fold the validated decision into the real code.
