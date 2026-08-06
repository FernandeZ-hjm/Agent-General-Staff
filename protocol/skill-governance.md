# Skill Governance Protocol

> AGS separates discovery, installation and activation. A catalog entry is
> never an installation claim, and an installed body is never routable until
> exact Host activation verifies it.

## Canonical layers

- `manifests/suite.yaml`: bundled body membership and content identity.
- `manifests/skills-registry.yaml`: the single routing contract for every
  bundled body, including explicit `host_command`, `skill_target` or
  fail-closed `not-routable` state. The public projector selects the bundled
  entries from this registry and only rewrites their public body paths; it
  never authors a second routing table.
- `manifests/third-party-capabilities.yaml`: reviewed `CatalogEntry` facts.
- `<runtime_home>/stable-capabilities/installed-skills.json`: the machine's
  `InstalledSkillRecord` truth.
- `<runtime_home>/stable-capabilities/bodies/<skill-id>/<content-hash>/`:
  immutable audited bodies.
- `<runtime_home>/stable-capabilities/snapshots/<host>.json`: one sealed active
  snapshot per Host.

The legacy `skill-registry/private-skills.json`, `skill-bodies/` and
`capability-snapshot/` paths are migration inputs only. Normal readers reject
their schema; setup performs a rollback-safe one-way migration and archives
them before the current-schema maintenance transaction begins.

Pre-0.5 suite-to-Host symlinks are also migration inputs, even when the old
runtime never wrote a projection state file. Setup recognizes one only when a
typed catalog id points exactly into the selected suite authority. It seals
the source identity, content hash, target Hosts and rollback facts in the same
runtime MaintenancePlan, removes the obsolete suite ownership, writes or
repairs the InstalledSkillRecord, then builds all Host snapshots once. An
existing catalog-matching body is rebound to its reviewed upstream; a diverged
body is preserved and remains `rebind-required`. Unowned Host entries are
ignored, never removed.

## Source and policy

The catalog is discovery metadata, not an allowlist. Users may select a catalog
id, an arbitrary GitHub HTTPS repository/tree/blob URL, or a local directory.
Every remote candidate is resolved to an immutable commit before its plan is
sealed. The local record retains repository, subdirectory, requested/tracking
ref, resolved commit, body hash, observed license, catalog review status,
target Hosts and one update policy:

- `notify` (default): periodic checks may notify; apply still needs approval.
- `manual`: check only when requested.
- `pinned`: do not track upstream.

Legacy local-only records remain usable but project `rebind-required`; the user
must explicitly reinstall from an upstream identity. There is no alias or old
JSON compatibility layer.

## One maintenance transaction

```text
source/ref resolve -> immutable candidate -> bounded audit -> semantic/file diff
-> MaintenancePlan + plan_hash -> explicit risk acknowledgement -> apply
-> immutable body + InstalledSkillRecord -> five-Host snapshot build
-> exact RouteResolution + preflight -> MaintenanceReceipt
```

Apply uses a cross-process runtime lock, CAS-checks the registry/body identity,
and writes a crash-recovery journal before its first mutation. Any apply or
activation failure restores the previous body, record, thin indexes and Host
snapshots. CLI and MCP call the same Rust `MaintenanceService`; neither owns a
parallel plan or receipt model.

Composite setup keeps that single lock while applying ordered Skill CAS
changes. Child changes defer activation; one shared inventory pass produces
all selected Host snapshots after the batch, so migration count does not multiply
Host scans.

## Safety boundary

AGS hard-blocks path traversal, boundary escape, symlinks, special files,
source/hash drift, size/count limits, Skill id collisions and writes outside
the transaction. Missing or unknown licenses, executable/script content,
external dependencies, suspected secrets, elevated commands and unreviewed or
catalog-divergent commits are acknowledgement-required findings. They do not
become a catalog whitelist. AGS never executes repository-provided install,
update or uninstall scripts.

Public onboarding does not classify or activate Skill bodies. Catalog discovery
and every installed/routable Skill fact flow only through `MaintenanceService`,
so license metadata cannot become a second, permanent routing gate after the
user has acknowledged the sealed risk plan.

## Routing

Installation and activation are independent. A `SkillTarget` resolves only
when its exact id exists in the preflight-bound active table, the immutable body
hash matches, the Host thin index points to that body, and the sealed snapshot
returns an exact `RouteResolution`. There is no fuzzy or fallback route.

Codex, Claude Code, Cursor, OMP and CodeBuddy-Code use distinct snapshots. Any
snapshot replacement invalidates old bindings; callers must reconnect or run
preflight again.

## Public commands

```bash
ags skill recommend
ags skill inspect <catalog-id|github-url|local-path>
ags skill install <catalog-id|github-url>
ags skill adopt <local-path>
ags skill check [skill-id]
ags skill update <skill-id>
ags skill rollback <skill-id>
ags skill status [skill-id]
ags skill verify <skill-id>
```

`status` returns the layered catalog/installation/activation/update projection.
All mutating commands are plan-first and require the exact current plan hash;
third-party Skill updates are never silently applied.
