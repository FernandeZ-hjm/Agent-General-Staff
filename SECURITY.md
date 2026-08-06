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
| 0.5.x | Yes |
| 0.3.x | Yes |
| < 0.3 | No |

## Runtime Executable Trust Boundary

AGS v0.3.6 hashes the complete running MCP executable before every governed
request and compares it with the startup identity. It deliberately does not use
inode, size, mtime, or ctime as a shortcut because those signals are not
reliable on every supported filesystem. A mismatch closes the gate and requires
the host to reconnect after `ags mcp restart`.

Versions v0.3.4 and v0.3.5 did not provide request-time executable
self-integrity verification. The earlier metadata-based check was removed
because it could miss equal-length replacement on filesystems with coarse or
non-standard timestamp behavior. v0.3.6 restores the control using full-content
hashing rather than filesystem metadata.

历史 `2.x` tag 不属于当前受支持产品线。
