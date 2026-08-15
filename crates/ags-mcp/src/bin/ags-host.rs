use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use ags_control_plane::{
    ApplyResult, ContentAddressedArtifactRef, Decision, DetailsChunk, DetailsReadRequest,
    HostArtifactState, HostEvidenceKind, HostExecutionAction, HostExecutionInstruction,
    HostOutcomeEvidence, HostOutcomeInput, HostOutcomeReceipt, HostOutcomeStatus,
    HostReleaseMember, HostWriteArtifact, LifecycleDecision, LifecycleSessionEndRequest,
    LifecycleSessionStartRequest, LifecycleStopGuardRequest, OpenedSession, OperationContext,
    OperationRequest, UpdateReceipt, DETAILS_CHUNK_LIMIT,
    HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION, HOST_OUTCOME_SCHEMA_VERSION,
};
use ags_session::{
    connect_workspace_control_client, dispatch_workspace_control, WorkspaceControlRequest,
    WorkspaceControlResponse, WorkspaceControlSurface,
};

type ControlResponse = WorkspaceControlResponse<OpenedSession, Decision, ApplyResult>;

const MAX_INPUT_BYTES: usize = 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("ags-host: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("execute") {
        return run_execute(&args);
    }
    if args.first().map(String::as_str) != Some("lifecycle") {
        return Err(
            "usage: ags-host lifecycle --event <event> --host <host> --workspace <path> [--input <path|->] | ags-host execute --action-ref <ref> --workspace <path>"
                .to_string(),
        );
    }
    let event = option(&args, "--event")?;
    let host = option(&args, "--host")?;
    let workspace = PathBuf::from(option(&args, "--workspace")?);
    let input = optional(&args, "--input").unwrap_or("-");
    let generic_agent =
        ags_host_integration::GenericAgent::new(host, ags_host_integration::AgentSurface::Cli)?;
    let payload = read_payload(input)?;
    let envelope = ags_control_plane::LifecycleEnvelope::new(
        &workspace,
        generic_agent.host_id.as_str(),
        event,
        payload,
    )?;
    let connection_id = format!("ags-host-{}-{}", std::process::id(), envelope.event_id);
    let context = OperationContext {
        workspace: Some(envelope.canonical_workspace.clone()),
    };
    let operation = match envelope.event.as_str() {
        "session-start" => {
            OperationRequest::HostLifecycleSessionStart(LifecycleSessionStartRequest {
                context: context.clone(),
                host_id: generic_agent.host_id.to_string(),
                host_session_id: envelope.host_session_id,
                event_id: envelope.event_id.clone(),
            })
        }
        "session-end" => OperationRequest::HostLifecycleSessionEnd(LifecycleSessionEndRequest {
            context: context.clone(),
            host_id: generic_agent.host_id.to_string(),
            host_session_id: envelope.host_session_id,
            event_id: envelope.event_id.clone(),
        }),
        "stop-guard" => OperationRequest::HostLifecycleStopGuard(LifecycleStopGuardRequest {
            context: context.clone(),
            host_id: generic_agent.host_id.to_string(),
            host_session_id: envelope.host_session_id,
            event_id: envelope.event_id.clone(),
            last_assistant_message: text_content(
                envelope
                    .payload
                    .get("last_assistant_message")
                    .or_else(|| envelope.payload.get("lastAssistantMessage"))
                    .unwrap_or(&Value::Null),
            ),
        }),
        other => return Err(format!("unsupported lifecycle event `{other}`")),
    };
    let mut client = connect_workspace_control_client(
        &workspace,
        &connection_id,
        generic_agent.host_id.as_str(),
    )?;
    let opened: ControlResponse = client.request(&WorkspaceControlRequest::<
        OperationRequest,
        HostOutcomeInput,
    >::Open {
        surface: WorkspaceControlSurface::Mcp,
    })?;
    if !matches!(opened, WorkspaceControlResponse::Opened(_)) {
        return Err("lifecycle open returned the wrong control-plane response".to_string());
    }
    let response: ControlResponse =
        client.request(
            &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Decide { operation },
        )?;
    let WorkspaceControlResponse::Decided(decision) = response else {
        return Err("lifecycle decide returned the wrong control-plane response".to_string());
    };
    let value = if let Some(result) = decision.result {
        let lifecycle: LifecycleDecision = serde_json::from_value(result)
            .map_err(|error| format!("lifecycle decision decode failed: {error}"))?;
        format_decision(generic_agent.host_id.as_str(), &lifecycle)?
    } else if let Some(action_ref) = decision.action_ref {
        let grant: ControlResponse = client.request(&WorkspaceControlRequest::<
            OperationRequest,
            HostOutcomeInput,
        >::Apply {
            action_ref: action_ref.clone(),
            outcome: None,
        })?;
        let WorkspaceControlResponse::Applied(grant) = grant else {
            return Err("lifecycle apply returned the wrong control-plane response".to_string());
        };
        let terminal = execute_host_grant(
            &mut client,
            &context,
            &action_ref,
            Path::new(&envelope.canonical_workspace),
            &envelope.event_id,
            grant,
        )?;
        serde_json::to_value(terminal)
            .map_err(|error| format!("lifecycle terminal result encode failed: {error}"))?
    } else {
        json!({"state": decision.state, "kind": decision.kind})
    };
    println!(
        "{}",
        serde_json::to_string(&value)
            .map_err(|error| format!("lifecycle response encode failed: {error}"))?
    );
    Ok(())
}

