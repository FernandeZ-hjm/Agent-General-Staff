use crate::{verify_receipt_artifacts, Receipt};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStatus {
    pub schema_version: String,
    pub memory_dir: String,
    pub initialized: bool,
    pub archive_count: usize,
    pub authoritative_source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveResult {
    pub schema_version: String,
    pub receipt_id: String,
    pub archive_dir: String,
    pub archived: bool,
    pub idempotent: bool,
}

pub fn status(memory_dir: &Path) -> MemoryStatus {
    let archive_dir = memory_dir.join("task-archive");
    let archive_count = std::fs::read_dir(&archive_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .count()
        })
        .unwrap_or(0);
    MemoryStatus {
        schema_version: "0.3.6-memory-status".to_string(),
        memory_dir: memory_dir.display().to_string(),
        initialized: memory_dir.join("context-capsule.md").is_file()
            && memory_dir.join("task-memory.md").is_file()
            && archive_dir.is_dir(),
        archive_count,
        authoritative_source: "verified 0.3.6-task-receipt archives".to_string(),
    }
}

pub fn init(memory_dir: &Path) -> Result<MemoryStatus, String> {
    std::fs::create_dir_all(memory_dir.join("task-archive"))
        .map_err(|error| format!("cannot initialize memory archive: {error}"))?;
    create_if_missing(
        &memory_dir.join("context-capsule.md"),
        "# Context Capsule\n\nNo verified receipt has been archived yet.\n",
    )?;
    create_if_missing(
        &memory_dir.join("task-memory.md"),
        "# Task Memory\n\n> Non-authoritative derived view. Permission facts live only in the task card, LaunchPlan, delivery report, and verified receipt.\n",
    )?;
    Ok(status(memory_dir))
}

pub fn archive(receipt_path: &Path, memory_dir: &Path) -> Result<ArchiveResult, String> {
    let bytes = std::fs::read(receipt_path)
        .map_err(|error| format!("cannot read receipt `{}`: {error}", receipt_path.display()))?;
    let receipt: Receipt =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid receipt JSON: {error}"))?;
    verify_receipt_artifacts(&receipt)?;

    init(memory_dir)?;
    let archive_dir = memory_dir.join("task-archive").join(&receipt.receipt_id);
    let archived_receipt = archive_dir.join("receipt.json");
    if archived_receipt.is_file() {
        let existing = std::fs::read(&archived_receipt)
            .map_err(|error| format!("cannot read archived receipt: {error}"))?;
        if existing != bytes {
            return Err(format!(
                "archive collision for receipt `{}`",
                receipt.receipt_id
            ));
        }
        return Ok(ArchiveResult {
            schema_version: "0.3.6-memory-archive".to_string(),
            receipt_id: receipt.receipt_id,
            archive_dir: archive_dir.display().to_string(),
            archived: false,
            idempotent: true,
        });
    }

    std::fs::create_dir_all(&archive_dir)
        .map_err(|error| format!("cannot create receipt archive: {error}"))?;
    for (destination, source) in [
        (
            archive_dir.join("task-card.md"),
            receipt
                .task_card_path
                .as_ref()
                .map(PathBuf::from)
                .ok_or_else(|| "receipt has no task_card_path".to_string())?,
        ),
        (
            archive_dir.join("launch-plan.json"),
            PathBuf::from(&receipt.launch_plan_path),
        ),
        (
            archive_dir.join("delivery-report.md"),
            PathBuf::from(&receipt.delivery_report_path),
        ),
    ] {
        let source_bytes = std::fs::read(&source)
            .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
        ags_platform::atomic_write(&destination, &source_bytes)?;
    }
    ags_platform::atomic_write(&archived_receipt, &bytes)?;

    let summary = format!(
        "# Task Memory\n\n> Non-authoritative derived view. Permission facts live only in the archived raw artifacts and verified receipt.\n\n- latest receipt: {}\n- closure status: {}\n- task-card-hash: {}\n- launch-plan-hash: {}\n- delivery-report-hash: {}\n",
        receipt.receipt_id,
        receipt.closure_status,
        receipt.task_card_hash,
        receipt.launch_plan_hash,
        receipt.delivery_report_hash
    );
    ags_platform::atomic_write(&memory_dir.join("task-memory.md"), summary.as_bytes())?;

    Ok(ArchiveResult {
        schema_version: "0.3.6-memory-archive".to_string(),
        receipt_id: receipt.receipt_id,
        archive_dir: archive_dir.display().to_string(),
        archived: true,
        idempotent: false,
    })
}

fn create_if_missing(path: &Path, content: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    ags_platform::atomic_write(path, content.as_bytes())
}

pub fn render<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| format!("{{\"error\":\"{error}\"}}"))
}
