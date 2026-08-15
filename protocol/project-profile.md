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

## Contract v2 schema

The v2 profile stores test commands only as structured `CommandSpec` values.
Free-form strings and the former `default_commands`, `smoke_commands`, and
`expensive_commands` keys are invalid. AGS passes `program` and `argv` directly
to the process API; it never invokes a shell or interpolates a command string.

```yaml
schema_version: ags://schema/contract/v2/project-profile
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
  execution_mode: single-writer
  execution_topology: single
  delegation_planning: no

verification:
  project_tests:
    smoke:
      program: cargo
      argv: [test, -p, example, --lib]
      cwd: .
      env: {}
      timeout_ms: 120000
      allowed_write_paths: [target]
    standard:
      program: cargo
      argv: [test, --workspace]
      cwd: .
      env: {}
      timeout_ms: 600000
      allowed_write_paths: [target]
    full:
      program: cargo
      argv: [test, --workspace, --all-features]
      cwd: .
      env: {}
      timeout_ms: 1200000
      allowed_write_paths: [target]
  evidence_required: []

risk:
  high_risk_paths: []
  protected_paths: []
  destructive_actions_require_confirmation: true
  heavy_triggers: []
  stop_conditions: []

workflow:
  governance_docs: []
  memory_uri: "ags-memory://projects/<project-slug>"
  default_review_policy: ""
  delivery_report: protocol/agent-task-protocol.md

user_preferences:
  interaction_style: ""
  ask_before: []
  do_not_do: []
```

## LocalExecution platform containment

LocalExecution is available only when `local_execution_platform_support()`
reports an audited containment backend. v0.4.20 enables two fail-closed paths:

- Linux `linux-bubblewrap`, when a runtime containment probe succeeds: a
  read-only root, explicit writable binds for existing declared write roots,
  isolated namespaces including a PID namespace with a reaping PID 1, direct
  argv, bounded output, and recursive descendant-tree termination. If
  bubblewrap, its namespace permissions, or
  the process-table probe is unavailable, LocalExecution returns structured
  `sandbox_unavailable` instead of executing without containment.

- macOS `macos-seatbelt`, when the fixed `/usr/bin/sandbox-exec` probe succeeds:
  direct argv, isolated scratch, declared-write allow, protected and
  undeclared-write deny rules, direct `exec`, and `process-fork` denial. The
  dedicated process group therefore contains exactly one killable process;
  multi-process project tools use the separately authorized HostDelegated
  path. A missing or failed Seatbelt probe returns structured
  `sandbox_unavailable` without execution.

Windows LocalExecution is blocked with `sandbox_unavailable` because `std::process`
cannot atomically assign a suspended child to a Job Object before the first
instruction and this release has no audited AppContainer or filesystem-filter
write-containment backend. Policy-approved `HostDelegated` execution is the
safe alternative. AGS never falls back to an unsandboxed local process.

## Governance

- The profile is project-owned, not suite-owned.
- Profile defaults may prefill a compiler input, but the emitted task card must
  explicitly contain all three authority fields. Task level never rewrites
  them. A later change in authority requires a new task card and LaunchPlan.
- Suite bootstrap installs only a template; it must not overwrite a project's
  real profile.
- Every `cwd` and `allowed_write_paths` entry resolves within the canonical
  workspace. Absolute escapes, `..`, and symlink escapes fail closed.
- `ags check` does not consume `project_tests` and always reports
  `project_tests_run=false`. Only `ags test smoke|standard|full` may execute a
  profile command and produce a `TestReceipt`.
- Profile changes are normal project changes and should be reviewed with the
  same risk level as other workflow changes.
