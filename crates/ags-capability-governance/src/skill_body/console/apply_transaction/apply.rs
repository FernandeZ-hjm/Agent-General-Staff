use super::*;

/// True iff `name` is a single, safe path component: not empty, not `.`/`..`,
/// no separators or NUL, and exactly one normal component. Keeps host-entry
/// writes from escaping the skills directory.
pub(in super::super) fn is_safe_path_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    let mut comps = Path::new(name).components();
    matches!(
        (comps.next(), comps.next()),
        (Some(std::path::Component::Normal(c)), None) if c == std::ffi::OsStr::new(name)
    )
}

/// Lexical containment: `path` is under `root` and contains no `..` escapes.
pub(in super::super) fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Create a symlink `link` → `target` (a directory) on the host's behalf.
/// Cross-platform; errors cleanly (→ apply error) where symlinks are
/// unsupported, rather than writing an unusable entry.
#[cfg(unix)]
pub(in super::super) fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
pub(in super::super) fn make_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}
#[cfg(not(any(unix, windows)))]
pub(in super::super) fn make_symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "thin-index symlink not supported on this platform",
    ))
}

/// Remove a host entry (symlink or real dir). A missing path is success.
pub(in super::super) fn remove_host_entry(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                match std::fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(file_error)
                        if file_error.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        match std::fs::remove_dir(path) {
                            Ok(()) => Ok(()),
                            Err(dir_error)
                                if dir_error.kind() == std::io::ErrorKind::NotADirectory =>
                            {
                                Err(file_error)
                            }
                            Err(dir_error) => Err(dir_error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            #[cfg(not(windows))]
            {
                std::fs::remove_file(path)
            }
        }
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    }
}

/// A scratch sibling path for staging a symlink before the atomic swap.
pub(in super::super) fn staging_path(entry: &Path) -> PathBuf {
    PathBuf::from(format!("{}.ags-tmp", entry.display()))
}

/// Read-only parent validation for preflight. This never creates directories.
pub(in super::super) fn validate_parent_path(parent: &Path) -> std::io::Result<()> {
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if path.is_dir() {
                return Ok(());
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists but is not a directory", path.display()),
            ));
        }
        current = path.parent();
    }
    Ok(())
}

/// Create missing parent directories during execution and record each one so a
/// later batch failure can roll them back. Preflight remains read-only.
pub(in super::super) fn ensure_parent_dirs(
    parent: &Path,
    changes: &mut Vec<AppliedChange>,
) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.exists() {
            if !path.is_dir() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} exists but is not a directory", path.display()),
                ));
            }
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    for dir in missing.iter().rev() {
        std::fs::create_dir(dir)?;
        changes.push(AppliedChange::CreatedDir(dir.clone()));
    }
    Ok(())
}

/// Transactionally install a thin-index symlink at `entry` → `canonical`.
/// Existing entries are moved to a temporary rollback sibling during the batch,
/// then removed after the whole apply succeeds. No `.bak` host clutter is left.
/// On **any** failure before success cleanup, the original entry is restored.
pub(in super::super) fn transactional_relink(
    entry: &Path,
    canonical: &Path,
) -> std::io::Result<(String, AppliedChange)> {
    let tmp = staging_path(entry);
    // 1. Stage the new symlink first. If this fails, nothing has moved.
    let _ = remove_host_entry(&tmp);
    make_symlink(canonical, &tmp)?;
    // 2. Move any existing entry to a temporary rollback path.
    let previous = if std::fs::symlink_metadata(entry).is_ok() {
        let old = next_replaced_path(entry);
        if let Err(e) = std::fs::rename(entry, &old) {
            let _ = remove_host_entry(&tmp);
            return Err(e);
        }
        Some(old)
    } else {
        None
    };
    // 3. Swap the staged link into place. On failure, roll the previous entry back.
    if let Err(e) = std::fs::rename(&tmp, entry) {
        if let Some(old) = &previous {
            let _ = std::fs::rename(old, entry);
        }
        let _ = remove_host_entry(&tmp);
        return Err(e);
    }
    let msg = match &previous {
        Some(_) => format!(
            "relink {} -> {} (old entry replaced; no .bak kept)",
            entry.display(),
            canonical.display()
        ),
        None => format!("relink {} -> {}", entry.display(), canonical.display()),
    };
    Ok((
        msg,
        AppliedChange::Relink {
            entry: entry.to_path_buf(),
            previous,
        },
    ))
}

