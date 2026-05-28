---
name: diagnose
description: >
  Disciplined diagnosis loop for hard bugs, failing tests, broken behavior, and performance regressions.
  Use when the user says diagnose/debug/root cause, reports a bug, says something is broken/failing/slow,
  or when a fix attempt failed and you need evidence before editing.
---

# Diagnose

Use this for bugs that need evidence, not guessing. The loop is: reproduce -> minimize -> hypothesize -> instrument -> fix -> regression-test. Skip phases only when explicitly justified.

## Workflow

1. Build a fresh feedback loop.
   - Capture the exact command, input, stack trace, screenshot, log line, or reproduction path.
   - Prefer an automated failing test. Try in roughly this order:
     - Failing test at the seam that reaches the bug — unit, integration, e2e.
     - Curl/HTTP script against a running dev server.
     - CLI invocation with fixture input, diffing stdout against known-good snapshot.
     - Headless browser script (Playwright/Puppeteer) driving the UI.
     - Replay a captured trace (request/payload/event log) in isolation.
     - Throwaway harness: minimal subset of the system with a single function call.
     - Property/fuzz loop: 1000 random inputs looking for the failure mode.
     - Bisection harness: automate "boot at state X, check, repeat" for `git bisect run`.
     - Differential loop: same input through old vs new version, diff outputs.
     - HITL bash script: drive a human through structured repro steps (see `scripts/hitl-loop.template.sh`).
   - Iterate on the loop: make it faster, sharper, more deterministic. A 2-second deterministic loop beats a 30-second flaky one.
   - For non-deterministic bugs: the goal is a higher reproduction rate. Loop 100×, parallelise, add stress, inject sleeps.
   - If you genuinely cannot build a loop, stop and say so explicitly. List what you tried. Do not proceed without one.

2. Map the relevant system and reproduce.
   - Read AGENTS.md/CLAUDE.md, domain glossary/CONTEXT.md if present, and ADRs near the affected area.
   - Identify the public interface, callers, persistence boundaries, async/jobs, caches, and external services involved.
   - Confirm the loop produces the failure the user described — not a different nearby failure.
   - Verify reproducibility across multiple runs (or, for non-deterministic bugs, at a high enough rate to debug against).

3. Form and test hypotheses.
   - Generate 3-5 ranked hypotheses before testing any. Single-hypothesis generation anchors on the first plausible idea.
   - Each hypothesis must be falsifiable: "If X is the cause, then changing Y will make the bug disappear / changing Z will make it worse."
   - Rank by: recent changes first, bisect-friendly, data-dependent first. Show the ranked list to the user if available.
   - Add temporary instrumentation only where it will distinguish hypotheses. Change one variable at a time.
   - Tag every debug log with a unique prefix (e.g. `[DEBUG-a4f2]`) for single-grep cleanup. Untagged logs survive; tagged logs die.
   - For performance regressions: establish a baseline measurement first, then bisect. Measure first, fix second.
   - Remove temporary logs/scripts before final unless the user asks to keep them.

4. Fix the smallest proven cause.
   - Write the regression test before the fix — but only if there is a correct seam for it.
   - A correct seam exercises the real bug pattern as it occurs at the call site. If no correct seam exists, note it as a finding.
   - Turn the minimised repro into a failing test at that seam, watch it fail, apply the fix, watch it pass.
   - Keep scope inside the failing behavior.
   - If the bug reveals hidden coupling or no testable interface, flag for improve-codebase-architecture with the specific pain point.

5. Lock it in.
   - Re-run the original feedback loop against the un-minimised scenario.
   - Regression test passes (or absence of seam is documented).
   - All `[DEBUG-...]` instrumentation removed.
   - Throwaway prototypes deleted or moved to a clearly-marked debug location.
   - Root cause and the correct hypothesis stated in the commit/PR message.
   - Ask: what would have prevented this bug? If architectural change is the answer, hand off to improve-codebase-architecture.

For deep methodology, see the systematic-debugging playbook.
For HITL reproduction loops, see `scripts/hitl-loop.template.sh`.

@../superpowers/playbooks/systematic-debugging/SKILL.md
