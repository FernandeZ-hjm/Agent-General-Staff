---
name: superpowers
description: >
  Internal engineering workflow parent. Use only when AGS Skill Resolver or a
  validated task card names a Superpowers entrypoint, or when the user explicitly
  invokes Superpowers. Load exactly that named PLAYBOOK resource. Do not
  auto-select this parent from generic design/build/debug/review/skill-authoring
  words, ordinary content transformations, or approved direct edits.
---

# AGS Superpowers Adapter

This is the optional AGS-modified compatibility parent for selected workflows
derived from `obra/superpowers`. It is not the upstream package, is not official,
and does not imply sponsorship, authorization, or endorsement by Jesse Vincent,
Prime Radiant, or the upstream project.

The distribution id is `ags-superpowers-adapter`; after an explicit AGS Skill
plan/apply transaction, the host compatibility id remains `superpowers` so
existing parent routes do not need to be rewritten.

This is the single host-visible entry for the adapted Superpowers workflows.
Choose the narrowest matching playbook, read only its `PLAYBOOK.md` resource,
and follow it. Playbooks are resources, not independently discoverable skills.
Do not preload or combine unrelated playbooks.

| Current intent or phase | Playbook |
|---|---|
| Explicit brainstorming or unresolved system/cross-module architecture | `playbooks/brainstorming/PLAYBOOK.md` |
| Independent parallel tasks | `playbooks/dispatching-parallel-agents/PLAYBOOK.md` |
| Execute an approved written plan | `playbooks/executing-plans/PLAYBOOK.md` |
| Finish a development branch | `playbooks/finishing-a-development-branch/PLAYBOOK.md` |
| Evaluate received review feedback | `playbooks/receiving-code-review/PLAYBOOK.md` |
| Request a code review | `playbooks/requesting-code-review/PLAYBOOK.md` |
| Execute a plan through independent task agents | `playbooks/subagent-driven-development/PLAYBOOK.md` |
| Skill Resolver explicitly selects the internal systematic-debugging fallback | `playbooks/systematic-debugging/PLAYBOOK.md` |
| Explicit test-first development | `playbooks/test-driven-development/PLAYBOOK.md` |
| Create or use an isolated worktree | `playbooks/using-git-worktrees/PLAYBOOK.md` |
| Explain the Superpowers workflow itself | `playbooks/using-superpowers/PLAYBOOK.md` |
| Before claiming completion or success | `playbooks/verification-before-completion/PLAYBOOK.md` |
| Turn an approved design into an implementation plan | `playbooks/writing-plans/PLAYBOOK.md` |
| `skill-creator` is unavailable and Skill Resolver explicitly selects the portable fallback | `playbooks/writing-skills/PLAYBOOK.md` |

## Loading rule

1. Use the current task phase and a precise task characteristic, not generic
   creation words such as “design”, “build”, or “implement”, to choose a playbook.
2. Load one primary playbook. Load another only after the task changes phase or the
   selected playbook explicitly requires it.
3. Ordinary content transformations and execution of an already approved design
   do not enter brainstorming. Use specialist capabilities for module design,
   domain modeling, architecture improvement, prototypes, and plan grilling.
4. `grill-with-docs` is manual-only when the user explicitly asks for a
   documentation-grounded interview or ADR/glossary workflow. Difficult diagnosis
   routes through the independent `diagnosing-bugs` capability; skill authoring
   routes through host `skill-creator` when available. Do not load a second bundled
   implementation for either demand.
5. Never treat `using-superpowers` as a conversation-start hook.
6. Legacy text such as `superpowers:<name>` inside a PLAYBOOK means “return to
   this parent and load `playbooks/<name>/PLAYBOOK.md`”. It is never a request to
   invoke a separately discoverable child skill.
