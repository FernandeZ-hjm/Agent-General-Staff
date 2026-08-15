# Agent Governance Suite Release Notes

## Release 0.4.20

0.4.20 is a contract-v2 hard cut. It replaces the distributed lifecycle,
Machine CLI, and MCP surfaces with one typed Operation control plane; no
runtime compatibility translator or deprecated command remains.

- The product CLI contains exactly `setup`, `init`, `agent`, `govern`, `update`,
  `doctor`, `check`, `test`, `apply`, and `schema`. Human and JSON callers share
  the same Operation registry and handler.
- MCP contains exactly `ags_decide` and `ags_apply`. A lightweight stdio router
  resolves each request to a canonical workspace and reuses a distinct
  authenticated daemon session per workspace.
- Bindings are immutable and seal connection, host, workspace, session, policy,
  plan, and request hashes. Replay, tamper, and cross-binding apply fail closed.
- Any normalized HostId can be governed as a Generic Agent over `cli`, `mcp`, or
  `hybrid`; official host adapters contribute probes and hooks, not an allowlist.
- Transaction operations close through apply, verify, receipt, and
  recover/risk terminal states. `check` never runs project tests. LocalExecution
  executes structured argv only where kernel-bound descendant containment is
  provable and otherwise fails closed before spawn.
- Host outcome input is now a single content-addressed receipt reference; the
  loose status/digest/write-set shape is rejected. HostDelegated apply returns
  `awaiting-outcome` until the bound host submits that receipt; AGS then verifies
  its receipt/details schema, content digest, sealed instruction and plan,
  workspace/session binding, and observed write set before issuing a terminal
  receipt or risk outcome.
- `ags-lifecycle` is replaced by the deep `ags-control-plane` Module. `ags-mcp`
  owns stdio/daemon-child transport and `ags-host` owns lifecycle callbacks.
- Product and launcher source versions advance to 0.4.20; schemas use contract
  v2 URIs. Stable/public promotion, installation, tags, releases, and npm
  publication are intentionally outside this source change.
