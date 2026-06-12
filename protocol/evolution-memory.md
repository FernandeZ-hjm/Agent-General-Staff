# Evolution Memory

Evolution memory defines how AGS may use Evolver without letting method recall
overwrite project truth.

## Authority Boundary

Evolver is an advisory layer, not an authority layer.

Project authority remains with AGS protocol, context capsule, task memory, task
archive, task cards, review gates, and verification gates.

### What Evolver May Influence

Evolver output may inform ONLY the solution formation phase:

- suggest alternative approaches or design patterns;
- flag risks or edge cases the planner should consider;
- reference reusable methods from past tasks;
- provide Gene/Capsule patterns as advisory design input.

All Evolver input is advisory. The planner decides whether to adopt, reject, or
adapt any suggestion. Adoption and rejection must both be traceable in the
solution text (see Pre-Solution Recall documentation requirements).

### What Evolver Must Not Decide

Evolver output must not decide or override:

- task level (Light / Medium / Heavy);
- permission mode;
- review gate;
- verification gate;
- protected-path handling;
- release boundary.

If Evolver output suggests a task level, permission mode, or gate change, the
planner must flag it as an Evolver overreach and ignore it. AGS protocol
authority wins without exception.

## Pre-Solution Recall

For non-trivial tasks (Medium, Heavy, or any task involving development,
architecture, refactoring, release, or governance rule changes), the planner
MUST run Evolver/GEP recall after reading project memory and before producing
a plan or task card. Light tasks may skip recall, but the planner must state
the skip reason in the solution text.

### Recall State

Every recall attempt must record its state in the solution text using three
explicit fields:

| Field | Required values |
|---|---|
| `status` | `available` / `unavailable` / `skipped` |
| `search` | `full` / `low_confidence_only` / `none` |
| `fetch` | `success` / `failed` / `not_attempted` |

When `status=unavailable`, the planner must state the reason and confirm that
the solution proceeds without Evolver input.

When `status=skipped`, the planner must explain why (Light task, trivial fix,
user override) and confirm that skipping does not change task classification.

### Recall Documentation

When Evolver recall is available (`status=available`), the solution text must
include a structured recall section:

- **Recall path**: `MCP` / `CLI` / `skill fallback` — the mechanism used.
- **Input signals**: concise task signals sent to recall (non-secret summary).
- **Hit signals**: what was retrieved (Gene / Capsule / EvolutionEvent / method
  pattern / none).
- **Cited Gene / Capsule**: IDs or content hashes when available.
- **Adoption**: what was explicitly adopted from recall output into the solution.
- **Rejection**: what was retrieved but explicitly rejected, with reason.
- **Impact**: how recall output changed the solution (scoped design decision,
  avoided approach, risk flag, none).
- **Confidence / limitations**: low-confidence recall must be flagged; partial
  or failed fetch must be noted.

Allowed inputs:

- stable project context already read from AGS memory;
- concise task signals from the current request;
- non-secret local workspace signals.

Required handling:

- summarize Evolver output as advisory evidence;
- name whether recall came from MCP, CLI, skill fallback, or was unavailable;
- do not present Evolver output as automatically injected context when no MCP
  tool is available.

## Dual-Source Asset Recall

When a reviewed GEP MCP is available, AGS may use it as a dual-source asset
broker for local and EvoMap Hub assets. The default recall posture is:

```text
local user-owned assets + EvoMap Hub assets -> source-tagged advisory candidates
```

Local recall may include explicitly configured GEP assets, local skills, Genes,
Capsules, and method patterns. Hub recall may include EvoMap assets such as
community or reviewed Skills, Genes, Capsules, and method patterns.

Dual-source recall must preserve source and trust metadata. Returned summaries
should include:

- `source`: `local`, `hub`, or `bundled`;
- `type`: Skill, Gene, Capsule, EvolutionEvent, or method pattern;
- `id` or content hash when available;
- `summary`;
- `provenance`;
- `trust`;
- `why_selected`.

Ranking rules:

- AGS project memory, task evidence, and local user-owned assets have priority
  over Hub assets.
- Hub verified, featured, or otherwise reviewed assets may supplement solution
  formation.
- Ordinary community assets are low-trust advisory references.
- If any recalled asset conflicts with AGS protocol, project memory, task cards,
  execution policy, verification, review gates, or release boundaries, AGS wins.

Allowed MCP actions during solution formation:

- inspect GEP protocol metadata;
- list or search local assets;
- list or search Hub assets;
- run advisory evolution recall over concise non-secret task signals.

Mutating GEP actions require explicit human approval or a task-card contract:

- installing local Genes, Capsules, or Skills;
- publishing bundles to EvoMap Hub;
- revoking Hub assets;
- submitting validation reports;
- exporting or writing portable evolution archives.

Agents must not send secrets, full task archives, runner receipts, private
memory capsules, or large private source excerpts to Hub search. When only a
task signal is needed, send a concise non-secret summary.

## Non-Override Rules

