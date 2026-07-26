# Agent Governance Suite Release Notes

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
