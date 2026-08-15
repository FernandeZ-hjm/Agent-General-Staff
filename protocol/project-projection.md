# Lightweight project projection — contract v2

`setup` initializes machine-global AGS state. `init` is the only project
adoption Operation. Both are domain Operations: they produce a sealed plan,
then the user applies that plan once through the shared control plane. Neither
adapter writes directly.

## Default project payload

The default init projection is intentionally small:

- a short AGS-managed entry block in existing host instruction files, or a
  short generated entry file when absent;
- `config/agent-project-profile.yaml` schema v2;
- one machine-local `ags-memory://projects/<slug>` URI;
- `.ags/ownership-v2.json`, containing only each generated path and its exact
  `last_applied_sha256`.

Protocol encyclopedias, copied memory, host-global configuration, credentials,
third-party Skills/MCPs, and release assets are not project defaults.

## Ownership and migration

Every existing target is classified before planning:

| Existing bytes | Sealed owned hash | Disposition |
|---|---|---|
| absent | any | `create` |
| exact hash match | present | `reclaim_exact_owned` |
| present | absent | `preserve_unowned` |
| hash differs | present | `preserve_modified` |

Absence from `.ags/ownership-v2.json` means unowned; the manifest has no
`owner`, `user_owned`, inferred-ownership, or marker fields. Only
`reclaim_exact_owned` may be replaced or retired. Location, filename,
markers, recognizable content, symlink target, or a former generator version
never establishes ownership. `preserve_unowned` and `preserve_modified` require
byte-for-byte preservation and remain visible in the plan and receipt as a
content-addressed conflict. Every emitted `ags-details://sha256/...` URI must
resolve to the corresponding retained conflict artifact; a dangling details
URI is invalid.

The desired projection is a complete set. A previously owned path omitted from
that set is deleted only when current bytes still equal its recorded
`last_applied_sha256`; a missing old path retires its entry; a modified old path
is preserved with its old digest and a conflict. A desired path is created when
absent, updated only from exact-last-applied bytes, recorded as current when its
bytes already equal the new projection, or preserved when unowned/modified.

The migration classifier is read-only. Its apply-capable plan is an opaque,
process-local capability with private fields and no serialization or
deserialization contract. Apply never trusts the stored disposition: it
recomputes ownership from the recorded owned SHA-256 and current bytes, and
rejects any mismatch as plan tampering.

Planning supports a pristine canonical project directory. Missing `.ags` and
profile parent directories are part of the opaque plan rather than a preflight
failure. Apply treats entry files, project profile, exact-owned retirements,
and `.ags/ownership-v2.json` as one recoverable transaction: create parents
first, revalidate every planned snapshot, apply generated file changes, commit
the new ownership manifest last, and roll back earlier effects on any failure.
The receipt closes only after the file set and manifest agree.

On macOS and Linux, apply opens every existing parent component descriptor-relative with
no-follow semantics and binds parent plus target filesystem identity. Create
uses atomic no-replace; exact-owned replacement uses atomic exchange, validates
the exchanged-out object, and only then removes it. A final target symlink,
regular-file replacement, or parent rename/symlink substitution is rejected and
rolled back without following or overwriting the substituted object. Unix
platforms without these conditional rename primitives and non-Unix platforms
remain structurally blocked until an audited equivalent backend exists.

Deletion and rollback quarantine is rooted at the retained
`.ags-projection-state` directory descriptor, not by reopening that path. The
state directory is effective-user-owned, mode `0700`, and on the source
filesystem. `EXDEV`, `ENOTSUP`, or failure to establish those properties blocks
before moving the public source. Mode `0700` excludes other credentials; it is
not isolation from another process with the same credentials, which is inside
the host trust boundary documented by `SECURITY.md`.

No public basename is directly unlinked or removed. A public object is first
moved with no-replace semantics into a private child of the retained state FD,
then its expected identity (and digest for files) is revalidated before the
private entry is consumed. If directory creation, identity inspection, or
rollback proof fails, the name is left as a visible residue and apply returns a
risk-escalated error rather than guessing which object the name denotes.

## Verification

Hermetic fixtures cover absent, exact-owned, unowned, and user-modified bytes.
Tests assert that only exact-owned bytes are reclaimable, disposition tampering
cannot upgrade ownership, and target/parent substitution preserves both user
and outside bytes across a real descriptor-relative apply attempt.
