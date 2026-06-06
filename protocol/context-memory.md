# Context Memory Protocol

`context-memory.md` defines how AGS-integrated projects can maintain durable
memory without shipping private state in the AGS public suite.

## Purpose

Context memory gives agents a compact, project-local source of truth before
they execute work. It should help answer:

- what the project is for;
- what must not be changed;
- which verification commands matter;
- which prior decisions should survive context compaction;
- where recent task archives live.

## Public Suite Default

The AGS public edition ships the memory protocol and blank templates only. It
does not ship real context capsules, real task archives, receipts, delivery
reports, local user history, or machine-specific paths.

## Expected User Growth

When a user integrates AGS into a project, their local memory may grow under an
operator-chosen memory root such as `$AGS_MEMORY_DIR` or a user-local agent
memory directory. The project may then maintain:

- a context capsule;
- a task-memory index;
- task archives;
- delivery receipts;
- review and verification notes.

## Safety Rules

- Do not commit secrets, credentials, private tokens, or customer data.
- Do not publish local memory archives in the AGS public distribution.
- Keep public templates generic and free of machine-specific paths.
- Treat memory as advisory context, not as proof of current repository state.
