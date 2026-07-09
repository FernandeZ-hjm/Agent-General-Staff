# Project Profile

Project profile is the cache-stable context source for task-card generation.
It lets Codex, Cursor, and Claude Code infer defaults without changing the
canonical task-card skeleton.

## Default Location

Use this file when present:

```text
config/agent-project-profile.yaml
```

If the file is absent, task-card generation must continue with explicit facts
from the current request, repository files, and project workflow docs. Do not
invent project defaults.

## Purpose

The profile provides stable project facts that are reused across tasks:

- project type and primary runtime,
- stable ASCII project slug for local memory paths,
- default executor preferences,
- common verification commands,
- high-risk paths,
- protected data or generated baselines,
- preferred review strictness,
- project-specific stop conditions,
- context memory locations, task archive location, or
  governance docs.

The profile is not a task card and must not become a second task-card template.
It only fills dynamic slots in the fixed task-card skeleton.

## Cache-Stable Use

When generating a task card:

1. Keep task-card headings, field order, and baseline wording unchanged.
2. Put profile-derived facts only in fixed dynamic slots:
   - `项目画像`
   - `背景`
   - `相关路径`
   - `本次任务相关文件`
   - `适用治理文档`
   - `实施要求`
   - `验证`
3. Prefer referencing the profile path over copying long profile content.
4. If profile facts conflict with the user's current request or live repo
   evidence, use current evidence and mention the conflict in `背景` or
   `实施要求`.

## Minimal Schema

```yaml
schema_version: 1
project:
  name: ""
  slug: ""
  type: ""
  primary_languages: []
  primary_runtime: ""

defaults:
  executor: ""
  runtime_adapter: ""
  execution_surface: ""
  # Omitted-field defaults only: use these values only when a generated card does
  # not declare `Permission mode:`. Task level is a risk/review tier — it never
  # downgrades an explicitly declared permission mode.
  permission_mode_by_level:
    light: execute-and-verify
    medium: edit-with-confirmation
    heavy: plan-only
  parallelism: none

verification:
  default_commands: []
  smoke_commands: []
  expensive_commands: []
  evidence_required: []

risk:
  high_risk_paths: []
  protected_paths: []
  destructive_actions_require_confirmation: true
  heavy_triggers: []
  stop_conditions: []

workflow:
  governance_docs: []
  context_memory_capsule: "$HOME/.agents/memory/projects/<project-slug>/context-capsule.md"
  task_memory: "$HOME/.agents/memory/projects/<project-slug>/task-memory.md"
  task_archive: "$HOME/.agents/memory/projects/<project-slug>/task-archive/"
  default_review_policy: ""
  delivery_report: protocol/agent-task-protocol.md

user_preferences:
  interaction_style: ""
  ask_before: []
  do_not_do: []
```

## Governance

- The profile is project-owned, not suite-owned.
- `permission_mode_by_level` is not an execution cap. It fills an omitted field
  during task-card generation only. A Heavy card that explicitly declares
  `execute-and-verify` remains executable and still requires the Heavy Review
  gate; only a Heavy card with omitted or explicit `plan-only` waits for approval
  before mutation.
- Suite bootstrap installs only a template; it must not overwrite a project's
  real profile.
- Profile changes are normal project changes and should be reviewed with the
  same risk level as other workflow changes.
