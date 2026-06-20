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
| 2.7.x | Yes |
| < 2.7 | No |