fn run_execute(args: &[String]) -> Result<(), String> {
    let action_ref = option(args, "--action-ref")?.to_string();
    let workspace = PathBuf::from(option(args, "--workspace")?)
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize host workspace: {error}"))?;
    let context = OperationContext {
        workspace: Some(workspace.to_string_lossy().to_string()),
    };
    let grant: ControlResponse = dispatch_workspace_control(
        &workspace,
        &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Apply {
            action_ref: action_ref.clone(),
            outcome: None,
        },
    )?;
    let WorkspaceControlResponse::Applied(grant) = grant else {
        return Err("host execute grant returned the wrong control-plane response".to_string());
    };
    let details = grant
        .details
        .as_ref()
        .ok_or_else(|| "host outcome grant omitted its typed instruction reference".to_string())?;
    let bytes = read_cli_details(
        &workspace,
        &context,
        ContentAddressedArtifactRef {
            uri: details.details_uri.clone(),
            sha256: details.sha256.clone(),
        },
        details.byte_length,
    )?;
    let instruction: HostExecutionInstruction = serde_json::from_slice(&bytes)
        .map_err(|error| format!("host execution instruction decode failed: {error}"))?;
    if instruction.schema_version != HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION
        || instruction.action_ref != action_ref
        || !ags_platform::is_sha256(&instruction.binding_hash)
        || !ags_platform::is_sha256(&instruction.plan_hash)
        || !ags_platform::is_sha256(&instruction.policy_hash)
        || !ags_platform::is_sha256(&instruction.instruction_digest)
    {
        return Err("host execution instruction binding is invalid".to_string());
    }
    let token = grant
        .outcome_token
        .ok_or_else(|| "host outcome grant omitted its token".to_string())?;
    let generation = grant
        .outcome_generation
        .ok_or_else(|| "host outcome grant omitted its generation".to_string())?;
    let event_id = match &instruction.action {
        HostExecutionAction::ArchiveClosures { event_id, .. } => event_id.clone(),
        _ => format!("host-execute-{}", std::process::id()),
    };
    let outcome = execute_instruction(&instruction, &workspace, &event_id, token, generation)?;
    let receipt = persist_host_outcome(&action_ref, &outcome)?;
    let terminal: ControlResponse = dispatch_workspace_control(
        &workspace,
        &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Apply {
            action_ref,
            outcome: Some(HostOutcomeInput { receipt }),
        },
    )?;
    let WorkspaceControlResponse::Applied(terminal) = terminal else {
        return Err("host execute outcome returned the wrong control-plane response".to_string());
    };
    println!(
        "{}",
        serde_json::to_string(&terminal)
            .map_err(|error| format!("host execute result encode failed: {error}"))?
    );
    Ok(())
}

