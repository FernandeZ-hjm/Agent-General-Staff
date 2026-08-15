# Agent Suite Protocol

This document summarizes the Agent General Staff public governance contract.
Canonical details live under `protocol/` and are self-contained.

Current product version: **0.4.20**. Product, wire/schema, and historical release
versions are independent.

## Product boundary

AGS governs admission, typed routing, authorization, maintenance transactions,
capability snapshots, verification, receipts, recovery, and project memory
closure. It does not schedule Agents, interpret raw natural language, maintain
a task queue, or negotiate execution authority.

The Host interprets the complete conversation and submits a closed
`HostRouteProposal`. `ags_route_request` is read-only. `ags_apply_action` is the
only effectful MCP tool and consumes a connection-bound one-shot action. A new
preflight invalidates prior bindings and leases.

## Current public interfaces

- `ags setup` installs or repairs AGS-owned runtime and required suite Skills.
- `ags skill recommend|inspect|install|adopt|check|update|rollback|status|verify`
  manages third-party Skill discovery and machine-local lifecycle.
- `ags update check|config|plan|status|apply|verify|recover` manages signed AGS
  release notices and explicit updates.
- `ags init`, `ags doctor`, `ags agents`, `ags capability`, `ags memory`,
  `ags host lifecycle`, and `ags task` expose their typed governance surfaces.
- `ags mcp serve --transport stdio` connects a Host to the canonical-workspace
  daemon.
- `ags verify --scope local|promotion|release` exposes separate, non-duplicated
  verification scopes.

CLI and MCP call the same Rust domain services. The npm CLI and MCP packages
use the same signed platform kernel, cache, version pointers, and state root.

## Maintenance transaction

Every mutation is represented by `MaintenanceIntent`, an immutable
`MaintenancePlan`, the approved `plan_hash`, and a `MaintenanceReceipt`.
The plan records source and target revisions, affected files/registries/
snapshots/Hosts, risk findings, content hashes, and rollback points. Apply
rejects stale source or target state. Activation or verification failure restores
the previous body, local record, snapshot, and Host pointer.

## Capability fact layers

`manifests/third-party-capabilities.yaml` is the canonical discovery/review
catalog. It does not authorize installation. The generated suite and registries
are projections, not machine state.

For third-party Skills:

1. `CatalogEntry` states what AGS reviewed or recommends.
2. machine-local `InstalledSkillRecord` states what the user installed.
3. Host indexes and capability snapshots state activation.
4. update policy (`notify`, `manual`, `pinned`) states when upstream is checked.

Remote content is resolved to an immutable commit before Apply. Unknown license,
scripts, binaries, dependencies, and privilege requests require explicit risk
acknowledgement. Traversal, symlink/special-file escape, protected writes, ID
overwrite, source/hash drift, and transaction bypass are blocked. Third-party
scripts are never executed by AGS.

## Required suite Skill projection

Setup and maintenance project required suite Skill bodies to the five supported
Skill Hosts in one transaction. The same transaction migrates declared upstream
renames, removes retired AGS-owned symlinks, compiles affected snapshots,
switches pointers atomically, and verifies a real route. Private maintainer
policy may additionally bind the authority root to stable; that path is not
hard-coded in the public kernel.

## Task and execution authority

Input beginning with `## 任务卡` is validated before classification. Invalid
cards fail closed and valid cards are not regenerated. New task-card creation
requires an explicit handoff request plus a confirmed handoff contract.

Execution authority comes only from the explicit `Execution mode`, `Execution
topology`, and `Delegation planning` tuple. It may shrink downstream but never
expand. Heavy adds independent review only. Destructive, external-write,
credential, protected-path, and publication actions keep their own stop gates.

## Public-safe boundary

The public release includes the twelve-module Rust workspace, dual npm
entrypoints, protocols, public-safe resources, typed catalogs, generated
command Skills, empty templates, and release workflows. It excludes private or
personal Skill bodies, real memory/archive/receipt data, credentials, machine
paths, Host configuration, and runtime state.

`manifests/public-release-payload.yaml` is the exact payload authority.
`ags release project-public` binds A-owned writes, typed generated outputs,
retired-file deletion, B-owned digests, post-apply verification, and rollback in
one PlanHash.

## Release verification

The exact A candidate and exact B public commit each run one full gate.
Promotion, tag, GitHub Release, and npm reuse a validated content-addressed
VerificationBundle and perform only boundary-specific artifact, signature,
unpack, hash, and startup checks.
