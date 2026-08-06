# @agent-governance-suite/cli

Run the verified Agent Governance Suite CLI without a Rust toolchain:

~~~bash
npx -y @agent-governance-suite/cli --version
~~~

This package is a thin entry point over `@agent-governance-suite/launcher`, the
same interface-neutral core used by the MCP package. It downloads and verifies the same
platform-specific Rust artifact, shares ~/.ags/versions/<version>/<triple>/
and ~/.ags/launcher-state/ (or AGS_CACHE_DIR), and forwards
process.argv.slice(2) to ags with shell: false. A fresh install verifies the
pinned Ed25519-signed release index and the exact platform artifact hash.

The cache is content-verified and version directories are immutable. Core
updates are plan-bound to the current pointer, exact candidate content, signed
index, and (when initialized) the runtime setup projection. Apply verifies the
candidate, runs the shared setup transaction, and records both receipts.
Recovery explicitly restores the setup transaction and previous pointer.