/// Move an existing thin index to a temporary rollback sibling. Missing entry
/// is a no-op; successful batches remove the rollback sibling before returning.
pub(in super::super) fn transactional_unlink(
    entry: &Path,
) -> std::io::Result<Option<(String, AppliedChange)>> {
    if std::fs::symlink_metadata(entry).is_err() {
        return Ok(None);
    }
    let bak = next_backup_path(entry);
    std::fs::rename(entry, &bak)?;
    Ok(Some((
        format!("unlinked {} (no .bak kept)", entry.display()),
        AppliedChange::Unlink {
            entry: entry.to_path_buf(),
            backup: bak,
        },
    )))
}
/// The single mutation gate. Returns which writes succeeded and which errored.
///
/// When `confirmed` is false it performs **no** filesystem writes. It first
/// PREFLIGHTS every planned write (containment + host skills dir creatable); if
/// any host fails preflight, NOTHING is mutated — a later host's failure can
/// never leave an earlier host half-changed. Each `relink`/`unlink` then runs
/// transactionally (stage → temporary rollback path → atomic swap). The batch also keeps a
/// rollback stack, so a later host failure restores earlier hosts and removes
/// directories created during this apply. Only thin-index ops run; no skill body
/// is copied; no external command is executed.
pub(in super::super) fn guarded_apply(
    confirmed: bool,
    planned: &[PlannedWrite],
    ctx: &ConsoleContext,
) -> ApplyOutcome {
    let mut outcome = ApplyOutcome::default();
    if !confirmed {
        return outcome;
    }
    let mut allowed_roots: Vec<PathBuf> = supported_skill_hosts()
        .iter()
        .filter_map(|h| host_skills_subdir(h).map(|s| ctx.home.join(s)))
        .collect();
    allowed_roots.push(ctx.home.join(".agents/skills"));

    // ── Preflight: validate ALL destinations before mutating ANY ──
    let mut preflight_errors: Vec<String> = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        if !allowed_roots.iter().any(|r| within(path, r)) {
            preflight_errors.push(format!(
                "refused: write target escapes the host skill roots: {}",
                w.path
            ));
            continue;
        }
        match w.op.as_str() {
            "relink" => {
                if w.from.is_none() {
                    preflight_errors.push(format!("relink {}: no canonical target", w.path));
                }
                if let Some(target) = w.from.as_deref() {
                    let target = Path::new(target);
                    let skill_md = target.join("SKILL.md");
                    let expected_name = path.file_name().and_then(|name| name.to_str());
                    let declared_name = std::fs::read_to_string(&skill_md)
                        .ok()
                        .and_then(|text| crate::skill_body::parse_front_matter(&text).0);
                    if std::fs::canonicalize(target).is_err()
                        || expected_name.is_none()
                        || declared_name.as_deref().map(str::trim) != expected_name
                    {
                        preflight_errors.push(format!(
                            "relink {}: canonical target is missing or declares a different skill name",
                            w.path
                        ));
                    }
                }
                if let Some(parent) = path.parent() {
                    if let Err(e) = validate_parent_path(parent) {
                        preflight_errors.push(format!(
                            "relink {}: host skills dir not creatable: {e}",
                            w.path
                        ));
                    }
                } else {
                    preflight_errors.push(format!("relink {}: no parent directory", w.path));
                }
            }
            "unlink" => {}
            "unlink-retired-suite-thin-index" => {
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    preflight_errors.push(format!(
                        "retired thin-index cleanup {}: missing safe skill name",
                        w.path
                    ));
                    continue;
                };
                if !retired_suite_thin_index_is_safe(ctx, path, name) {
                    preflight_errors.push(format!(
                        "retired thin-index cleanup {}: entry is no longer a proven AGS suite symlink",
                        w.path
                    ));
                }
            }
            other => preflight_errors.push(format!("unknown op '{other}' for {}", w.path)),
        }
    }
    if !preflight_errors.is_empty() {
        // Abort with zero mutation so no host is left half-changed.
        outcome.errors = preflight_errors;
        return outcome;
    }

    // ── Execute: each op is transactional; the batch rolls back on first error ──
    let mut changes = Vec::new();
    for w in planned {
        let path = Path::new(&w.path);
        match w.op.as_str() {
            "relink" => {
                let target = w.from.as_ref().expect("preflight guaranteed a target");
                if let Some(parent) = path.parent() {
                    if let Err(e) = ensure_parent_dirs(parent, &mut changes) {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
                match transactional_relink(path, Path::new(target)) {
                    Ok((msg, change)) => {
                        outcome.applied_writes.push(msg);
                        changes.push(change);
                    }
                    Err(e) => {
                        outcome.errors.push(format!("relink {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
            }
            "unlink" => match transactional_unlink(path) {
                Ok(Some((msg, change))) => {
                    outcome.applied_writes.push(msg);
                    changes.push(change);
                }
                Ok(None) => {}
                Err(e) => {
                    outcome.errors.push(format!("unlink {}: {e}", w.path));
                    outcome.errors.extend(rollback_changes(&changes));
                    outcome.applied_writes.clear();
                    return outcome;
                }
            },
            "unlink-retired-suite-thin-index" => {
                let safe = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| retired_suite_thin_index_is_safe(ctx, path, name));
                if !safe {
                    outcome.errors.push(format!(
                        "retired thin-index cleanup {}: safety proof changed before unlink",
                        w.path
                    ));
                    outcome.errors.extend(rollback_changes(&changes));
                    outcome.applied_writes.clear();
                    return outcome;
                }
                match transactional_unlink(path) {
                    Ok(Some((msg, change))) => {
                        outcome.applied_writes.push(msg);
                        changes.push(change);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        outcome
                            .errors
                            .push(format!("retired thin-index cleanup {}: {e}", w.path));
                        outcome.errors.extend(rollback_changes(&changes));
                        outcome.applied_writes.clear();
                        return outcome;
                    }
                }
            }
            _ => {} // unknown ops already rejected in preflight
        }
    }
    let cleanup_errors = cleanup_successful_changes(&changes);
    if !cleanup_errors.is_empty() {
        outcome.errors = cleanup_errors;
        outcome.applied_writes.clear();
        return outcome;
    }
    outcome
}