fn read_cli_details(
    workspace: &Path,
    context: &OperationContext,
    artifact: ContentAddressedArtifactRef,
    byte_length: u64,
) -> Result<Vec<u8>, String> {
    let capacity = usize::try_from(byte_length)
        .map_err(|_| "host execution instruction is too large for this platform".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut offset = 0_u64;
    loop {
        let response: ControlResponse = dispatch_workspace_control(
            workspace,
            &WorkspaceControlRequest::<OperationRequest, HostOutcomeInput>::Decide {
                operation: OperationRequest::DetailsRead(DetailsReadRequest {
                    context: context.clone(),
                    artifact: artifact.clone(),
                    offset,
                    max_bytes: DETAILS_CHUNK_LIMIT,
                }),
            },
        )?;
        let WorkspaceControlResponse::Decided(decision) = response else {
            return Err("details read returned the wrong control-plane response".to_string());
        };
        let result = decision
            .result
            .ok_or_else(|| "details read omitted its typed chunk".to_string())?;
        let chunk: DetailsChunk = serde_json::from_value(result)
            .map_err(|error| format!("details chunk decode failed: {error}"))?;
        if chunk.artifact != artifact
            || chunk.offset != offset
            || chunk.byte_length != byte_length
            || chunk.encoding != "hex"
            || chunk.next_offset < offset
        {
            return Err("details chunk binding is invalid".to_string());
        }
        bytes.extend(decode_hex(&chunk.data)?);
        offset = chunk.next_offset;
        if chunk.eof {
            break;
        }
    }
    if bytes.len() as u64 != byte_length || ags_platform::sha256(&bytes) != artifact.sha256 {
        return Err("host execution instruction content digest mismatch".to_string());
    }
    Ok(bytes)
}

fn execute_host_grant(
    client: &mut ags_session::WorkspaceControlClient,
    context: &OperationContext,
    action_ref: &str,
    workspace: &Path,
    event_id: &str,
    grant: ApplyResult,
) -> Result<ApplyResult, String> {
    let details = grant
        .details
        .as_ref()
        .ok_or_else(|| "host outcome grant omitted its typed instruction reference".to_string())?;
    let bytes = read_details(
        client,
        context,
        ContentAddressedArtifactRef {
            uri: details.details_uri.clone(),
            sha256: details.sha256.clone(),
        },
        details.byte_length,
    )?;
    let instruction: HostExecutionInstruction = serde_json::from_slice(&bytes)
        .map_err(|error| format!("host execution instruction decode failed: {error}"))?;
    if instruction.schema_version != HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION
        || instruction.action_ref != action_ref
        || !ags_platform::is_sha256(&instruction.binding_hash)
        || !ags_platform::is_sha256(&instruction.plan_hash)
        || !ags_platform::is_sha256(&instruction.policy_hash)
        || !ags_platform::is_sha256(&instruction.instruction_digest)
    {
        return Err("host execution instruction binding is invalid".to_string());
    }
    let token = grant
        .outcome_token
        .ok_or_else(|| "host outcome grant omitted its token".to_string())?;
    let generation = grant
        .outcome_generation
        .ok_or_else(|| "host outcome grant omitted its generation".to_string())?;
    let outcome = execute_instruction(&instruction, workspace, event_id, token, generation)?;
    let receipt = persist_host_outcome(action_ref, &outcome)?;
    let response: ControlResponse = client.request(&WorkspaceControlRequest::<
        OperationRequest,
        HostOutcomeInput,
    >::Apply {
        action_ref: action_ref.to_string(),
        outcome: Some(HostOutcomeInput { receipt }),
    })?;
    let WorkspaceControlResponse::Applied(terminal) = response else {
        return Err("host outcome apply returned the wrong control-plane response".to_string());
    };
    Ok(terminal)
}

fn read_details(
    client: &mut ags_session::WorkspaceControlClient,
    context: &OperationContext,
    artifact: ContentAddressedArtifactRef,
    byte_length: u64,
) -> Result<Vec<u8>, String> {
    let capacity = usize::try_from(byte_length)
        .map_err(|_| "host execution instruction is too large for this platform".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut offset = 0_u64;
    loop {
        let response: ControlResponse = client.request(&WorkspaceControlRequest::<
            OperationRequest,
            HostOutcomeInput,
        >::Decide {
            operation: OperationRequest::DetailsRead(DetailsReadRequest {
                context: context.clone(),
                artifact: artifact.clone(),
                offset,
                max_bytes: DETAILS_CHUNK_LIMIT,
            }),
        })?;
        let WorkspaceControlResponse::Decided(decision) = response else {
            return Err("details read returned the wrong control-plane response".to_string());
        };
        let result = decision
            .result
            .ok_or_else(|| "details read omitted its typed chunk".to_string())?;
        let chunk: DetailsChunk = serde_json::from_value(result)
            .map_err(|error| format!("details chunk decode failed: {error}"))?;
        if chunk.artifact != artifact
            || chunk.offset != offset
            || chunk.byte_length != byte_length
            || chunk.encoding != "hex"
            || chunk.next_offset < offset
        {
            return Err("details chunk binding is invalid".to_string());
        }
        bytes.extend(decode_hex(&chunk.data)?);
        offset = chunk.next_offset;
        if chunk.eof {
            break;
        }
    }
    if bytes.len() as u64 != byte_length || ags_platform::sha256(&bytes) != artifact.sha256 {
        return Err("host execution instruction content digest mismatch".to_string());
    }
    Ok(bytes)
}

fn execute_instruction(
    instruction: &HostExecutionInstruction,
    workspace: &Path,
    event_id: &str,
    outcome_token: String,
    generation: u64,
) -> Result<HostOutcomeReceipt, String> {
    match &instruction.action {
        HostExecutionAction::ArchiveClosures {
            event_id: instruction_event_id,
            receipt_ids,
            pointer_paths,
            expected_write_paths,
        } => {
            if instruction_event_id != event_id {
                return Err(
                    "lifecycle callback event does not match sealed instruction".to_string()
                );
            }
            execute_archive_closures(
                instruction,
                instruction_event_id,
                outcome_token,
                generation,
                receipt_ids,
                pointer_paths,
                expected_write_paths,
            )
        }
        HostExecutionAction::Command {
            profile,
            program,
            argv,
            cwd,
            env,
            timeout_ms,
            allowed_write_paths,
        } => execute_command(
            instruction,
            workspace,
            outcome_token,
            generation,
            profile,
            program,
            argv,
            cwd,
            env,
            *timeout_ms,
            allowed_write_paths,
        ),
        HostExecutionAction::RuntimeUpdate {
            channel,
            target_version,
            candidate_directory,
            release_directory,
            manifest,
            tree_digest,
            members,
            expected_write_paths,
        } => execute_runtime_update(
            instruction,
            outcome_token,
            generation,
            channel,
            target_version.as_deref(),
            candidate_directory,
            release_directory,
            manifest,
            tree_digest,
            members,
            expected_write_paths,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_command(
    instruction: &HostExecutionInstruction,
    workspace: &Path,
    outcome_token: String,
    generation: u64,
    profile: &ags_control_plane::TestProfile,
    program: &str,
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout_ms: u64,
    allowed_write_paths: &[PathBuf],
) -> Result<HostOutcomeReceipt, String> {
    let spec = ags_verification::CommandSpec {
        program: program.to_string(),
        argv: argv.to_vec(),
        cwd: cwd.to_path_buf(),
        env: env.clone(),
        timeout_ms,
        allowed_write_paths: allowed_write_paths.to_vec(),
    };
    let profile = match profile {
        ags_control_plane::TestProfile::Smoke => ags_verification::TestProfile::Smoke,
        ags_control_plane::TestProfile::Standard => ags_verification::TestProfile::Standard,
        ags_control_plane::TestProfile::Full => ags_verification::TestProfile::Full,
    };
    let receipt = ags_verification::run_host_project_test(workspace, profile, &spec)
        .map_err(|error| format!("host command execution failed closed: {error}"))?;
    let succeeded = receipt.status == ags_verification::TestExecutionStatus::Succeeded;
    let output_digest = receipt.output_digest.clone();
    let observed = receipt
        .observed_write_set
        .iter()
        .map(|path| {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            }
            .display()
            .to_string()
        })
        .collect::<Vec<_>>();
    let evidence_bytes = serde_json::to_vec(&receipt)
        .map_err(|error| format!("test receipt encode failed: {error}"))?;
    host_outcome(
        instruction,
        outcome_token,
        generation,
        if succeeded {
            HostOutcomeStatus::Succeeded
        } else {
            HostOutcomeStatus::Failed
        },
        output_digest,
        observed,
        HostEvidenceKind::TestReceipt,
        evidence_bytes,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn host_outcome(
    instruction: &HostExecutionInstruction,
    outcome_token: String,
    generation: u64,
    status: HostOutcomeStatus,
    output_digest: String,
    observed_write_set: Vec<String>,
    evidence_kind: HostEvidenceKind,
    evidence_bytes: Vec<u8>,
    include_artifacts: bool,
) -> Result<HostOutcomeReceipt, String> {
    let artifacts = if include_artifacts {
        observed_write_set
            .iter()
            .map(|path| host_artifact(Path::new(path)))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let evidence_digest = ags_platform::sha256(&evidence_bytes);
    Ok(HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: instruction.action_ref.clone(),
        binding_hash: instruction.binding_hash.clone(),
        plan_hash: instruction.plan_hash.clone(),
        policy_hash: instruction.policy_hash.clone(),
        instruction_digest: instruction.instruction_digest.clone(),
        outcome_token,
        generation,
        status,
        output_digest,
        observed_write_set,
        artifacts,
        evidence: Some(HostOutcomeEvidence {
            kind: evidence_kind,
            artifact: ContentAddressedArtifactRef {
                uri: format!(
                    "ags://host-evidence/{}",
                    evidence_digest.trim_start_matches("sha256:")
                ),
                sha256: evidence_digest,
            },
            content_hex: encode_hex(&evidence_bytes),
        }),
    })
}

fn host_artifact(path: &Path) -> Result<HostWriteArtifact, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "host write artifact path is unsafe: {}",
            path.display()
        ));
    }
    let state = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if metadata.len() > 16 * 1024 * 1024 {
                return Err(format!(
                    "host write artifact exceeds 16 MiB: {}",
                    path.display()
                ));
            }
            let bytes = std::fs::read(path).map_err(|error| {
                format!("cannot read host artifact {}: {error}", path.display())
            })?;
            HostArtifactState::Present {
                sha256: ags_platform::sha256(bytes),
            }
        }
        Ok(metadata) if metadata.file_type().is_dir() => HostArtifactState::Directory,
        Ok(_) => {
            return Err(format!(
                "host write artifact is not a regular file/directory: {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HostArtifactState::Absent,
        Err(error) => {
            return Err(format!(
                "cannot inspect host write artifact {}: {error}",
                path.display()
            ));
        }
    };
    Ok(HostWriteArtifact {
        path: path.display().to_string(),
        state,
    })
}

#[allow(clippy::too_many_arguments)]
fn execute_runtime_update(
    instruction: &HostExecutionInstruction,
    outcome_token: String,
    generation: u64,
    channel: &str,
    target_version: Option<&str>,
    candidate_directory: &Path,
    release_directory: &Path,
    manifest: &HostReleaseMember,
    tree_digest: &str,
    members: &[HostReleaseMember],
    expected_write_paths: &[PathBuf],
) -> Result<HostOutcomeReceipt, String> {
    let releases_directory = release_directory
        .parent()
        .ok_or_else(|| "release directory has no parent".to_string())?;
    let runtime_home = releases_directory
        .parent()
        .ok_or_else(|| "release directory is outside a runtime root".to_string())?;
    let candidate_root = candidate_directory
        .parent()
        .ok_or_else(|| "candidate directory has no parent".to_string())?;
    if !candidate_directory.is_absolute()
        || !release_directory.is_absolute()
        || candidate_directory == release_directory
        || releases_directory
            .file_name()
            .and_then(|name| name.to_str())
            != Some("releases")
        || candidate_root != runtime_home.join("update-candidates")
    {
        return Err("runtime update source/destination binding is invalid".to_string());
    }
    let release_id = release_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "release id is not UTF-8".to_string())?;
    if manifest.name != "release-manifest.json"
        || members.is_empty()
        || members.iter().any(|member| !safe_member_name(&member.name))
        || members
            .iter()
            .map(|member| &member.name)
            .collect::<BTreeSet<_>>()
            .len()
            != members.len()
    {
        return Err("runtime update member set is invalid".to_string());
    }
    let computed_tree = ags_platform::sha256(
        serde_json::to_vec(members)
            .map_err(|error| format!("release member set encode failed: {error}"))?,
    );
    if computed_tree != tree_digest {
        return Err("runtime update tree digest mismatch".to_string());
    }
    let current_release = runtime_home.join("current-release.json");
    let update_state = runtime_home.join("update-state.json");
    const PAYLOAD_NAMES: [&str; 5] = [
        "ags",
        "ags-mcp",
        "ags-host",
        "ags-launcher.js",
        "release-metadata.json",
    ];
    if members
        .iter()
        .map(|member| member.name.as_str())
        .collect::<BTreeSet<_>>()
        != PAYLOAD_NAMES.into_iter().collect::<BTreeSet<_>>()
    {
        return Err("runtime update payload names do not match contract v2".to_string());
    }
    let mut required_paths = vec![
        releases_directory.to_path_buf(),
        release_directory.to_path_buf(),
    ];
    required_paths.extend(PAYLOAD_NAMES.map(|name| release_directory.join(name)));
    required_paths.extend([
        release_directory.join("release-manifest.json"),
        current_release.clone(),
        update_state.clone(),
    ]);
    if required_paths != expected_write_paths {
        return Err("runtime update expected write set is not canonical".to_string());
    }
    let staged_members = members
        .iter()
        .map(|member| {
            read_release_member(&candidate_directory.join(&member.name), member)
                .map(|bytes| (member, bytes))
        })
        .collect::<Result<Vec<_>, _>>()?;
    #[derive(serde::Serialize)]
    struct ReleaseManifest<'a> {
        schema_version: &'static str,
        version: &'a str,
        tree_digest: &'a str,
        members: &'a [HostReleaseMember],
    }
    let manifest_bytes = serde_json::to_vec_pretty(&ReleaseManifest {
        schema_version: "ags://schema/contract/v2/sealed-release-manifest",
        version: release_id,
        tree_digest,
        members,
    })
    .map_err(|error| format!("release manifest encode failed: {error}"))?;
    if manifest.size != manifest_bytes.len() as u64
        || manifest.sha256 != ags_platform::sha256(&manifest_bytes)
    {
        return Err("sealed release manifest digest mismatch".to_string());
    }
    let current_bytes = serde_json::to_vec_pretty(&json!({
        "schema_version": "ags://schema/contract/v2/current-release",
        "version": release_id,
        "release_directory": release_directory,
        "tree_digest": tree_digest,
    }))
    .map_err(|error| format!("current release encode failed: {error}"))?;
    let output_digest = ags_platform::sha256(
        serde_json::to_vec(&json!({
            "channel": channel,
            "target_version": target_version,
            "version": release_id,
            "tree_digest": tree_digest,
            "completed": true,
        }))
        .map_err(|error| format!("update outcome encode failed: {error}"))?,
    );
    let state_bytes = serde_json::to_vec_pretty(&json!({
        "schema_version": "ags://schema/contract/v2/update-state",
        "channel": channel,
        "target_version": target_version,
        "release_directory": release_directory,
        "tree_digest": tree_digest,
        "output_digest": output_digest,
        "completed": true,
    }))
    .map_err(|error| format!("update state encode failed: {error}"))?;
    let current_preimage = regular_preimage(&current_release)?;
    let temp_stem = instruction.instruction_digest.trim_start_matches("sha256:");
    let current_temp = runtime_home.join(format!(".current-release-{temp_stem}.tmp"));
    let update_temp = runtime_home.join(format!(".update-state-{temp_stem}.tmp"));
    write_regular_new(&current_temp, &current_bytes, 0o644)?;
    if let Err(error) = write_regular_new(&update_temp, &state_bytes, 0o644) {
        let _ = std::fs::remove_file(&current_temp);
        return Err(error);
    }
    let releases_created = create_directory_if_missing(releases_directory)?;
    if let Err(error) = create_new_directory(release_directory) {
        let _ = std::fs::remove_file(&current_temp);
        let _ = std::fs::remove_file(&update_temp);
        if releases_created {
            let _ = std::fs::remove_dir(releases_directory);
        }
        return Err(error);
    }
    let effect = (|| {
        for (member, bytes) in &staged_members {
            write_regular_new(&release_directory.join(&member.name), bytes, member.mode)?;
        }
        write_regular_new(
            &release_directory.join("release-manifest.json"),
            &manifest_bytes,
            manifest.mode,
        )?;
        atomic_replace(&current_temp, &current_release)?;
        if let Err(error) = atomic_replace(&update_temp, &update_state) {
            restore_regular(&current_release, &current_preimage, temp_stem)?;
            return Err(error);
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = effect {
        let _ = std::fs::remove_file(&current_temp);
        let _ = std::fs::remove_file(&update_temp);
        let _ = std::fs::remove_dir_all(release_directory);
        if releases_created {
            let _ = std::fs::remove_dir(releases_directory);
        }
        return Err(error);
    }
    let observed = expected_write_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let update_receipt = UpdateReceipt {
        schema_version: "ags://schema/contract/v2/update-receipt".to_string(),
        channel: channel.to_string(),
        target_version: target_version.map(str::to_string),
        action_ref: instruction.action_ref.clone(),
        binding_hash: instruction.binding_hash.clone(),
        plan_hash: instruction.plan_hash.clone(),
        observed_write_set: observed.clone(),
        release_manifest_sha256: manifest.sha256.clone(),
        release_tree_digest: tree_digest.to_string(),
        output_digest: output_digest.clone(),
        completed: true,
    };
    let evidence = serde_json::to_vec(&update_receipt)
        .map_err(|error| format!("update receipt encode failed: {error}"))?;
    host_outcome(
        instruction,
        outcome_token,
        generation,
        HostOutcomeStatus::Succeeded,
        output_digest,
        observed,
        HostEvidenceKind::UpdateReceipt,
        evidence,
        true,
    )
}

fn safe_member_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 4096
        && Path::new(name).components().count() == 1
        && !matches!(name, "." | "..")
}

fn create_directory_if_missing(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(false),
        Ok(_) => Err(format!(
            "runtime update directory is occupied: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map(|_| true).map_err(|error| {
                format!(
                    "cannot create runtime directory {}: {error}",
                    path.display()
                )
            })
        }
        Err(error) => Err(format!(
            "cannot inspect runtime directory {}: {error}",
            path.display()
        )),
    }
}

fn create_new_directory(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "release directory already exists: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(path)
            .map_err(|error| {
                format!(
                    "cannot create release directory {}: {error}",
                    path.display()
                )
            }),
        Err(error) => Err(format!(
            "cannot inspect release directory {}: {error}",
            path.display()
        )),
    }
}

