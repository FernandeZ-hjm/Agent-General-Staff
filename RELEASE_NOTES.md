# Agent General Staff Public Edition Release Notes

Agent General Staff (AGS) is the public Rust-native AGS governance kernel and
CLI.

The 2.0 release turned AGS from a protocol-and-script kit into a full public
governance runtime for multi-agent engineering work. Current releases continue
to ship the `ags` CLI, canonical task-card protocols, execution-policy checks,
release-boundary verification, memory-capsule templates, and public
skill-governance workflows.

The current CLI/package version is `0.3.0`; current release tags use the same
`v0.3.0` form. Older `2.x` headings below are preserved as historical product
release labels and are not rewritten.

## Release 0.3.0

### Host semantic proposals and explicit apply

- Replaced the natural-language `request-router` with typed
  `request-governance`. The host keeps full conversation context and submits a
  closed `HostRouteProposal`; AGS validates phase, authority, exact skill and
  closed machine targets without interpreting raw text.
- `ags_route_request` is strictly read-only and rejects legacy raw requests.
  `ags_apply_action` is the sole effectful MCP tool and consumes a one-shot,
  connection-bound held action by lease/action ID.
- Added deterministic per-host capability catalogs, exact skill/entrypoint
  validation, machine-private adopt/ignore/rollback overlays, auth availability
  gates, and non-sensitive skill outcome/activity records.
- Renamed canonical `TaskExecute` to `TaskPrepareExecution`; `ags run` now stops
  at a validated `LaunchPlan` and reports `HOST_EXECUTION_REQUIRED` instead of
  claiming execution, verification, or completion.
- Added receipt 2.1 governance evidence while preserving 2.0 readers, made typed
  handoff task level explicit, and fixed the full-width-colon compiler panic.
- Public packaging now excludes capability snapshots, overlays, usage ledgers,
  leases, auth state and runtime receipts.

## Release 0.2.8

### One request router, one structured decision

- Replaced overlapping request and skill routing implementations with one
  natural-language `request-router`. MCP `ags_route_request` receives complete
  host conversation context and returns a closed `RequestDecision`.
- `DirectResponse` is exclusive. One `SkillDemand` and one business-level
  `MachineCli` target may coexist; the router never assembles CLI subcommands
  into a workflow.
- Added `skill-resolver`, which maps closed demands against a validated
  `ActiveSkillTable`. Missing skills return unavailable without automatic
  substitution; alternatives remain advisory only.
- MCP invokes machine capabilities through fixed argv on the real `ags` CLI.
  Compiler, Policy, Gate, and Runner consume structured contracts and never
  re-parse natural language.
- Capability snapshots now carry a registry/runtime hash. Stale snapshots fail
  closed and can be refreshed with `ags capability snapshot --write`.
- Task-card compilation requires both an explicit task-card request and a
  confirmed, closed handoff contract. Existing canonical task cards remain
  validate-first inputs.
- Removed compatibility routing aliases and enrollment state, added positive
  and negative routing tests, and isolated MCP tests from machine-local
  snapshots.

## Release 0.2.7

### Unified routing model

- Entry architecture used the previous multi-stage request-routing model before
  the single-decision architecture introduced in 0.2.8.
- Existing canonical task cards are the validate-first exception: input whose
  first non-empty line is `## 任务卡` is validated before request
  classification. A valid card proceeds directly to policy/runner consumption;
  invalid card-shaped input fails closed and never falls through to generation.
- Task-card permission is binary: `plan-only` or `execute-and-verify`. Light and
  Medium default to direct execution; Heavy defaults to `plan-only`, while an
  explicit Heavy `execute-and-verify` card runs directly with its independent
  review gate.
- CLI top-level surface consolidated to the **five-stage pipeline**: setup →
  agents → skill → init → update. `doctor` and `capability` remain available
  but are no longer primary user-facing pipeline stages.

### Memory capsule capture

