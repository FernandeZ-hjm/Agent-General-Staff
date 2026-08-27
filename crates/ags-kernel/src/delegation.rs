//! Delegation (contract v3 §7.10).
//!
//! A DelegationGrant is issued through the sealed `govern.delegation.issue`
//! operation, narrowed at issuance against the parent task authority and the
//! workspace guardrails, and recorded inside the evidence event (the chain
//! hash is the tamper protection — no separate grant file exists). A child
//! instance accepts, executes, and RETURNS; only the task owner integrates,
//! verifies and closes. Delegation only narrows, never widens.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::evidence::{Event, EvidenceLog};
use crate::workspace::WorkspaceBinding;

pub const GRANT_ID_PREFIX: &str = "grant-";
static GRANT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct DelegationGrant {
    pub grant_id: String,
    pub parent_contract: String,
    pub target_agent: String,
    pub subtask: String,
    pub allowed_resources: Vec<String>,
    pub allowed_capabilities: Vec<String>,
    pub delegation_depth: u32,
    pub return_contract: String,
    pub owner_instance: String,
}

impl DelegationGrant {
    pub fn from_event(event: &Event) -> Option<DelegationGrant> {
        serde_json::from_value(event.payload.clone()).ok()
    }
}

/// Issue a grant: narrow the parent task authority, check guardrails, and
/// record the grant inside a sealed evidence event. Returns the event.
pub fn issue(
    binding: &WorkspaceBinding,
    config: &Config,
    payload: &serde_json::Value,
) -> Result<Event> {
    let parent_contract = str_field(payload, "parent_contract")?;
    let target_agent = str_field(payload, "target_agent")?;
    let subtask = str_field(payload, "subtask")?;
    let return_contract = str_field(payload, "return_contract")?;
    let owner_instance = str_field(payload, "owner_instance")?;
    let depth = payload
        .get("delegation_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let allowed_resources = str_list(payload, "allowed_resources");
    let allowed_capabilities = str_list(payload, "allowed_capabilities");

    let max_depth = config.guardrails.max_delegation_depth;
    if max_depth == 0 || depth == 0 {
        return Err(Error::new(
            "delegation_disabled",
            "delegation is disabled by guardrails.max_delegation_depth",
        ));
    }
    if depth > max_depth {
        return Err(Error::new(
            "delegation_depth_exceeded",
            format!("depth {depth} exceeds guardrails.max_delegation_depth {max_depth}"),
        ));
    }
    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let all = evidence.read_all()?;
    EvidenceLog::verify_chain(&all)?;

    // Parent authority: the prepare event must exist and its writable
    // resources must cover every allowed resource (narrowing only).
    let parent_authority = all
        .iter()
        .find(|e| {
            e.task_card_hash.as_deref() == Some(parent_contract)
                && e.event_type == "execution"
                && e.payload.get("phase").and_then(|v| v.as_str()) == Some("prepare")
        })
        .ok_or_else(|| {
            Error::new(
                "delegation_parent_unknown",
                format!("no prepare evidence for parent contract {parent_contract}"),
            )
        })?;
    let parent_writable: Vec<String> = parent_authority
        .payload
        .get("authority")
        .and_then(|a| a.get("writable_resources"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let parent_ceiling: Vec<String> = parent_authority
        .payload
        .get("authority")
        .and_then(|a| a.get("capability_ceiling"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let root_open = parent_writable.iter().any(|w| w == ".");
    for resource in &allowed_resources {
        let rel = resource.trim_start_matches("./");
        if config
            .guardrails
            .protected_resources
            .iter()
            .any(|p| rel == p || rel.starts_with(&format!("{p}/")))
        {
            return Err(Error::new(
                "delegation_protected_resource",
                format!("resource `{resource}` is protected by guardrails"),
            ));
        }
        if !root_open
            && !parent_writable
                .iter()
                .any(|w| rel == w || rel.starts_with(&format!("{w}/")))
        {
            return Err(Error::new(
                "delegation_widens_writes",
                format!("resource `{resource}` is outside the parent task's writable resources"),
            ));
        }
    }
    for capability in &allowed_capabilities {
        if !parent_ceiling.is_empty() && !parent_ceiling.iter().any(|c| c == capability) {
            return Err(Error::new(
                "delegation_widens_capabilities",
                format!("capability `{capability}` is outside the parent task's ceiling"),
            ));
        }
    }

    // The target must be a configured host that declares dispatch support;
    // otherwise the host cannot spawn the child (dispatch_unsupported).
    let target_ok = config.hosts.iter().any(|host| {
        host.dispatch
            && crate::hosts::normalize_host_id(&host.id)
                .map(|id| id == target_agent)
                .unwrap_or(false)
    });
    if !target_ok {
        return Err(Error::new(
            "dispatch_unsupported",
            format!(
                "host `{target_agent}` is not configured with dispatch=true in ags.toml [hosts]"
            ),
        ));
    }

    let nonce = format!(
        "{}:{}:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        GRANT_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let grant_material = serde_json::json!({
        "parent_contract": parent_contract,
        "target_agent": target_agent,
        "subtask": subtask,
        "allowed_resources": &allowed_resources,
        "allowed_capabilities": &allowed_capabilities,
        "delegation_depth": depth,
        "return_contract": return_contract,
        "owner_instance": owner_instance,
        "evidence_tip": all.last().map(|event| event.sha256.as_str()),
        "nonce": nonce,
    });
    let grant = DelegationGrant {
        grant_id: format!(
            "{GRANT_ID_PREFIX}{}",
            crate::workspace::sha256_hex(grant_material.to_string().as_bytes())
                .chars()
                .take(16)
                .collect::<String>()
        ),
        parent_contract: parent_contract.to_string(),
        target_agent: target_agent.to_string(),
        subtask: subtask.to_string(),
        allowed_resources,
        allowed_capabilities,
        delegation_depth: depth,
        return_contract: return_contract.to_string(),
        owner_instance: owner_instance.to_string(),
    };
    let payload_json = serde_json::to_value(&grant)
        .map_err(|e| Error::new("delegation_encode_failed", e.to_string()))?;
    evidence.append_with_instance(
        "delegation.issue",
        &binding.slug,
        Some(parent_contract),
        &Event::scoped_scope(Some(parent_contract), Some(owner_instance)),
        Some(owner_instance),
        None,
        payload_json,
    )
}

pub fn dispatch_result(event: &Event) -> Result<serde_json::Value> {
    let grant = DelegationGrant::from_event(event).ok_or_else(|| {
        Error::new(
            "delegation_grant_corrupted",
            "delegation.issue did not produce a readable grant",
        )
    })?;
    Ok(serde_json::json!({
        "state": "dispatch_ready",
        "grant_id": grant.grant_id,
        "target_agent": grant.target_agent,
        "child_contract": grant,
    }))
}

/// Child instance accepts the grant. Requires the grant to exist and to not
/// have been accepted before.
pub fn accept(binding: &WorkspaceBinding, grant_id: &str, instance: &str) -> Result<Event> {
    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let all = evidence.read_all()?;
    let (issue_event, _grant) = find_grant(&all, grant_id)?;
    if all.iter().any(|e| {
        e.event_type == "delegation.accept"
            && e.payload.get("grant_id") == Some(&serde_json::json!(grant_id))
    }) {
        return Err(Error::new(
            "delegation_already_accepted",
            format!("grant {grant_id} was already accepted"),
        ));
    }
    let owner = issue_event
        .payload
        .get("owner_instance")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let task = issue_event.task_card_hash.as_deref().unwrap_or("");
    evidence.append_with_instance(
        "delegation.accept",
        &binding.slug,
        Some(task),
        &Event::scoped_scope(Some(task), Some(instance)),
        Some(instance),
        Some(owner),
        serde_json::json!({ "grant_id": grant_id }),
    )
}

/// Child returns its result. Requires a prior accept by the same instance.
pub fn return_result(
    binding: &WorkspaceBinding,
    grant_id: &str,
    instance: &str,
    summary: serde_json::Value,
) -> Result<Event> {
    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let all = evidence.read_all()?;
    let (issue_event, _grant) = find_grant(&all, grant_id)?;
    let accepted = all.iter().find(|e| {
        e.event_type == "delegation.accept"
            && e.payload.get("grant_id") == Some(&serde_json::json!(grant_id))
    });
    let Some(accepted) = accepted else {
        return Err(Error::new(
            "delegation_not_accepted",
            format!("grant {grant_id} has not been accepted"),
        ));
    };
    if accepted.agent_instance_id.as_deref() != Some(instance) {
        return Err(Error::new(
            "delegation_instance_mismatch",
            "the returning instance did not accept this grant",
        ));
    }
    let owner = issue_event
        .payload
        .get("owner_instance")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let task = issue_event.task_card_hash.as_deref().unwrap_or("");
    evidence.append_with_instance(
        "delegation.return",
        &binding.slug,
        Some(task),
        &Event::scoped_scope(Some(task), Some(instance)),
        Some(instance),
        Some(owner),
        serde_json::json!({ "grant_id": grant_id, "summary": summary }),
    )
}

/// The task owner integrates a returned grant. Only the owner instance may
/// integrate; a child can never close the parent task.
pub fn integrate(binding: &WorkspaceBinding, grant_id: &str, instance: &str) -> Result<Event> {
    let evidence = EvidenceLog::new(binding.evidence_dir.clone());
    let all = evidence.read_all()?;
    let (issue_event, _grant) = find_grant(&all, grant_id)?;
    let owner = issue_event
        .payload
        .get("owner_instance")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if instance != owner {
        return Err(Error::new(
            "delegation_owner_required",
            "only the task owner may integrate a returned grant",
        ));
    }
    let returned = all.iter().find(|e| {
        e.event_type == "delegation.return"
            && e.payload.get("grant_id") == Some(&serde_json::json!(grant_id))
    });
    if returned.is_none() {
        return Err(Error::new(
            "delegation_not_returned",
            format!("grant {grant_id} has not returned"),
        ));
    }
    let task = issue_event.task_card_hash.as_deref().unwrap_or("");
    evidence.append_with_instance(
        "delegation.integrate",
        &binding.slug,
        Some(task),
        &Event::scoped_scope(Some(task), Some(instance)),
        Some(instance),
        None,
        serde_json::json!({ "grant_id": grant_id }),
    )
}

/// Locate the issue event for a grant id.
pub fn find_grant<'a>(all: &'a [Event], grant_id: &str) -> Result<(&'a Event, DelegationGrant)> {
    let event = all
        .iter()
        .find(|e| {
            e.event_type == "delegation.issue"
                && e.payload.get("grant_id") == Some(&serde_json::json!(grant_id))
        })
        .ok_or_else(|| {
            Error::new(
                "delegation_grant_unknown",
                format!("no delegation.issue evidence for grant {grant_id}"),
            )
        })?;
    let grant = DelegationGrant::from_event(event).ok_or_else(|| {
        Error::new(
            "delegation_grant_corrupted",
            format!("grant {grant_id} payload is unreadable"),
        )
    })?;
    Ok((event, grant))
}

fn str_field<'a>(payload: &'a serde_json::Value, name: &str) -> Result<&'a str> {
    payload.get(name).and_then(|v| v.as_str()).ok_or_else(|| {
        Error::new(
            "delegation_payload_missing",
            format!("payload requires `{name}`"),
        )
    })
}

fn str_list(payload: &serde_json::Value, name: &str) -> Vec<String> {
    payload
        .get(name)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::scaffold;
    use crate::workspace::bind;
    use serde_json::json;
    use std::fs;

    fn ws(tmp: &tempfile::TempDir) -> WorkspaceBinding {
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ags.toml"),
            "[workspace]\nslug = \"t\"\nrole = \"A\"\n",
        )
        .unwrap();
        bind(&root).unwrap()
    }

    /// Prepare evidence for a parent task with the given authority.
    fn seed_parent(binding: &WorkspaceBinding, resources: &[&str], ceiling: &[&str]) -> String {
        let evidence = EvidenceLog::new(binding.evidence_dir.clone());
        evidence
            .append_with_instance(
                "execution",
                &binding.slug,
                Some("tc-0123456789abcdef"),
                "local",
                Some("owner-1"),
                None,
                json!({
                    "phase": "prepare",
                    "authority": {
                        "card_hash": "tc-0123456789abcdef",
                        "goals": ["G-01"],
                        "acceptance_criteria": ["AC-01"],
                        "writable_resources": resources,
                        "capability_ceiling": ceiling,
                        "verification": ["V-01"],
                        "review_required": false,
                    }
                }),
            )
            .unwrap();
        "tc-0123456789abcdef".to_string()
    }

    fn config_with_dispatch() -> crate::config::Config {
        let mut config = scaffold("t");
        config.hosts = vec![crate::config::HostEntry {
            id: "codebuddy".to_string(),
            surface: "hybrid".to_string(),
            dispatch: true,
        }];
        config
    }

    #[test]
    fn issue_narrows_and_records_grant() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let config = config_with_dispatch();
        seed_parent(&binding, &["src"], &["skill:database-migration"]);
        let event = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "migrate the schema",
                "allowed_resources": ["src/db"],
                "allowed_capabilities": ["skill:database-migration"],
                "delegation_depth": 1,
                "return_contract": "migration diff",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap();
        assert_eq!(event.event_type, "delegation.issue");
        let dispatch = dispatch_result(&event).unwrap();
        assert_eq!(dispatch["state"], "dispatch_ready");
        assert_eq!(dispatch["target_agent"], "codebuddy");
        assert_eq!(dispatch["grant_id"], event.payload["grant_id"]);
        assert_eq!(
            event.agent_instance_id.as_deref(),
            Some("owner-1"),
            "the issue is recorded under the owner instance"
        );
        let (_, grant) = find_grant(
            &event_chain(&binding),
            event.payload["grant_id"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(grant.delegation_depth, 1);
        assert_eq!(grant.allowed_resources, vec!["src/db"]);

        let second = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "migrate the API",
                "allowed_resources": ["src/api"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "API diff",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap();
        assert_ne!(event.payload["grant_id"], second.payload["grant_id"]);
    }

    fn event_chain(binding: &WorkspaceBinding) -> Vec<Event> {
        EvidenceLog::new(binding.evidence_dir.clone())
            .read_all()
            .unwrap()
    }

    #[test]
    fn issue_rejects_unknown_or_undispatchable_target() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let config = scaffold("t"); // no hosts at all
        seed_parent(&binding, &["src"], &[]);
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["src/a"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "dispatch_unsupported");
    }

    #[test]
    fn issue_rejects_widening_and_protected_resources() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let config = config_with_dispatch();
        seed_parent(&binding, &["src"], &["skill:database-migration"]);
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["outside"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "delegation_widens_writes");
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": [".ags/secret"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "delegation_protected_resource");
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["src/db"],
                "allowed_capabilities": ["skill:superpowers"],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "delegation_widens_capabilities");
    }

    #[test]
    fn issue_respects_max_depth() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let mut config = config_with_dispatch();
        config.guardrails.max_delegation_depth = 1;
        seed_parent(&binding, &["src"], &[]);
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["src/a"],
                "allowed_capabilities": [],
                "delegation_depth": 2,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "delegation_depth_exceeded");
        config.guardrails.max_delegation_depth = 0;
        let err = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["src/a"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap_err();
        assert_eq!(err.code, "delegation_disabled");
    }

    #[test]
    fn accept_return_integrate_flow_and_owner_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = ws(&tmp);
        let config = config_with_dispatch();
        seed_parent(&binding, &["src"], &[]);
        let issued = issue(
            &binding,
            &config,
            &json!({
                "parent_contract": "tc-0123456789abcdef",
                "target_agent": "codebuddy",
                "subtask": "x",
                "allowed_resources": ["src/a"],
                "allowed_capabilities": [],
                "delegation_depth": 1,
                "return_contract": "x",
                "owner_instance": "owner-1",
            }),
        )
        .unwrap();
        let grant_id = issued.payload["grant_id"].as_str().unwrap();
        // A child can never integrate before returning.
        let err = integrate(&binding, grant_id, "owner-1").unwrap_err();
        assert_eq!(err.code, "delegation_not_returned");
        // A stranger cannot integrate even after a return.
        accept(&binding, grant_id, "child-1").unwrap();
        return_result(&binding, grant_id, "child-1", json!({"diff": "ok"})).unwrap();
        let err = integrate(&binding, grant_id, "child-1").unwrap_err();
        assert_eq!(err.code, "delegation_owner_required");
        // Owner integrates and the derived state becomes INTEGRATED.
        integrate(&binding, grant_id, "owner-1").unwrap();
        let all = event_chain(&binding);
        assert_eq!(crate::evidence::Event::scoped_scope(None, None), "local");
        assert!(all.iter().any(|e| e.event_type == "delegation.integrate"));
        // Double accept is rejected.
        let err = accept(&binding, grant_id, "child-2").unwrap_err();
        assert_eq!(err.code, "delegation_already_accepted");
    }
}
