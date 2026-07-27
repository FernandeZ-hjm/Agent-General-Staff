//! Closed onboarding assessment, planning and application service.

#[cfg(test)]
use ags_capability_governance::third_party_manifest as manifest;
use ags_capability_governance::third_party_manifest::{
    resolve_third_party_manifest, CapabilityKind, ManifestResolution, ThirdPartyCapability,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ONBOARDING_PLAN_SCHEMA_VERSION: &str = "0.3.4-onboarding-plan";
const EMBEDDED_PUBLIC_PROFILE: &str = include_str!("../../../../manifests/onboarding-public.yaml");

mod assess;
mod execute;
mod model;
mod util;

pub use assess::{assess_public, assess_public_with_resolution, AssessContext};
pub use execute::{action_hash, execute_action, find_action};
pub use model::*;

#[cfg(test)]
mod tests;
