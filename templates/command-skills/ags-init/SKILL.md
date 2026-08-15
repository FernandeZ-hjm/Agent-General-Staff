---
name: ags-init
description: Attach exactly one project with the lightweight contract-v2 projection and preserve user files.
---

# AGS Init

AGS 产品版本：0.4.20

```bash
ags init --workspace . --format json
ags apply <ACTION_REF> --workspace .
ags doctor all --workspace . --format json
```

Use `--migration exact-owned-only` only for a byte-identical AGS-owned projection. Modified or unowned files are preserved.
