# AGS Runtime Profile Templates

Portable installation skeletons for EvoMap runtime profile, hooks, and proxy
configuration. These templates define the structure and role policies; users
fill in machine-specific values during installation.

## Files

| File | Purpose |
|------|---------|
| `runtime-profiles.template.yaml` | EvoMap role profiles: executor (Stop-only) and planner (advisory opt-in) |
| `hooks/claude-code-executor-stop.template.js` | Stop hook for post-task method capture (Claude Code executor) |
| `hooks/codex-planner-recall.template.json` | Optional pre-solution advisory recall hooks (Codex/Cursor planner) |

## Installation

1. Copy `runtime-profiles.template.yaml` to the target repo's `manifests/` as
   `runtime-profiles.yaml`.
2. Fill `evolver_proxy.proxy_url` from the target machine's
   `~/.evolver/settings.json` `proxy.url` field.
3. Point `evolver_proxy.auth_token_file` to a file containing the proxy Bearer
   token (mode 0600 recommended).
4. Copy hook templates to the target repo's `.claude/hooks/` or Codex hooks
   directory. Replace `REPLACE:` command paths with actual paths.
5. To enable planner pre-solution recall, set
   `profiles.planner.enabled_by_default` to `true` in the installed
   `runtime-profiles.yaml`.

## Public / Private Boundary

These templates are **public-safe**: they contain no real token, node_secret,
API key, absolute home-directory path, real task archive path, or real memory
capsule path. They can safely enter:
- Bootstrap dry-run / apply payloads
- Public-full sanitized release packages
- Cross-machine migration archives

After installation, the **filled** `runtime-profiles.yaml` with machine-specific
values (token paths, proxy URLs, absolute hook paths) is **private** and must
NOT enter public or bootstrap payloads.

## AGS Authority Boundary

EvoMap is an advisory layer. These templates enforce the boundary:
- Executor (Claude Code): Stop-only method capture. SessionStart,
  UserPromptSubmit, PostToolUse are explicitly denied.
- Planner (Codex/Cursor): pre-solution advisory recall only. It must not decide
  task level, permission mode, review gate, verification gate, or release
  boundary.
- AGS protocol, project memory, task cards, and verification gates always win
  over EvoMap output.

## Verification

```bash
# Check templates parse correctly
python3 -c "import yaml; yaml.safe_load(open('manifests/templates/runtime-profiles.template.yaml'))"
node --check manifests/templates/hooks/claude-code-executor-stop.template.js
python3 -c "import json; json.load(open('manifests/templates/hooks/codex-planner-recall.template.json'))"

# Verify no real secrets or paths leaked
grep -E '/Users/|sk-[a-zA-Z0-9]{20,}|Bearer [a-zA-Z0-9_-]{20,}' manifests/templates/ -r \
  && echo "FAIL: real secrets or paths found" || echo "OK"
```