- `claude-stop-memory-capture.py` publishes capsule capture: the stop hook now
  archives delivery reports and receipts into task-memory and task-archive
  automatically.

### Task-card validator modular refactoring

- `task-card-validator` refactored from a single 5000+ line `lib.rs` into
  focused modules: `parse.rs` (markdown parsing), `validate.rs` (field and
  combination checks), `contradictions.rs` (contradiction detection engine),
  `types.rs` (shared types), and `tests.rs`. No behavioral change — same
  validation rules, cleaner boundaries.

### Capability skill retirement

- The public `ags capability` skill entry was retired from setup; capability
  discovery remained available through the CLI compatibility surface.

### Capability host integrity

- `ags capability verify --host <host> --strict` derives its expected set from
  the AGS source authority recorded during installation, so running the command
  from an empty or newly governed project cannot silently reduce coverage.
- Required parent skills now fail closed when their host entry is missing.
  Bundled router skills also verify internal playbook completeness and reject
  stale playbooks exposed as standalone host skills.
- `ags doctor` reports these third-party host-routing gaps as formal failures,
  and the setup recommendation view recognizes skills exposed through the
  shared `~/.agents/skills` store.

### Cross-platform CI hardening

- Windows and macOS CI stabilized: path separator normalization, LF line
  endings via `.gitattributes`, `#[cfg(unix)]` gates on shell-dependent tests,
  `PATHEXT`-aware command lookup, and temp-path spelling fixes.

### GitHub release automation

- Pushing an explicit `v<workspace-version>` tag now starts a release workflow.
  It verifies that the tag matches the Cargo workspace version, points to a
  commit on `main`, has a matching release-notes section, and passes the full
  Linux/macOS/Windows matrix before creating the GitHub Release.
- The workflow never creates tags and currently publishes the source release
  only; prebuilt platform binaries remain out of scope.

### Task compiler updates

- `task-compiler` gains Windows absolute-path acceptance and tighter test
  assertions for compiled task-card output.

## Release 2.7.0

AGS 2.7.0 is the kernel-architecture release. It consolidates governance logic
into a unified kernel, restructures the CLI entry surface, and switches the
project license from MIT to GPL-3.0-only.

### Kernel architecture

- The `ags-cli` crate is restructured around a `kernel/` subsystem: awareness,
  bootstrap, compliance, gate, hooks, mcp, policy, receipt, rollback, runner,
  sync, task, and verify modules form a gate → policy → runner → receipt →
  rollback closed loop. Previously scattered governance logic now lives behind
  a single kernel entry surface.
- New `agents/` subsystem (govern, host_specs, scan, verify) adds lightweight
  built-in agent dispatch within the CLI.
- CLI routing split into `cli/actions` (user-facing commands) and
  `cli/kernel_actions` (governance commands); `main.rs` is now a thin dispatcher.
- `setup/`, `init/`, and `update/` are independent modules, each with plan →
  apply → verify → rollback stages supporting dry-run and rollback.

### Capability and routing

- Retired the legacy automatic routing aliases. Brainstorm
  demand now routes to `grill-with-docs`, debugging to `diagnosing-bugs`, and
  verification to `verification-before-completion`. The aliases are no longer
  suite-required or auto-triggered.
- Aligned skill names to upstream canonical names (no local aliases, no compat
  rows). Rename map: `diagnose` → `diagnosing-bugs`, `tdd` →
  `test-driven-development`, `code-review` / `caveman-review` → `review`,
  `zoom-out` → `codebase-design`; `caveman-commit` removed (no replacement). The
  Light review gate now names `requesting-code-review`.
- Capability Route ships as a tracked advisory routing crate
  — manifest-driven and advisory-only across the MCP, CLI,
  and skill-governance inventory surfaces.

### Diagnostics

- `suite-doctor` checks rewritten (~1400 lines changed) for alignment with the
  kernel architecture and expanded diagnostic coverage.