Evolver output must not overwrite:

- project memory;
- task cards;
- delivery reports;
- runner receipts;
- verification results;
- review gate decisions;
- AGS execution-policy results.

If Evolver output conflicts with project memory, task evidence, or AGS gates,
AGS authority wins.

## Post-Task Method Capture

After task completion, Evolver may capture reusable method only. It must not
record project facts as durable truth.

### Fact / Method Split

- **Project memory** records factual task result: what was done, what was the
  outcome, what evidence exists. This goes into `task-memory.md`,
  `task-archive/`, and delivery reports.
- **Evolver / GEP** records reusable method: patterns, approaches, techniques
  that can be applied to future tasks. This goes into Genes, Capsules, or
  EvolutionEvents.

Evolver must not write project-truth artifacts (`context-capsule.md`,
`task-memory.md`, `task-archive/`, delivery report, receipt, verification
evidence). Project memory records fact; Evolver records method. If they
conflict, project memory wins.

### Evidence Priority

Method capture must respect evidence priority:

```text
delivery report / receipt / verification result > git diff signal > fallback observation
```

### Observed vs Successful

| Label | Condition | Confidence |
|---|---|---|
| `successful method` | Available authoritative success evidence (delivery report, receipt, or verification result) confirms success | High — can be promoted to reviewed Gene/Capsule |
| `observed method` | Only git diff signal or fallback observation available; no verification evidence exists | Low — must remain `observed` until verification evidence appears |

Rules:

- Project memory records factual task result first.
- Evolver extracts reusable method from delivery report, receipt, or
  verification evidence after those artifacts exist.
- If no delivery report, receipt, or verification result exists, Evolver may
  record only an `observed method` signal. It must NOT record a `successful
  method`.
- Git diff alone must not be treated as task success.
- Absence of error strings must not be treated as verification success.
- Fallback observations must be labeled `observed`, not `success`.
- A method captured as `observed` may be promoted to `successful` only when
  subsequent verification evidence (delivery report, receipt, verification
  result) confirms the outcome.

## Runtime Hook Profiles

Hook activation is governed by `manifests/runtime-profiles.yaml`, not by
ad-hoc settings edits. Each role profile declares which EvoMap hooks are
allowed and which are denied.

### Executor Profile (claude-code-executor)

Stop method capture is the only allowed EvoMap hook for the executor role.
SessionStart, UserPromptSubmit, and PostToolUse are explicitly denied.

The Stop hook reads evidence in priority order:

- delivery report
- receipt
- verification result
- git diff signal
- fallback observation

### Planner Profile

SessionStart, UserPromptSubmit recall, and PostToolUse signal detection
may be enabled for the planner role. They are not enabled by default —
opt-in is required.

All planner recall is advisory only. It must not decide task level,
permission mode, review gate, or verification gate.

## Portable Template Boundary

Runtime profile templates in `manifests/templates/` are portable installation
skeletons for EvoMap integration. They define role policies and hook structures
that can be installed on any machine.

### What Templates MUST NOT Contain

Templates are public-safe artifacts. They must not include:

- real tokens, Bearer tokens, API keys, or node_secret values;
- absolute `$HOME` paths (e.g. `/Users/<name>/`);
- real task archive paths or memory capsule paths;
- machine-specific configuration (proxy URLs with real ports, auth file paths
  pointing to real home directories).

Documentation text that mentions "token" or "secret" in explanatory context is
acceptable — templates are about configuring tokens, not leaking them.

### What Templates Define

- Role policies: executor (Stop-only) vs. planner (advisory opt-in).
- Hook event types, evidence priority, and AGS authority boundary rules.
- Replacement slots (marked `REPLACE:`) for users to fill during installation.
- EvoMap proxy configuration structure (URL, token file pointer, health endpoint).

### Installation Flow

1. Bootstrap copies templates to the target repository.
2. User fills `REPLACE:` slots with machine-specific paths and proxy URL.
3. User configures `auth_token_file` to point to a local token file (mode 0600).
4. Installed `runtime-profiles.yaml` with filled values is **private** — it must
   not enter bootstrap payloads, public releases, or version control.

### Doctor / Verify Checks

`ags doctor` and `ags verify` check that:
- template files exist and parse correctly;
- templates contain no detectable real secrets, absolute `/Users/` paths, or
  real memory/archive paths (smart detection, not naive grep);
- EvoMap proxy health is checked via authenticated `/proxy/status` when
  `~/.evolver/settings.json` is available, with output always sanitized and
  never blocking offline/non-EvoMap machines.

## Gene / Capsule Naming Boundary

Evolver Gene and Capsule names are intentionally close to
`context-capsule.md`, but their semantics are different.

- `context-capsule.md` is an AGS project charter and project-truth capsule.
- Evolver Capsules are reusable method assets.
- Evolver Genes are reusable method or prompt patterns.

Evolver Genes and Capsules must not modify `context-capsule.md` and must not be
used as substitutes for AGS project memory.
