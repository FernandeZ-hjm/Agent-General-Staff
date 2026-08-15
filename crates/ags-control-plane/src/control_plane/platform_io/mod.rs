//! Private platform I/O authority.
//!
//! Unix uses retained descriptors for stable reads and physical-state proofs.
//! Targets without equivalent descriptor semantics fail closed instead of
//! substituting path-based checks that cannot prove the same invariant.

#[cfg(unix)]
pub(crate) mod unix;

#[cfg(not(unix))]
mod non_unix;
#[cfg(not(unix))]
pub(crate) use non_unix::*;

pub(crate) const DESCRIPTOR_SEMANTICS_UNAVAILABLE: &str =
    "platform_descriptor_semantics_unavailable";