### License

- License changed from MIT to GPL-3.0-only. Derivative works that are
  distributed must also be licensed under GPL-3.0-only. Internal use is
  unaffected. See `LICENSE`, `COMMERCIAL.md`, and `NOTICE.md`.

### Other

- Version surface aligned to 2.7.0 across Cargo metadata, the suite manifest, the MCP
  registry and serverInfo example, and suite diagnostics.
- Third-party skill recommendations remain manual-install only; the public edition
  bundles no third-party skill bodies and no private runtime or memory state.

## Release 2.6.2

AGS 2.6.2 refreshes the public runtime to the current core architecture while
keeping the public-full boundary strict:

- Capability Route is manifest-driven and advisory-only across MCP, CLI, and
  skill-governance inventory surfaces.
- Runner and execution-policy flows use the resolver-first `ags run` contract,
  including structured current-task approval handling.
- Public manifests keep third-party skill bodies out of the payload while still
  exposing safe recommendation and route-target metadata.
- Public documentation and release checks remove private runtime names and
  machine-local capability surfaces.

## Release 2.6.0

AGS 2.6.0 is the quiet-governance public release. It keeps the public-full
sanitized boundary while bringing the public runtime up to the 2.6 protocol
surface:

- Advisory-intent no-mutation gate: consultation requests such as "是否需要",
  "你觉得", or "should we" are classified as advisory and block mutation until
  explicit execution authorization is present.
- Quiet foreground status: MCP responses expose `visible_status` summaries while
  retaining full traceable evidence in the structured report.
- The previous advisory phase gate recommended the lightest execution-path form
  that still covered the risk.
- Tencent Agent host recognition: WorkBuddy and CodeBuddy-Code are recognized as
  Tencent Agent host clients with governed-host preflight behavior.
- Verification routing: `ags verify lane` and the shell lane-decision helper
  classify diffs into minimal, standard, full, and release verification profiles.
- Public boundary retained: local runtime assets, local memory, build output, and
  machine-local overlays remain excluded from the public-full payload.

## Release 2.5.1

AGS 2.5.1 is the local-overlay maintenance release. It keeps the 2.5 public
surface while making project onboarding safer for repositories that should not
commit AGS-managed local entry files:

- `ags init` defaults to a local overlay that writes AGS-managed files to
  `.git/info/exclude` through an idempotent managed block.
- `--mode shared|tracked` remains available when a project intentionally wants
  committed AGS entry files.
- `--migrate-tracked-overlay` safely untracks previously committed AGS-owned
  overlay files with `git rm --cached`, without deleting the working copy.
- Task-card template sources are collapsed to the single canonical
  `protocol/task-card-template.md`; per-level fallback templates are removed
  and compact task cards are rejected.

## Release 2.5.0

AGS 2.5.0 hardens the public edition for cross-platform use and supply-chain
safety while preserving the 2.0 governance product surface:

- Supply-chain gate: repo-local `deny.toml` (RustSec advisories; MIT / Apache-2.0
  / Unicode-3.0 licenses; crates.io-only sources), wired fail-closed into
  `scripts/verify.sh` and the CI matrix.
- Cross-platform portability: new std-only, zero-dependency `ags-platform` crate
  (`home_dir` / `temp_root` / `find_in_path` / `is_on_path`, aware of Windows
  `USERPROFILE` and `PATHEXT`); core crates route `$HOME` and command-lookup
  assumptions through it instead of Unix-only `std::env::var("HOME")` / `which`.
- CI matrix: GitHub Actions now runs on `ubuntu-latest`, `macos-latest`, and
  `windows-latest`; Windows and macOS run the Rust-native `ags verify --scope
  local`, Ubuntu additionally runs the Bash gate and `cargo deny`.
- Pre-push verifier: `templates/hooks/pre-push.verify.sh` (repo-local-first,
  fail-closed; opt-in install, never automatic).
