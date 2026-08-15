# AGS Task Handoff Rules

Generate a task card only after an explicit handoff request and a confirmed,
closed handoff contract. The host renders the canonical `## 任务卡`; AGS core
validates typed fields and does not interpret the source conversation.

Validate with:

```bash
ags govern task validate --task-card <path> --workspace <repo> --format json
```

The card's `Execution mode`, `Execution topology`, `Delegation planning`,
mutation boundary, and commit prohibition are authoritative. Heavy adds an
independent review gate; it does not change writer authority.

For an authorized `fanout-in-card`/`worktree` task, lanes use separate
worktrees and may run concurrently. If the card prohibits commits, every lane
returns an uncommitted diff, untracked-file inventory, ignored-owned-file
inventory, and verification evidence. The main executor alone integrates.

Close through the contract-v2 govern/apply flow only after the delivery report
binds the task-card and LaunchPlan hashes and all required review evidence.
