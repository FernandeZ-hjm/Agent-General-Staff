use super::*;
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_STAGE: AtomicU64 = AtomicU64::new(1);

    let parent = path
        .parent()
        .ok_or_else(|| "private file has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "private file name is not UTF-8".to_string())?;
    let sequence = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
    let stage = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), sequence));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| -> Result<(), String> {
        let mut file = options
            .open(&stage)
            .map_err(|error| format!("cannot stage {}: {error}", stage.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("cannot write {}: {error}", stage.display()))?;
        file.sync_all()
            .map_err(|error| format!("cannot sync {}: {error}", stage.display()))?;
        commit_private_stage(&stage, path)
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&stage);
    }
    write_result
}

/// Prepare every fallible property on the stage path first. The rename is the
/// final operation, so a successful replacement can never be reported as a
/// failed transaction by a later chmod or metadata step.
pub(super) fn commit_private_stage(stage: &Path, path: &Path) -> Result<(), String> {
    set_private_permissions(stage)?;
    std::fs::rename(stage, path).map_err(|error| {
        format!(
            "cannot atomically replace {} from {}: {error}",
            path.display(),
            stage.display()
        )
    })
}

fn set_private_permissions(_path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot chmod {}: {error}", _path.display()))?;
    }
    Ok(())
}