fn read_release_member(path: &Path, member: &HostReleaseMember) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect release member {}: {error}", path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.len() != member.size
        || member.size > 16 * 1024 * 1024
    {
        return Err(format!(
            "release member metadata mismatch: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != member.mode {
            return Err(format!("release member mode mismatch: {}", path.display()));
        }
    }
    let mut bytes = Vec::with_capacity(member.size as usize);
    open_regular_input(path)?
        .take(member.size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read release member {}: {error}", path.display()))?;
    if bytes.len() as u64 != member.size || ags_platform::sha256(&bytes) != member.sha256 {
        return Err(format!(
            "release member digest mismatch: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

fn write_regular_new(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot write runtime member {}: {error}", path.display()))?;
    use std::io::Write;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("cannot persist runtime member {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|error| {
            format!("cannot set runtime member mode {}: {error}", path.display())
        })?;
    }
    Ok(())
}

#[derive(Clone)]
struct RegularPreimage {
    bytes: Option<Vec<u8>>,
    mode: u32,
}

fn regular_preimage(path: &Path) -> Result<RegularPreimage, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if metadata.len() > 8 * 1024 * 1024 {
                return Err(format!("runtime pointer is too large: {}", path.display()));
            }
            #[cfg(unix)]
            use std::os::unix::fs::PermissionsExt;
            #[cfg(unix)]
            let mode = metadata.permissions().mode() & 0o777;
            #[cfg(not(unix))]
            let mode = 0o644;
            Ok(RegularPreimage {
                bytes: Some(
                    std::fs::read(path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
                ),
                mode,
            })
        }
        Ok(_) => Err(format!(
            "runtime pointer is not regular: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(RegularPreimage {
            bytes: None,
            mode: 0o644,
        }),
        Err(error) => Err(format!(
            "cannot inspect runtime pointer {}: {error}",
            path.display()
        )),
    }
}

fn atomic_replace(staged: &Path, target: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(target).is_ok_and(|metadata| !metadata.file_type().is_file()) {
        return Err(format!(
            "runtime pointer is not regular: {}",
            target.display()
        ));
    }
    std::fs::rename(staged, target).map_err(|error| {
        format!(
            "cannot atomically replace runtime pointer {}: {error}",
            target.display()
        )
    })
}

fn restore_regular(path: &Path, preimage: &RegularPreimage, stem: &str) -> Result<(), String> {
    if let Some(bytes) = &preimage.bytes {
        let staged = path.with_file_name(format!(".restore-{stem}.tmp"));
        let _ = std::fs::remove_file(&staged);
        write_regular_new(&staged, bytes, preimage.mode)?;
        atomic_replace(&staged, path)
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "cannot restore absent pointer {}: {error}",
                path.display()
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_archive_closures(
    instruction: &HostExecutionInstruction,
    event_id: &str,
    outcome_token: String,
    generation: u64,
    receipt_ids: &[String],
    pointer_paths: &[PathBuf],
    expected_write_paths: &[PathBuf],
) -> Result<HostOutcomeReceipt, String> {
    if event_id.is_empty() || pointer_paths != expected_write_paths {
        return Err("archive instruction does not match its lifecycle event/write set".to_string());
    }
    let mut consumed = Vec::new();
    let mut failure = None;
    for path in pointer_paths {
        match remove_regular_pointer(path) {
            Ok(()) => consumed.push(path.display().to_string()),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let completed = failure.is_none() && consumed.len() == pointer_paths.len();
    let output_digest = ags_platform::sha256(
        serde_json::to_vec(&json!({
            "event_id": event_id,
            "receipt_ids": receipt_ids,
            "consumed_pointer_paths": consumed,
            "completed": completed,
            "failure": failure,
        }))
        .map_err(|error| format!("lifecycle outcome digest encode failed: {error}"))?,
    );
    let evidence_bytes = serde_json::to_vec(&json!({
        "schema_version": "ags://schema/contract/v2/lifecycle-host-outcome",
        "event_id": event_id,
        "receipt_ids": receipt_ids,
        "observed_write_set": consumed,
        "consumed_pointer_paths": consumed,
        "output_digest": output_digest,
        "completed": completed,
    }))
    .map_err(|error| format!("lifecycle evidence encode failed: {error}"))?;
    let evidence_digest = ags_platform::sha256(&evidence_bytes);
    let artifacts = consumed
        .iter()
        .map(|path| HostWriteArtifact {
            path: path.clone(),
            state: HostArtifactState::Absent,
        })
        .collect();
    Ok(HostOutcomeReceipt {
        schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
        action_ref: instruction.action_ref.clone(),
        binding_hash: instruction.binding_hash.clone(),
        plan_hash: instruction.plan_hash.clone(),
        policy_hash: instruction.policy_hash.clone(),
        instruction_digest: instruction.instruction_digest.clone(),
        outcome_token,
        generation,
        status: if completed {
            HostOutcomeStatus::Succeeded
        } else {
            HostOutcomeStatus::Failed
        },
        output_digest,
        observed_write_set: consumed,
        artifacts,
        evidence: Some(HostOutcomeEvidence {
            kind: HostEvidenceKind::LifecycleReceipt,
            artifact: ContentAddressedArtifactRef {
                uri: format!(
                    "ags://host-evidence/{}",
                    evidence_digest.trim_start_matches("sha256:")
                ),
                sha256: evidence_digest,
            },
            content_hex: encode_hex(&evidence_bytes),
        }),
    })
}

fn remove_regular_pointer(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "closure pointer path is not safe: {}",
            path.display()
        ));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect closure pointer {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "closure pointer must be a regular file: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path)
        .map_err(|error| format!("cannot consume closure pointer {}: {error}", path.display()))
}

fn persist_host_outcome(
    action_ref: &str,
    outcome: &HostOutcomeReceipt,
) -> Result<ContentAddressedArtifactRef, String> {
    persist_host_outcome_at(&ags_platform::runtime_home(), action_ref, outcome)
}

fn persist_host_outcome_at(
    runtime_home: &Path,
    action_ref: &str,
    outcome: &HostOutcomeReceipt,
) -> Result<ContentAddressedArtifactRef, String> {
    let bytes = serde_json::to_vec(outcome)
        .map_err(|error| format!("host outcome receipt encode failed: {error}"))?;
    let digest = ags_platform::sha256(&bytes);
    let runtime_home = std::fs::canonicalize(runtime_home).map_err(|error| {
        format!(
            "cannot canonicalize host outcome runtime root {}: {error}",
            runtime_home.display()
        )
    })?;
    let directory = runtime_home.join("host-outcomes");
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "cannot create host outcome directory {}: {error}",
            directory.display()
        )
    })?;
    let directory = std::fs::canonicalize(&directory).map_err(|error| {
        format!(
            "cannot canonicalize host outcome directory {}: {error}",
            directory.display()
        )
    })?;
    if !directory.starts_with(&runtime_home) {
        return Err("host outcome directory escapes the runtime root".to_string());
    }
    let name = ags_platform::sha256(format!(
        "{}\n{}\n{}",
        action_ref, outcome.generation, outcome.instruction_digest
    ));
    let path = directory.join(format!("{}.json", name.trim_start_matches("sha256:")));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path).map_err(|error| {
        format!(
            "cannot create host outcome receipt {}: {error}",
            path.display()
        )
    })?;
    use std::io::Write;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "cannot persist host outcome receipt {}: {error}",
                path.display()
            )
        })?;
    Ok(ContentAddressedArtifactRef {
        uri: file_uri(&path),
        sha256: digest,
    })
}

