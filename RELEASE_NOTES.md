# Agent General Staff 2.0 Public Edition

Agent General Staff (AGS) 2.0 is the first public edition of the Rust-native
AGS governance kernel and CLI.

This release turns AGS from a protocol-and-script kit into a full public
governance runtime for multi-agent engineering work. It ships the `ags` CLI,
canonical task-card protocols, execution-policy checks, release-boundary
verification, memory-capsule templates, and public skill-governance workflows.

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
- Public release boundary: the public-full sanitized payload strips EvoMap/GEP
  capability-plugin runtime and the two EvoMap boundary backing resources; the
  AGS↔EvoMap integration itself is unchanged as product form.
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
- MIT license: AGS may be used, copied, modified, distributed, and used
  commercially under the standard MIT terms.

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

AGS 2.0 Public Edition is distributed under the MIT License.
Superpowers-related workflow inspiration and optional skill references are
attributed separately in THIRD_PARTY_NOTICES.md.
