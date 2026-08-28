# @agent-governance-suite/cli

Run the verified Agent Governance Suite CLI without a Rust toolchain:

~~~bash
npx -y @agent-governance-suite/cli --version
npx -y @agent-governance-suite/cli setup
~~~

This package is a thin entry point over `@agent-governance-suite/launcher`, the
same interface-neutral core used by the MCP package. It downloads and verifies the same
platform-specific Rust artifact, shares ~/.ags/versions/<version>/<triple>/
and ~/.ags/launcher-state/ (or AGS_CACHE_DIR), and forwards
process.argv.slice(2) to ags with shell: false. A fresh install verifies the
pinned Ed25519-signed release index and the exact platform artifact hash.

The signed archive carries the five binaries plus the public contract-v3
`ags-skills/` runtime profile. The launcher supplies that immutable profile to
`ags setup`, which verifies the complete binary inventory before writing the
normal `~/.ags/v3/install.json` record and machine lock.

Runtime versions are owned by the Rust command surface:

~~~bash
ags upgrade check
ags upgrade plan --workspace /path/to/project
ags apply <ACTION_REF> --workspace /path/to/project
ags upgrade verify <ACTION_REF> --workspace /path/to/project
~~~

The npm process does not intercept `update` or implement its own notifier.
`ags update --workspace <path>` remains the separate sealed capability and
project-projection convergence operation.

Native activation keeps a crash-recovery journal and switches `ags` last.
Launcher and native plans bind the install kind, install source, machine root,
cache root, launcher state root (when applicable), and target root; apply
rejects drift before changing the runtime.
