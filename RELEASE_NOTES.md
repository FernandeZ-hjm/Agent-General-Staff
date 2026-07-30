# Agent Governance Suite Release Notes

## Release 0.4.0

0.4.0 completes the canonical-workspace lifecycle architecture. The MCP
`protocolVersion` remains `2024-11-05`; this release changes AGS product,
runtime, registration, and freshness verification rather than the MCP wire
generation.

- Claude Code, Codex, Cursor, CodeBuddy-Code, and OMP now share one
  host-neutral lifecycle event contract backed by the canonical workspace
  daemon. Host adapters only translate native hook input/output.
- Lifecycle projections are workspace-owned, bind an absolute canonical target,
  and are recorded in a versioned local manifest. AGS no longer generates
  machine-global lifecycle hooks.
- Migration previews and backs up affected files, installs and verifies every
  workspace adapter, then removes only AGS-owned user-level lifecycle entries.
  User hooks and MCP configuration are preserved.
- Stop is a per-turn guard only. SessionEnd performs verified closure archival;
  OMP no longer closes the session from `agent_settled`.
- The workspace daemon owns host-session state and event-id idempotency while
  connection-bound DecisionLease isolation remains unchanged.
- Doctor adds blocking local conformance checks for current runtime surfaces,
  workspace daemon identity, managed projection, capability snapshots, MCP
  registration, lifecycle versions, legacy commands, duplicate hooks, and
  canonical targets. Runtime health and local conformance are reported
  separately; enabled-host local drift exits 1. Remote latest checks remain
  advisory and offline operation does not block.
- Exact MCP conformance is independent of host inventory summaries: Codex uses
  its native JSON detail command, while Cursor, CodeBuddy-Code, and OMP use
  their documented JSON configuration. Malformed, disabled, or stale
  registrations still fail closed even when a host list reports AGS as merely
  available.
- Claude-compatible clear Stop output omits the optional
  `hookSpecificOutput` field instead of serializing it as JSON `null`.

## Release 0.3.8

0.3.8 restores one semantic invocation plane for independently installed
Skills and host-connected MCP servers. It keeps the 0.3.6 governance and wire
schemas and does not move natural-language interpretation into AGS.

- `HostRouteProposal` now admits an exact `McpTarget` alongside `SkillTarget`
  and closed `MachineCliTarget` values. AGS validates canonical server, optional
  registered tool, and snapshot hash without keyword or similarity fallback.
- Host capability snapshots now compile and seal MCP catalog and active-index
  rows using explicit setup/update-time registration, health, and auth probes.
  Normal preflight, resource reads, routing, and apply remain offline and
  read-only.
- An admitted MCP target returns host-native dispatch metadata with no
  `DecisionLease`; AGS never proxies or invokes the third-party MCP server.
- Setup threads the explicit host home through live capability discovery, while
  hermetic and onboarding paths keep process discovery disabled.
- Rust unit and stdio E2E coverage proves exact MCP/tool resolution, fail-closed
  rejection of unknown tools, and `HOST_EXECUTION_REQUIRED` host dispatch.

## Release 0.3.7

0.3.7 is a process-lifecycle hotfix for the `ags mcp restart` command introduced
in 0.3.6. It does not change the 0.3.6 governance or wire schemas.

- Workspace daemons now detach from the CLI caller session on Unix and from the
  caller process group on Windows. Short-lived shells and host command runners
  can exit without terminating the newly restarted daemon.
- The MCP CLI E2E contract now verifies `restart -> status`, current-binary
  identity, PID continuity, and Unix session isolation.

## Release 0.3.6

0.3.6 is a direct protocol migration to one Rust governance kernel. It does not
retain legacy authority fields, closure schemas, Python implementations, or
hidden compatibility entrypoints.

- Task authority is now the explicit tuple `Execution mode`, `Execution
  topology`, and `Delegation planning`. The retired `Permission mode`,
  `Parallelism`, `Workflow authority`, `limited`, and `execute-and-verify`
  values are rejected rather than translated.
- `0.3.6-launch-plan` deterministically binds the task-card hash, effective
  authority, resolved launch arguments, downgrade reasons, and its own
  content-derived hash.
- Closure schema 1.1 reports actual mode, topology, and delegation use.
  `ags task close <card> <plan> <report> --receipt-out <receipt>` verifies the
  complete chain, enforces downscope-only execution, emits a
  `0.3.6-task-receipt`, and atomically writes the lifecycle closure pointer.
