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
`ags setup`, which writes the normal `~/.ags/v3/install.json` record and machine
lock. Core updates are plan-bound to the current pointer and exact candidate
content; when a runtime is initialized, apply converges and verifies the five
official Skills before activation. Recovery converges the previous profile and
restores the previous pointer. Workspace entry projection remains an explicit
sealed `ags update --workspace <path>` followed by `ags apply`.
