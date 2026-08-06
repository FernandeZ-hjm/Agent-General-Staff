use super::{MaintenancePlan, MaintenanceReceipt};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn persist_plan(runtime_home: &Path, plan: &MaintenancePlan) -> Result<(), String> {
    let path = object_path(runtime_home, "plans", &plan.plan_hash)?;
    write_json(&path, plan)
}

pub(super) fn load_plan(runtime_home: &Path, plan_hash: &str) -> Result<MaintenancePlan, String> {
    let path = object_path(runtime_home, "plans", plan_hash)?;
    let bytes = fs::read(&path)
        .map_err(|error| format!("cannot read maintenance plan {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid maintenance plan {}: {error}", path.display()))
}

pub(super) fn persist_receipt(
    runtime_home: &Path,
    receipt: &MaintenanceReceipt,
) -> Result<(), String> {
    let path = object_path(runtime_home, "receipts", &receipt.receipt_id)?;
    write_json(&path, receipt)?;
    if receipt.phase == super::MaintenancePhase::Apply {
        let applied = object_path(runtime_home, "applied-plans", &receipt.plan_hash)?;
        write_json(&applied, receipt)?;
    }
    Ok(())
}

pub(super) fn load_apply_receipt(
    runtime_home: &Path,
    plan_hash: &str,
) -> Result<MaintenanceReceipt, String> {
    let path = object_path(runtime_home, "applied-plans", plan_hash)?;
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "maintenance plan has no successful apply receipt {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid apply receipt {}: {error}", path.display()))
}

pub(super) fn load_apply_receipt_optional(
    runtime_home: &Path,
    plan_hash: &str,
) -> Result<Option<MaintenanceReceipt>, String> {
    let path = object_path(runtime_home, "applied-plans", plan_hash)?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read maintenance apply receipt {}: {error}",
                path.display()
            ))
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("invalid apply receipt {}: {error}", path.display()))
}

fn object_path(runtime_home: &Path, kind: &str, id: &str) -> Result<PathBuf, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("invalid maintenance object id `{id}`"));
    }
    Ok(ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join(kind)
        .join(format!("{id}.json")))
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot serialize maintenance object: {error}"))?;
    ags_platform::atomic_write(path, &bytes)
}