- Memory archives only verified receipts and their three hash-bound source
  artifacts. SessionEnd no longer searches transcripts or messages for task
  cards; a missing closure pointer produces a safe skipped close receipt.
- Codex, Claude Code, Cursor, and OMP share one Rust Host Adapter contract.
  Cursor uses its native lowercase `sessionStart` / `sessionEnd` / `stop`
  command hooks and native `additional_context` / `followup_message` response
  fields; `cursor-agent mcp list` supplies its read-only registration probe
  without depending on the locked macOS login keychain. OMP itself is unchanged;
  its required JavaScript extension now only registers events, invokes
  `ags host lifecycle`, and maps the returned envelope.
- First-party validation, mutation testing, release staging, performance
  measurement, lifecycle, stop guarding, and archive logic are Rust-owned.
  Retired Python and shell implementations are absent from the release payload.
- `ags mcp status` and `ags mcp restart` place process ownership at the
  CLI/lifecycle boundary. The MCP server does not restart itself.
- MCP request-time self-integrity is restored using a complete executable
  content hash for every governed request. Versions 0.3.4 and 0.3.5 intentionally
  lacked this check after the unreliable metadata shortcut was removed.
- The Rust mutation gate requires all nine authority and binding mutations
  (A1-A3, P1, X1-X2, R1-R3) to compile and then be killed by exact semantic
  tests. Compilation or lint failure is not counted as a killed mutation.
- Release runtime staging now fails closed on authority-plan mismatch, path
  traversal, symlinks, and non-empty targets. Rust performance evidence compares
  the candidate with the 0.3.5 stable binary.
- Agent rules explicitly require first-principles reasoning for debugging,
  architecture, and modification decisions. Skills and MCP provide evidence and
  capability but cannot replace host judgment or bypass authorization.

Promotion remains private authority → stable → public. Release is allowed only
after the full Rust gate, zero skipped release checks, native lifecycle E2E,
platform assets, exact-commit CI, and npm package verification.

## Release 0.3.5

0.3.5 is a corrective release for the 0.3.4 slimming pass. It restores
high-value proof without restoring retired runtime complexity.

- The task-card hard gate now has focused semantic contracts for every one of
  its 19 active machine error codes, including authorization, protected-path,
  Heavy review, closure mapping, OMP executor, and skill-tag failures. Two
  exported error constants that never had a production emission path were
  removed instead of being preserved as misleading compatibility surface.
- The release verifier again executes the canonical full task-card fixture
  through the real CLI and proves that the removed compact format is rejected.
  Missing fixtures fail rather than silently skip.
- Project initialization has a permanent regression contract proving that AGS
  `.gitignore` rules remain idempotent when a user changes headings or rule
  order, while missing rules still trigger an append.
- The version gate now covers the workspace release marker and the documented
  executable-trust boundary, preventing release documentation from lagging the
  product version or implying a security control that is not present.
- 0.3.4 deliberately removed request-time MCP executable replacement detection:
  its metadata shortcut was not portable across overlay and coarse-timestamp
  filesystems. 0.3.5 does not restore that unreliable control. Release
  checksums/provenance, controlled installation, filesystem permissions, and
  daemon restart on upgrade remain the executable trust boundary.
- The 0.3.4 static capability model is unchanged: normal preflight, resource
  reads, route, and apply do not scan, refresh, compare, or advance capability
  state. OMP remains a first-class executor and host.
- The hidden section-drift/sync engine and its `full` verification scope are
  removed. Promotion now has one authoritative boundary: the exact, pinned
  public release manifest. Private-to-stable equivalence is established by
  fast-forward commit/tree identity instead of a second Markdown diff system.
- Host integration now has one adapter engine over declared host protocols.
  Codex and Claude Code use their direct CLI protocols; OMP uses native JSONL
  RPC `/mcp list`. Skill verification, agent scan, onboarding, and memory
  lifecycle consume the same platform facts instead of maintaining host string
  branches or borrowing Codex evidence for OMP.
- The shared Agent rules put first-principles reasoning ahead of skill/MCP
  advice during diagnosis, architecture, and modification decisions, while
  preserving authorization boundaries and fail-closed execution gates.
- CLI commands now share one closed `text | json` output seam. Domain text
  renderers remain separate, while JSON serialization failures are reported
  consistently instead of silently becoming empty output.
- The fixed-sample performance gate keeps its 5% median and 10% p95/RSS
  thresholds. Each timed path also declares a small absolute materiality floor,
  preventing microsecond timer noise and macOS process-scheduler jitter from
  producing contradictory pass/fail results.

