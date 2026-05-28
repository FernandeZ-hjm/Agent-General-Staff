# Full Installer

The Full Installer profile layers required curated skills and runtime setup on
top of the DIY/Core governance framework.

## Runtime Install

```bash
bash scripts/bootstrap.sh --dry-run
bash scripts/bootstrap.sh --apply
bash scripts/diff-local.sh
```

## Project Install

```bash
bash scripts/install-suite-to-project.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

After reviewing the planned writes:

```bash
bash scripts/install-suite-to-project.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

## What It Writes

- `AGENTS.md`
- `CLAUDE.md`
- `docs/agent-workflow/`
- `config/agent-project-profile.yaml`

## Third-Party Skills

Third-party upstreams and candidate skills are listed by GitHub author in
[`third-party-skills.md`](third-party-skills.md). The local suite files remain
canonical; upstream repositories are comparison or tool sources, not automatic
overwrite sources.

## Runtime Mechanisms

- Hook normalization is included through `scripts/configure-review-hooks.mjs`.
- Claude Code keeps skill alias sync, local memory start context, and
  `PreToolUse(Bash)` RTK command rewriting.
- Codex keeps skill alias sync and local memory start context.
- CodeGraph MCP is documented as a supported installable MCP server in
  [`mcp-servers.md`](mcp-servers.md).
- `bootstrap.sh` does not install MCP servers silently.

## Safety Model

- Dry-run is default.
- Apply creates `.agent-suite-backups/<timestamp>/` in the target project.
- The installer writes only project workflow files.
- Global runtime install is separate from target-project workflow install.
- Optional third-party skill packs are not bundled by default.
- `skill-packs/optional/` is reserved as an extension point for future packs.
