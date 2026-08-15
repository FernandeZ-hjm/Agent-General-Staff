use super::DESCRIPTOR_SEMANTICS_UNAVAILABLE;
use crate::control_plane::{
    AuthenticatedBinding, ControlPlaneError, HostOutcomeReceipt, HostTerminalDelta,
};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct HostPhysicalSeal;

fn unavailable(detail: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError {
        code: DESCRIPTOR_SEMANTICS_UNAVAILABLE,
        detail: detail.into(),
    }
}

pub(crate) fn seal_host_physical_state(
    _expected_write_paths: &[String],
    _binding: &AuthenticatedBinding,
) -> Result<HostPhysicalSeal, ControlPlaneError> {
    Err(unavailable(
        "this target cannot seal host physical state with retained descriptors",
    ))
}

pub(crate) fn host_physical_seal_digest(
    _seal: &HostPhysicalSeal,
) -> Result<String, ControlPlaneError> {
    Err(unavailable(
        "this target cannot digest an unavailable descriptor seal",
    ))
}

pub(crate) fn verify_host_physical_delta(
    _seal: &HostPhysicalSeal,
    _receipt: &HostOutcomeReceipt,
) -> HostTerminalDelta {
    HostTerminalDelta::Risk {
        known_residuals: Vec::new(),
        unexpected: Vec::new(),
        proof_error: unavailable("this target cannot verify a descriptor-bound host delta"),
    }
}

pub(crate) fn descriptor_host_artifact_is_directory(
    _binding: &AuthenticatedBinding,
    path: &Path,
) -> Result<bool, ControlPlaneError> {
    Err(unavailable(format!(
        "cannot prove directory identity for {}",
        path.display()
    )))
}

pub(crate) fn descriptor_read_host_artifact(
    _binding: &AuthenticatedBinding,
    path: &Path,
    _limit: u64,
) -> Result<Option<Vec<u8>>, ControlPlaneError> {
    Err(unavailable(format!(
        "cannot perform a descriptor-stable read for {}",
        path.display()
    )))
}

pub(crate) fn tree_digest(roots: &[PathBuf]) -> Result<String, ControlPlaneError> {
    if roots.is_empty() {
        Ok(ags_platform::sha256(""))
    } else {
        Err(unavailable(
            "this target cannot prove an unchanged filesystem tree",
        ))
    }
}
