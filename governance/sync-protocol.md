# Public Sync Protocol

This public repository does not prescribe a private machine topology. Use the
following generic release flow.

## Local Runtime

```bash
bash scripts/bootstrap.sh --dry-run
bash scripts/bootstrap.sh --apply
bash scripts/diff-local.sh
```

## Target Project

```bash
bash scripts/install-suite-to-project.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

Apply only after reviewing the dry-run.

## Public Release

Recommended flow:

1. Build and verify locally.
2. Run `bash scripts/verify.sh`.
3. Run `bash scripts/security-doctor.sh`.
4. Push a normal branch or update the repository main branch according to your
   own release policy.

Forbidden by default:

- force-pushing without explicit human approval
- publishing private project identities or local machine paths
- pushing secrets, tokens, API keys, memory archives, or local receipts