Promotion remains private authority → stable → public. Public `main` and
exact-commit CI must pass before the annotated `v0.3.5` tag, release assets,
and npm `latest` are published.

## Release 0.3.4

0.3.4 removes runtime capability churn and retires compatibility surfaces that
had no current caller or working implementation.

- Each host has one sealed static capability snapshot. Setup creates the initial
  snapshot; an explicit `ags capability snapshot --write --host <host>` refreshes
  it during an upstream update. Normal preflight, resource reads, route, and
  apply neither scan the machine nor compare live registries.
- Workspace capability bundles, bundle epochs, user overlays, source registries,
  adoption/ignore logs, usage ledgers, runtime sync/dedupe, persistent backups,
  quarantine, and plan-only rollback data are removed.
- `ags-setup`, `ags-init`, `ags-doctor`, and `ags-agents` remain direct host
  command skills and cannot be submitted as MCP `SkillTarget`s. OMP remains a
  first-class host and task-card executor.
- The current CLI contract is authoritative. Retired aliases, dynamic skill
  commands, the periodic network update notifier, the shell `run-task-card.sh`
  wrapper, old task-card fixtures, and historical projection documents are
  deleted instead of kept as hidden compatibility paths.
- Action receipts record planned/applied writes and verification evidence only.
  Multi-file initialization still restores earlier writes immediately if the
  same apply fails, preventing a half-written project without creating backup
  files or a later rollback interface.
- The Rust implementation is reduced from 81,270 to about 49,600 lines and the
  Rust test inventory from 1,058 to 313 focused tests. Duplicate permutations,
  captured legacy CLI contracts, retired-feature tests, and self-referential
  projection tests are removed; policy, lease isolation, static snapshot,
  routing semantics, task validation, release boundaries, and host E2E remain.

This is a current-contract release rather than a compatibility-preserving patch.
Promotion still follows the private authority → stable → public order, with the
v0.3.3 stable binary retained only as the read-only E2E/performance comparison
baseline during release verification.

## Release 0.3.3

0.3.3 is a compatibility-preserving patch release for deterministic capability
routing and native OMP task execution.

- Each workspace daemon now loads one sealed capability snapshot per host and
  reuses it unchanged across preflight, resource reads, route, and apply.
  Request handling no longer rebuilds capabilities, compares live directories,
  or advances a bundle epoch.
- Apply validates the registry and snapshot hashes already sealed into the
  daemon catalog. Editing or replacing the on-disk registry after route cannot
  invalidate an otherwise valid lease; explicit setup/update plus daemon
  restart remains the only snapshot refresh path.
- Capability cards now distinguish `skill_target`, `host_command`, and
  `not_routable`. AGS command skills such as `ags-setup` carry a frozen direct
  invocation hint, stay outside `ActiveSkillTable`, and return
  `skill_target_kind_mismatch` if submitted to MCP as `SkillTarget`.
- `Executor: OMP` is accepted as a first-class task-card executor and maps to
  the native `omp` runtime adapter across compiler, validator, policy, runner,
  protocol, and regression tests.
- Native registration evidence continues to pass for Codex, Claude Code, and
  OMP. Cursor remains covered by hermetic adapter/daemon tests with the existing
  explicit native-CLI keychain waiver.
- The captured v0.3.0 human and Machine CLI contracts remain unchanged.

Release order remains fixed: public `main` and exact-commit CI first; then the
annotated `v0.3.3` tag and five-platform GitHub Release assets with checksums
and provenance; npm `latest` is published only after those assets are verified.

## Release 0.3.2

0.3.2 completes the package-boundary migration started in 0.3.1 without
changing the captured v0.3.0 human or Machine CLI contracts.

- The runtime workspace now exposes exactly twelve authoritative packages:
  platform, workspace facts, host integration, capability governance, task
  contract, governance decision, session, evidence, verification, lifecycle,
  CLI, and MCP.
- The nine former support packages have moved behind those owners. Their
  package manifests and dependency edges are retired so validation, policy,
  snapshot, lease, delivery, and verification logic cannot retain a second
  authority.
- Workspace-service E2E continues to cover Codex, Claude Code, Cursor, and OMP
  client identities, session/lease isolation, reconnect persistence, snapshot
  rebind, cross-project isolation, idle recycle, and stop-before-restart
  upgrades.
