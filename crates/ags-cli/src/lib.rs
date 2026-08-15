//! Contract-v2 AGS command adapter.
//!
//! Parsing and rendering live here; domain assessment, planning, apply,
//! verification, and receipts belong to `ags-control-plane`.

mod adapter;
pub mod cli;

pub use adapter::{read_typed_json, render};
pub use cli::{Cli, Invocation, OutputFormat, ParsedInvocation};

use ags_control_plane::{
    ApplyResult, Decision, HostOutcomeInput, OpenedSession, OperationRequest, OperationState,
    ReceiptStatus,
};
use ags_session::{
    dispatch_workspace_control, WorkspaceContext, WorkspaceControlRequest,
    WorkspaceControlResponse, WorkspaceResolver,
};
use std::path::PathBuf;

type ControlResponse = WorkspaceControlResponse<OpenedSession, Decision, ApplyResult>;

/// Execute one parsed product invocation through the authenticated workspace
/// service. Both the binary entrypoint and adapter conformance tests use this
/// seam, so tests cannot bypass the real CLI routing path.
pub fn execute(parsed: ParsedInvocation, adapter_cwd: PathBuf) -> Result<(String, bool), String> {
    if let Invocation::Release(release) = &parsed.invocation {
        return execute_release(release, parsed.format);
    }
    let workspace = WorkspaceResolver
        .resolve(&WorkspaceContext {
            workspace: parsed.workspace,
            mcp_roots: Vec::new(),
            adapter_cwd,
        })
        .map_err(|error| error.to_string())?;
    let response: ControlResponse = match parsed.invocation {
        Invocation::Decide(operation) => dispatch_workspace_control(
            &workspace,
            &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Decide { operation },
        )?,
        Invocation::Release(_) => {
            unreachable!("release handled before workspace resolution")
        }
        Invocation::Apply {
            action_ref,
            outcome,
        } => {
            let outcome = outcome
                .as_deref()
                .map(read_typed_json::<HostOutcomeInput>)
                .transpose()?;
            dispatch_workspace_control(
                &workspace,
                &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Apply {
                    action_ref,
                    outcome,
                },
            )?
        }
    };
    match response {
        WorkspaceControlResponse::Opened(value) => {
            render(&value, parsed.format).map(|output| (output, true))
        }
        WorkspaceControlResponse::Decided(value) => {
            let succeeded = value.receipt.as_ref().is_none_or(|receipt| {
                !matches!(
                    receipt.status,
                    ReceiptStatus::Failed | ReceiptStatus::RiskEscalated
                )
            });
            render(&value, parsed.format).map(|output| (output, succeeded))
        }
        WorkspaceControlResponse::Applied(value) => {
            let succeeded = !matches!(
                value.state,
                OperationState::Blocked | OperationState::RiskEscalated
            ) && value.receipt.as_ref().is_none_or(|receipt| {
                !matches!(
                    receipt.status,
                    ReceiptStatus::Failed | ReceiptStatus::RiskEscalated
                )
            });
            render(&value, parsed.format).map(|output| (output, succeeded))
        }
    }
}


/// Ops-only A-to-B public projection (restored 0.4.16 mechanism).
/// Bypasses the workspace-bound Operation registry: the projection spans two
/// checkouts and is driven by the private-public promotion script.
fn execute_release(
    release: &crate::cli::ReleaseInvocation,
    format: crate::cli::OutputFormat,
) -> Result<(String, bool), String> {
    use crate::cli::OutputFormat;
    if release.project_public.source.as_os_str().is_empty() {
        let stage = &release.stage_runtime;
        // The workflow passes the check-report as --plan; when it is not a
        // release-plan, regenerate the plan from the source checkout so the
        // staged payload stays authoritative.
        let plan_path = if ags_verification::release_package::is_release_plan(&stage.plan) {
            stage.plan.clone()
        } else {
            let (plan, _) = ags_verification::release_package::release_package_plan(
                &stage.source,
                "public-full",
                true,
            );
            let tmp = std::env::temp_dir().join(format!(
                "ags-release-plan-{}.json",
                std::process::id()
            ));
            std::fs::write(&tmp, serde_json::to_vec_pretty(&plan).map_err(|e| {
                format!("release stage-runtime: plan encode failed: {e}")
            })?)
            .map_err(|e| format!("release stage-runtime: plan write failed: {e}"))?;
            tmp
        };
        let result = ags_verification::release_package::stage_release_runtime(
            &plan_path,
            &stage.source,
            &stage.target,
        )
        .map_err(|e| format!("release stage-runtime: {e}"))?;
        let text = format!(
            "Runtime staged\nstaged_files: {}\nsource_root: {}\ntarget_root: {}",
            result.staged_files.len(),
            result.source_root,
            result.target_root,
        );
        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| format!("release stage-runtime: encode failed: {e}"))?;
        return Ok((
            match format {
                OutputFormat::Json => json,
                OutputFormat::Text => text,
            },
            true,
        ));
    }
    let plan = ags_verification::public_source_projection::plan_public_source_projection(
        &release.project_public.source,
        &release.project_public.target,
    );
    if !release.project_public.apply {
        let blocked = !plan.blocking_findings.is_empty();
        let mut text = format!(
            "Public source projection plan\nplan_hash: {}\nshared writes: {}\ngenerated writes: {}\nretired deletes: {}\nblocking: {}\nstatus: {}",
            plan.plan_hash,
            plan.writes.len(),
            plan.capability_projection.generated_files.len(),
            plan.deletes.len(),
            plan.blocking_findings.len(),
            if blocked { "blocked" } else { "ready" },
        );
        if blocked {
            for finding in &plan.blocking_findings {
                text.push_str(&format!("\n  ! {finding}"));
            }
        }
        let json = serde_json::to_string_pretty(&plan)
            .map_err(|e| format!("release plan encode failed: {e}"))?;
        let out = match format {
            OutputFormat::Json => json,
            OutputFormat::Text => text,
        };
        return Ok((out, !blocked));
    }
    let approved = release
        .project_public
        .plan_hash
        .clone()
        .ok_or_else(|| "release project-public: --plan-hash is required with --apply".to_string())?;
    if approved != plan.plan_hash {
        return Err(format!(
            "release project-public: plan hash mismatch (approved {} != current {})",
            approved, plan.plan_hash
        ));
    }
    let receipt = ags_verification::public_source_projection::apply_public_source_projection(
        &release.project_public.source,
        &release.project_public.target,
        &approved,
    )
    .map_err(|e| format!("release project-public: {e}"))?;
    let text = format!(
        "Public source projection applied and verified\nplan_hash: {}\nwritten: {}\ndeleted: {}",
        receipt.plan_hash,
        receipt.written_files.len() + receipt.capability_projection.written_files.len(),
        receipt.deleted_files.len(),
    );
    let json = serde_json::to_string_pretty(&receipt)
        .map_err(|e| format!("release receipt encode failed: {e}"))?;
    Ok((
        match format {
            OutputFormat::Json => json,
            OutputFormat::Text => text,
        },
        true,
    ))
}