fn file_uri(path: &Path) -> String {
    let value = path.to_string_lossy();
    let mut encoded = String::from("file://");
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'.' | b'_' | b'~') {
            encoded.push(*byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("details chunk has odd-length hex".to_string());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair)
                .map_err(|_| "details chunk contains invalid hex".to_string())?;
            u8::from_str_radix(digits, 16)
                .map_err(|_| "details chunk contains invalid hex".to_string())
        })
        .collect()
}

fn text_content(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

fn option<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    optional(args, name).ok_or_else(|| format!("missing required option `{name}`"))
}

fn optional<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn read_payload(path: &str) -> Result<Value, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        std::io::stdin()
            .take((MAX_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read lifecycle input: {error}"))?;
    } else {
        open_regular_input(Path::new(path))?
            .take((MAX_INPUT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read lifecycle input `{path}`: {error}"))?;
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("lifecycle input exceeds 1 MiB".to_string());
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        Ok(json!({}))
    } else {
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid lifecycle JSON: {error}"))
    }
}

fn open_regular_input(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("cannot open lifecycle input `{}`: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "cannot inspect lifecycle input `{}`: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "lifecycle input `{}` must be a regular file",
            path.display()
        ));
    }
    Ok(file)
}

fn format_decision(host: &str, decision: &LifecycleDecision) -> Result<Value, String> {
    let mut response = serde_json::to_value(decision)
        .map_err(|error| format!("lifecycle decision encode failed: {error}"))?;
    let Some(protocol) = ags_host_integration::platform_spec(host)
        .and_then(|spec| spec.lifecycle.map(|lifecycle| lifecycle.output))
    else {
        // Generic hosts receive the canonical response. Official codecs are an
        // optional presentation enhancement, not an admission allowlist.
        return Ok(response);
    };
    match (decision.event.as_str(), protocol) {
        (
            "session-start",
            ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible
            | ags_host_integration::LifecycleOutputProtocol::CodeBuddy,
        ) => {
            response["suppressOutput"] = json!(true);
            response["hookSpecificOutput"] = json!({
                "hookEventName": "SessionStart",
                "additionalContext": decision.additional_context.clone().unwrap_or_default(),
            });
        }
        ("session-start", ags_host_integration::LifecycleOutputProtocol::Cursor) => {
            response["additional_context"] =
                json!(decision.additional_context.clone().unwrap_or_default());
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::ClaudeCompatible) => {
            if decision.status == "blocked" {
                response["suppressOutput"] = json!(true);
                response["hookSpecificOutput"] = json!({
                    "hookEventName": "Stop",
                    "additionalContext": decision.additional_context.clone().unwrap_or_default(),
                });
            }
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::CodeBuddy) => {
            let blocked = decision.status == "blocked";
            response["continue"] = json!(!blocked);
            response["suppressOutput"] = json!(true);
            if blocked {
                response["reason"] = json!(decision.additional_context.clone().unwrap_or_default());
            }
        }
        ("stop-guard", ags_host_integration::LifecycleOutputProtocol::Cursor) => {
            if let Some(context) = &decision.additional_context {
                response["followup_message"] = json!(context);
            }
        }
        ("session-end", _) => {}
        (event, _) => return Err(format!("unsupported lifecycle event `{event}`")),
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn instruction(action: HostExecutionAction) -> HostExecutionInstruction {
        HostExecutionInstruction {
            schema_version: HOST_EXECUTION_INSTRUCTION_SCHEMA_VERSION.to_string(),
            action_ref: "action-test".to_string(),
            binding_hash: ags_platform::sha256("binding"),
            plan_hash: ags_platform::sha256("plan"),
            policy_hash: ags_platform::sha256("policy"),
            instruction_digest: ags_platform::sha256("instruction"),
            action,
        }
    }

    #[test]
    fn command_uses_shared_host_runner_and_emits_test_evidence() {
        let temp = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "ags-test@example.invalid"],
            vec!["config", "user.name", "AGS Test"],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .unwrap()
                .success());
        }
        std::fs::write(temp.path().join("seed"), b"seed").unwrap();
        assert!(Command::new("git")
            .args(["add", "seed"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-qm", "seed"])
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
        let workspace = temp.path().canonicalize().unwrap();
        std::fs::create_dir(workspace.join("out")).unwrap();
        let instruction = instruction(HostExecutionAction::Command {
            profile: ags_control_plane::TestProfile::Smoke,
            program: "/usr/bin/touch".to_string(),
            argv: vec!["out/result".to_string()],
            cwd: workspace.clone(),
            env: BTreeMap::new(),
            timeout_ms: 5_000,
            allowed_write_paths: vec![workspace.join("out")],
        });
        let outcome =
            execute_instruction(&instruction, &workspace, "", "token".to_string(), 1).unwrap();
        assert_eq!(outcome.status, HostOutcomeStatus::Succeeded);
        assert_eq!(
            outcome.observed_write_set,
            [workspace.join("out").display().to_string()]
        );
        assert!(outcome.artifacts.is_empty());
        assert_eq!(
            outcome.evidence.as_ref().unwrap().kind,
            HostEvidenceKind::TestReceipt
        );
    }

    #[test]
    fn archive_closures_consumes_only_the_sealed_pointer_set() {
        let temp = tempfile::tempdir().unwrap();
        let pointer = temp.path().join("pointer.json");
        std::fs::write(&pointer, b"{}").unwrap();
        let instruction = instruction(HostExecutionAction::ArchiveClosures {
            event_id: "event-a".to_string(),
            receipt_ids: vec!["receipt-a".to_string()],
            pointer_paths: vec![pointer.clone()],
            expected_write_paths: vec![pointer.clone()],
        });
        let outcome =
            execute_instruction(&instruction, temp.path(), "event-a", "token".to_string(), 1)
                .unwrap();
        assert_eq!(outcome.status, HostOutcomeStatus::Succeeded);
        assert!(!pointer.exists());
        assert_eq!(outcome.artifacts[0].state, HostArtifactState::Absent);
    }

    #[cfg(unix)]
    #[test]
    fn persisted_outcome_uses_the_canonical_runtime_root() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let alias = temp.path().join("runtime-alias");
        std::os::unix::fs::symlink(&runtime, &alias).unwrap();
        let outcome = HostOutcomeReceipt {
            schema_version: HOST_OUTCOME_SCHEMA_VERSION.to_string(),
            action_ref: "action-test".to_string(),
            binding_hash: ags_platform::sha256("binding"),
            plan_hash: ags_platform::sha256("plan"),
            policy_hash: ags_platform::sha256("policy"),
            instruction_digest: ags_platform::sha256("instruction"),
            outcome_token: "outcome-test".to_string(),
            generation: 1,
            status: HostOutcomeStatus::Succeeded,
            output_digest: ags_platform::sha256([]),
            observed_write_set: Vec::new(),
            artifacts: Vec::new(),
            evidence: None,
        };

        let artifact = persist_host_outcome_at(&alias, "action-test", &outcome).unwrap();
        let canonical_runtime = runtime.canonicalize().unwrap();
        assert!(artifact
            .uri
            .starts_with(&format!("file://{}", canonical_runtime.display())));
        assert!(!artifact.uri.contains("runtime-alias"));
    }

    #[test]
    fn runtime_update_materializes_exact_sealed_candidate_members() {
        const NAMES: [&str; 5] = [
            "ags",
            "ags-mcp",
            "ags-host",
            "ags-launcher.js",
            "release-metadata.json",
        ];
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().canonicalize().unwrap();
        let candidate = runtime.join("update-candidates/0.4.20");
        let release = runtime.join("releases/0.4.20");
        std::fs::create_dir_all(&candidate).unwrap();
        let mut members = Vec::new();
        for name in NAMES {
            let path = candidate.join(name);
            let bytes = format!("sealed-{name}").into_bytes();
            std::fs::write(&path, &bytes).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            members.push(HostReleaseMember {
                name: name.to_string(),
                sha256: ags_platform::sha256(&bytes),
                size: bytes.len() as u64,
                mode: 0o644,
            });
        }
        members.sort_by(|left, right| left.name.cmp(&right.name));
        let tree_digest = ags_platform::sha256(serde_json::to_vec(&members).unwrap());
        #[derive(serde::Serialize)]
        struct Manifest<'a> {
            schema_version: &'static str,
            version: &'static str,
            tree_digest: &'a str,
            members: &'a [HostReleaseMember],
        }
        let manifest_bytes = serde_json::to_vec_pretty(&Manifest {
            schema_version: "ags://schema/contract/v2/sealed-release-manifest",
            version: "0.4.20",
            tree_digest: &tree_digest,
            members: &members,
        })
        .unwrap();
        let manifest = HostReleaseMember {
            name: "release-manifest.json".to_string(),
            sha256: ags_platform::sha256(&manifest_bytes),
            size: manifest_bytes.len() as u64,
            mode: 0o644,
        };
        let mut expected = vec![runtime.join("releases"), release.clone()];
        expected.extend(NAMES.map(|name| release.join(name)));
        expected.extend([
            release.join("release-manifest.json"),
            runtime.join("current-release.json"),
            runtime.join("update-state.json"),
        ]);
        let instruction = instruction(HostExecutionAction::RuntimeUpdate {
            channel: "stable".to_string(),
            target_version: Some("0.4.20".to_string()),
            candidate_directory: candidate.clone(),
            release_directory: release.clone(),
            manifest,
            tree_digest,
            members,
            expected_write_paths: expected,
        });
        let last_candidate = candidate.join("release-metadata.json");
        std::fs::write(&last_candidate, b"bad").unwrap();
        assert!(
            execute_instruction(&instruction, temp.path(), "", "token".to_string(), 1).is_err()
        );
        assert!(
            !release.exists(),
            "candidate validation must finish before creating the release"
        );
        let restored = b"sealed-release-metadata.json";
        std::fs::write(&last_candidate, restored).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&last_candidate, std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        let outcome =
            execute_instruction(&instruction, temp.path(), "", "token".to_string(), 1).unwrap();
        assert_eq!(outcome.status, HostOutcomeStatus::Succeeded);
        assert_eq!(outcome.observed_write_set.len(), 10);
        assert_eq!(std::fs::read(release.join("ags")).unwrap(), b"sealed-ags");
        assert_eq!(
            outcome.evidence.as_ref().unwrap().kind,
            HostEvidenceKind::UpdateReceipt
        );
    }

    #[cfg(unix)]
    #[test]
    fn lifecycle_input_path_rejects_symlinks_and_non_regular_files() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("input.json");
        let link = temp.path().join("input-link.json");
        let fifo = temp.path().join("input.fifo");
        let oversized = temp.path().join("oversized.json");
        std::fs::write(&target, br#"{"event":"test"}"#).unwrap();
        symlink(&target, &link).unwrap();
        let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
        std::fs::write(&oversized, vec![b'x'; MAX_INPUT_BYTES + 1]).unwrap();

        assert!(read_payload(link.to_str().unwrap()).is_err());
        assert!(read_payload("/dev/null").is_err());
        assert!(read_payload(fifo.to_str().unwrap()).is_err());
        assert_eq!(
            read_payload(oversized.to_str().unwrap()).unwrap_err(),
            "lifecycle input exceeds 1 MiB"
        );
    }
}