- Native registration probes passed for Codex, Claude Code, and OMP. The Cursor
  adapter remains covered by the hermetic process E2E, but its v0.3.2 native
  CLI probe was explicitly waived by the operator because the macOS login
  keychain was locked; this release does not claim a native Cursor pass.
- Performance evidence compares the current workspace-service paths against the
  v0.3.0 process model with fixed sampling and explicit median, p95, and RSS
  thresholds.
- Chinese and English README surfaces now state the same control-plane
  positioning, support matrix, limitations, daemon transparency, GPL-3.0-only
  license, and release order.
- The private-to-public guard validates source/module topology, documentation,
  E2E/performance evidence, current version surfaces, and retired-authority
  absence in addition to the public manifest and redaction checks.

Release order remains fixed: public `main` and exact-commit CI first; then the
annotated `v0.3.2` tag and five-platform GitHub Release assets with checksums
and provenance; npm `latest` is published only after those assets are verified.

## Release 0.3.1

0.3.1 keeps the v0.3.0 human and machine command contracts while deepening the
implementation into explicit platform, workspace-facts, host-integration,
capability-governance, task-contract, governance-decision, session, evidence,
verification, lifecycle, CLI, and MCP boundaries.

- The stdio process is a thin `connect-or-start` proxy to one daemon keyed only
  by canonical workspace path.
- Hosts share one atomic workspace capability bundle while every conversation
  retains an independent session, preflight binding, and one-shot
  DecisionLease.
- Snapshot refresh can rebind an existing session immediately; stale hashes
  invalidate the old binding without misclassifying the newly published
  snapshot.
- Apply input shape is validated before the lease consumption point; actions
  remain fail-closed after entering the governed effect boundary.
- Task-card validation no longer treats ordinary prose containing `workflow`
  as a parallelism declaration.
- The former capability, discovery, MCP-tool, and skill-console monolith files
  are split by read model, probe, decision, mutation, transaction, rollback,
  publication, and rendering responsibilities.
- A captured 36-node v0.3.0 Clap help manifest protects every visible human
  command, subcommand, option, enum, default, and help description.
- Product version and GPL-3.0-only license surfaces are checked separately from
  historical release labels and compatibility-preserved wire/schema versions.

Release order remains fixed: public `main` and CI first; then the exact
annotated `v0.3.1` tag and five-platform GitHub Release assets with checksums
and provenance; npm `latest` is published only after those assets are verified.

## Release 0.3.0

0.3.0 establishes the public AGS host-adapter and delivery-closure baseline:

- preflight-bound `ags://capabilities/current-host` routing with exact typed
  proposals and a single lease-bound effectful apply tool;
- unified onboarding for skills, CLIs, MCP servers, and hooks from a validated,
  hash-frozen GitHub capability manifest with offline fallback provenance;
- availability-aware third-party routing so catalog presence is not confused
  with host visibility, authentication, or runtime health;
- native project-memory continuity for Claude Code (`SessionStart`/`Stop`),
  Codex (`SessionStart`/`SessionEnd`), and OMP
  (`session_start`/`agent_settled`/`session_shutdown`), with per-host close
  receipts and no capture from ordinary card-less conversations;
- explicit `ags agents govern --agent <host> --apply` writes for AGS-owned
  memory adapters while external MCP registration remains advice-only;
- explicit task-card handoff gates and deterministic delivery-report closure
  across goals, acceptance criteria, verification, review, and unresolved IDs;
- concise, non-cyclic `AGENTS.md`/`CLAUDE.md` startup maps backed by canonical
  protocol documents instead of duplicated always-on manuals;
- setup-installed shared global rule modules under `$HOME/.agents/rules`, so
  concise host entrypoints remain complete on a clean machine;
- prebuilt macOS, Linux, and Windows release assets plus an integrity-checking
  npm MCP launcher.
- one canonical-path workspace daemon shared by Codex, Claude Code, Cursor, and
  OMP clients, with thin stdio forwarding, session-isolated preflight/leases,
  atomic workspace capability state, idle recycling, and stop-before-restart
  executable upgrades;
- real-host MCP E2E coverage for same-workspace sharing, cross-project
  isolation, reconnects, foreign-lease rejection, and daemon upgrades;
- npm trusted publishing through GitHub OIDC with a pinned supported npm CLI
  and a canonical executable `bin` entry.

Release order is fixed: public `main` and CI first; then an explicit maintainer
tag `v0.3.0`; then GitHub assets, checksums, and attestation; only after those
assets exist may the npm publish workflow be dispatched for `0.3.0`.
