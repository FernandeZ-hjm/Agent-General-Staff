# Full Installer

The Full Installer profile layers curated skills and runtime setup on top of the
DIY/Core governance framework.

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

## Safety Model

- Dry-run is default.
- Apply creates `.agent-suite-backups/<timestamp>/` in the target project.
- The installer writes only project workflow files.
- Global runtime install is separate from target-project workflow install.
- Optional dependencies are described, not installed silently.
