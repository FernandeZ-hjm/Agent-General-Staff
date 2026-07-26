//! Compile-time contract for the twelve authoritative module facades.
//!
//! This test intentionally imports only crate-root exports. Moving an
//! authority behind a private implementation module must not force adapters
//! to learn that implementation layout.

use std::path::Path;

#[test]
fn authoritative_crate_root_facades_compile() {
    let _: String = ags_platform::sha256_hex(b"facade-contract");
    let _: fn(&Path) -> ags_workspace_facts::ProjectIdentity = ags_workspace_facts::detect_project;
    let _: fn(&str) -> Option<&'static str> = ags_host_integration::recognized_host_display;
    let _: fn(&str) -> ags_task_contract::ParsedIntent = ags_task_contract::parse_intent;
    let _: fn(
        &ags_governance_decision::HostRouteProposal,
    ) -> Result<(), Vec<ags_governance_decision::ProposalError>> =
        ags_governance_decision::validate_proposal;
    let _: fn(&[u8]) -> String = ags_evidence::sha256_hex;
    let _: fn(ags_verification::Scope, &Path) -> ags_verification::VerificationReport =
        ags_verification::run_verify;
    let _: fn(&Path) -> Result<(), String> = ags_mcp::run_workspace_daemon;

    let _ = std::mem::size_of::<ags_session::WorkspaceClientSession<()>>();
    let _ = std::mem::size_of::<ags_session::SessionActionStore<()>>();
    let _ = std::mem::size_of::<ags_lifecycle::OnboardingPlan>();
}
