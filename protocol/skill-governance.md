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

The legacy `skill-registry/private-skills.json`, `skill-bodies/`,
`capability-snapshot/`, and pre-contract-v2 suite-to-Host symlinks are not input
to the contract-v2 runtime. Normal readers and setup neither read nor migrate
them, and their presence never creates an installed or active Skill fact. Setup
refuses an enclosing non-contract-v2 runtime manifest with the structured
`setup_legacy_install_requires_migration` result; otherwise those legacy files
remain inert and untouched. Re-admission is explicit: the operator selects a
source and target Host through `ags govern skill install`, reviews the sealed
plan, and consumes it with `ags apply`. There is no compatibility migration or
implicit catalog rebind.

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

Legacy local-only records are inert and never project a route or compatibility
status. The user must explicitly install the selected source and target Host
through the contract-v2 Operation. There is no alias, old JSON reader, or
implicit adoption path.

## One maintenance transaction

```text
source/ref resolve -> immutable candidate -> bounded audit -> semantic/file diff
-> MaintenancePlan + plan_hash -> explicit risk acknowledgement -> apply
-> immutable body + InstalledSkillRecord -> selected-Host snapshot build
-> exact RouteResolution in a new authenticated session -> MaintenanceReceipt
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
when its exact id exists in the authenticated host session's active table, the immutable body
hash matches, the Host thin index points to that body, and the sealed snapshot
returns an exact `RouteResolution`. There is no fuzzy or fallback route.

Every normalized Generic Agent HostId has a distinct snapshot. An official
adapter adds probe metadata only. Snapshot replacement is atomic and new
sessions observe the new hash; existing sessions remain bound to their loaded
snapshot until reconnect.

## Public commands

```bash
ags govern capability inventory [--host <id>] --workspace . --format json
ags govern skill install <skill-id> <source-uri> --source-kind local|github|git --target-host <host-id> [--target-host <host-id>...] --workspace . --format json
ags govern skill remove <skill-id> --workspace . --format json
ags govern capability snapshot --host <id> [--replace-all] --workspace . --format json
```

Inventory is read-only. Every mutation returns a sealed `action_ref` and is
performed only by `ags apply` in the same authenticated binding. Third-party
Skill changes are never downloaded or silently applied during unrelated work.
The resolver binds remote sources to an immutable commit and content hash during
planning.
