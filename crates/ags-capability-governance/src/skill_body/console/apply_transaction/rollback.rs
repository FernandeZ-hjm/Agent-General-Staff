use super::*;

pub(in super::super) fn rollback_change(change: &AppliedChange) -> std::io::Result<()> {
    match change {
        AppliedChange::CreatedDir(path) => match std::fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
        AppliedChange::Relink { entry, previous } => {
            remove_host_entry(entry)?;
            if let Some(old) = previous {
                if std::fs::symlink_metadata(old).is_ok() {
                    std::fs::rename(old, entry)?;
                }
            }
            Ok(())
        }
        AppliedChange::Unlink { entry, backup } => {
            if std::fs::symlink_metadata(backup).is_ok() {
                if std::fs::symlink_metadata(entry).is_ok() {
                    remove_host_entry(entry)?;
                }
                std::fs::rename(backup, entry)?;
            }
            Ok(())
        }
    }
}

pub(in super::super) fn rollback_changes(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes.iter().rev() {
        if let Err(e) = rollback_change(change) {
            errors.push(format!("rollback {:?}: {e}", change));
        }
    }
    errors
}

pub(in super::super) fn cleanup_successful_changes(changes: &[AppliedChange]) -> Vec<String> {
    let mut errors = Vec::new();
    for change in changes {
        match change {
            AppliedChange::Relink {
                previous: Some(old),
                ..
            } => {
                if let Err(e) = remove_host_entry(old) {
                    errors.push(format!("cleanup replaced entry {}: {e}", old.display()));
                }
            }
            AppliedChange::Unlink { backup, .. } => {
                if let Err(e) = remove_host_entry(backup) {
                    errors.push(format!(
                        "cleanup unlinked-entry backup {}: {e}",
                        backup.display()
                    ));
                }
            }
            _ => {}
        }
    }
    errors
}
