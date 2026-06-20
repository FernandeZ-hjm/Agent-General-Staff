//! Shared file-plan data used by setup / init / overlay.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct InstallFile {
    pub(crate) path: PathBuf,
    pub(crate) description: String,
    pub(crate) content: String,
    pub(crate) mode: Option<u32>,
}
