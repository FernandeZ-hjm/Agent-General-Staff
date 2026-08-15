# Security Policy

## Reporting a Vulnerability

If you find a security vulnerability in AGS, please report it privately:

1. **Preferred:** Use [GitHub Security Advisories](https://github.com/FernandeZ-hjm/Agent-General-Staff/security/advisories/new) to report privately.
2. **Alternative:** Email the maintainer directly (see the GitHub profile for contact information).

**Do not open a public issue for security vulnerabilities.**

## What Qualifies

- Command injection or privilege escalation in `ags` CLI commands
- Path traversal allowing reads/writes outside intended directories
- Task-card validation bypass that allows unauthorized execution
- Supply-chain issues in dependencies (please include the advisory ID)

## Response

- Acknowledgment within 72 hours
- Fix or mitigation plan within 14 days for confirmed vulnerabilities
- Credit in the release notes (unless you prefer to remain anonymous)

## Supported Versions

| Version | Supported |
|---|---|
| 0.4.x | Yes |
| Earlier contracts | No |

## Runtime Executable Trust Boundary

AGS v0.4.20 authenticates each per-workspace daemon handshake and binds every
sealed action reference to the adapter connection, normalized host, canonical
workspace, authenticated workspace session, policy, plan, and payload hashes.
Cross-binding use, replay, restart, or tampering fails closed.

The standalone `ags-mcp` adapter resolves a workspace per request. It never uses
the daemon process cwd, HOME, a recent project, or fuzzy managed-project lookup
as governance identity. `ags apply` and the MCP `ags_apply` tool accept only the
sealed action reference plus a controlled host outcome when the operation kind
requires one; callers cannot resubmit write plans or binding facts.

Runtime executable identity uses complete-content hashing rather than inode,
size, or timestamps. LocalExecution runs only through the bounded execution
policy and must fail closed when write containment cannot be established.

## Local Projection Filesystem Trust Boundary

Projection apply treats a public pathname as a lookup name, never as object
identity. Unknown inodes are not unlinked or removed. If a created directory or
rollback target cannot be proved by a retained descriptor plus device/inode and
expected content where applicable, AGS preserves the residue and returns
`risk-escalated` with `created_directory_residue` when relevant.

Deletion quarantine lives under a retained AGS state-directory descriptor on
the same filesystem. The state directory must be owned by the effective user
and mode `0700`; these checks prevent access by other credentials, but they do
not isolate AGS from another process running with the same credentials. Such
same-credential local processes are inside the host trust boundary. Platforms
provide no portable atomic mkdir-plus-fd or remove-if-expected-inode primitive,
so protection against an indefinitely racing same-credential process is not a
supported security claim. Cross-filesystem or unsupported atomic moves fail
closed before the public source object is moved.