- Public release boundary: the public-full sanitized payload strips local runtime
  state, backing private resources, and machine-local overlays.
- Skill governance console: `ags skill` now exposes a management console on top
  of the existing `scan` / `check` / `install` flow — `ags skill inventory`
  (audit on-disk skill assets, optionally writing `governance/skills-inventory.md`),
  `ags skill verify --host <host>` (read-only host-visibility check), and
  `ags skill propose --action adopt|update|remove|uninstall|repair|verify --skill <name>`
  dry-run proposals,
  plus confirmed `ags skill adopt --skill <name> --apply` / `ags skill ignore
  --skill <name> --apply` writes. `scripts/verify.sh` gained smoke coverage
  for the skill console command surface.

### Windows support

Verified on Windows in 2.5.0:

- The Rust-native `ags` CLI core builds, tests, and runs on `windows-latest` in
  CI. The Windows and macOS CI legs run the Rust-native `ags verify --scope
  local`; the Linux leg additionally runs the Bash gate (`scripts/verify.sh`)
  and `cargo deny`.
- `ags-platform` resolves the Windows home directory (`USERPROFILE`, then
  `HOMEDRIVE`+`HOMEPATH`, then `APPDATA`) and performs `PATH` lookups that honor
  `PATHEXT`, so the CLI never depends on a Unix `$HOME` or an external `which`.

Not claimed in 2.5.0:

- The `scripts/*.sh` helpers (`install.sh`, `update.sh`, `verify.sh`,
  `validate.sh`, …) are Bash/Unix paths. They are not promised to run natively
  under Windows PowerShell or `cmd`; run them under Linux, macOS, WSL, or Git
  Bash.
- No pre-built Windows binary ships with this release. Native Windows users
  should build the CLI from source with Cargo (PowerShell: `cargo build
  --release`, then `$env:Path = "$PWD\target\release;$env:Path"`, then
  `.\target\release\ags.exe verify --scope local`) rather than expecting a
  downloadable `ags.exe` artifact.

## Highlights

- Rust governance kernel: task-card validation, execution policy resolution,
  suite diagnostics, protocol drift checking, scoped verification, receipt and
  compliance checks, runner planning, and capability discovery.
- CLI-first workflow: `ags task`, `ags policy`, `ags sync`, `ags doctor`,
  `ags bootstrap`, `ags project`, `ags protocol`, `ags session`, `ags verify`,
  `ags run`, `ags receipt`, `ags compliance`, `ags skill`, `ags capability`,
  `ags init`, and `ags archive`.
- Standing engineering hub lifecycle: ambient preflight, solution formation,
  user confirmation, explicit task-card request, execution contract, routing,
  gate, execution, receipt, and verification.
- Public-full sanitized distribution: includes the public Rust workspace and
  governance framework, while excluding build output, installed third-party
  skills, private memory, private task archives, secrets, and local machine
  state.
- GPL-3.0-only license: AGS may be used, studied, modified, and redistributed
  under the terms of the GNU General Public License v3.0 only. Distributed
  derivative works must also be GPL-3.0-only.

## Rust And CLI Conversion

AGS 2.0 consolidates the core governance surface into a Rust workspace. The
public CLI exposes structured, repeatable commands for validation, policy
resolution, preflight, release checks, memory capture, and audit receipts.

The release keeps third-party skills optional. AGS can recommend development
skills, but it does not install them silently. Confirmed installs are explicit
and auditable.

## Verification

Before release, run:

```bash
cargo fmt --check
RUSTFLAGS="-D warnings" cargo test
cargo build --release
bash scripts/verify.sh
ags verify --scope release
```

## License And Attribution

AGS Public Edition is distributed under the GNU General Public License v3.0
only (GPL-3.0-only). Superpowers-related workflow inspiration and optional
skill references are attributed separately in THIRD_PARTY_NOTICES.md.
