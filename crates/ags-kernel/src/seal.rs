//! Sealed transactions (contract v3 §7.2 / §7.8).
//!
//! decide → sealed plan + single-use `action_ref`; only `apply` may consume
//! it, once, in the same authenticated binding. Five states:
//! blocked / planned / applying / receipted / risk-escalated. Replay, tamper
//! and cross-binding use fail closed: apply recomputes the payload hash, the
//! binding hash and the action_ref token from the plan contents, so any edit
//! to `.ags/state/seals/{token}.json` (payload, plan_hash, nonce or state
//! metadata) is rejected before the effect runs. The registry only contains
//! this sealed subset (G-06).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::workspace::{binding_hash, sha256_hex, WorkspaceBinding};

pub const SEAL_STATES: [&str; 5] = [
    "blocked",
    "planned",
    "applying",
    "receipted",
    "risk-escalated",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRef {
    pub token: String,
    pub operation: String,
    pub plan_hash: String,
    pub binding_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub operation: String,
    pub payload: Value,
    pub plan_hash: String,
    pub binding_hash: String,
    pub nonce: String,
    pub state: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub receipt_id: String,
    pub operation: String,
    pub plan_hash: String,
    pub token: String,
    pub binding_hash: String,
    pub state: String,
    pub observed_write_set: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOutput {
    pub observed_write_set: Vec<String>,
    pub result: Option<Value>,
}

impl ApplyOutput {
    pub fn with_result(observed_write_set: Vec<String>, result: Value) -> Self {
        ApplyOutput {
            observed_write_set,
            result: Some(result),
        }
    }
}

impl From<Vec<String>> for ApplyOutput {
    fn from(observed_write_set: Vec<String>) -> Self {
        ApplyOutput {
            observed_write_set,
            result: None,
        }
    }
}

pub struct SealStore {
    pub dir: PathBuf,
}

/// Canonical payload serialization shared by seal and apply so the content
/// hash is recomputable byte-for-byte.
pub fn canonical_payload(payload: &Value) -> Result<String> {
    serde_json::to_string(payload)
        .map_err(|e| Error::new("plan_payload_encode_failed", e.to_string()))
}

/// The token derivation: everything that must not change between seal and
/// apply participates.
fn derive_token(bhash: &str, operation: &str, plan_hash: &str, nonce: &str) -> String {
    sha256_hex(format!("{bhash}|{operation}|{plan_hash}|{nonce}").as_bytes())
}

/// Unpredictable nonce from the OS CSPRNG; a timestamp/pid fallback keeps the
/// seal usable on exotic platforms without weakening the common case.
fn random_nonce() -> String {
    let mut buf = [0u8; 32];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| {
            use std::io::Read;
            f.read_exact(&mut buf)
        })
        .is_ok();
    if ok {
        return buf.iter().map(|b| format!("{b:02x}")).collect();
    }
    format!(
        "{:x}-{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::ptr::addr_of!(buf) as usize,
    )
}

impl SealStore {
    pub fn new(binding: &WorkspaceBinding) -> Self {
        SealStore {
            dir: binding.state_dir.join("seals"),
        }
    }

    fn plan_path(&self, token: &str) -> PathBuf {
        self.dir.join(format!("{token}.json"))
    }

    fn applied_path(&self, token: &str) -> PathBuf {
        self.dir.join("applied").join(token)
    }

    /// Sealed decide: freeze a plan for `operation` + `payload`. No mutation
    /// happens here; the returned action_ref is the only replay authority.
    pub fn seal_plan(
        &self,
        operation: &str,
        payload: &Value,
        binding: &WorkspaceBinding,
    ) -> Result<ActionRef> {
        let canonical = canonical_payload(payload)?;
        let plan_hash = sha256_hex(canonical.as_bytes());
        let bhash = binding_hash(binding);
        let nonce = random_nonce();
        let token = derive_token(&bhash, operation, &plan_hash, &nonce);
        let plan = Plan {
            operation: operation.to_string(),
            payload: payload.clone(),
            plan_hash,
            binding_hash: bhash.clone(),
            nonce,
            state: "planned".to_string(),
            token: token.clone(),
        };
        fs::create_dir_all(&self.dir)
            .map_err(|e| crate::error::io("seal_dir_create_failed", &e))?;
        write_json_atomic(&self.plan_path(&token), &plan)?;
        Ok(ActionRef {
            token,
            operation: operation.to_string(),
            plan_hash: plan.plan_hash,
            binding_hash: bhash,
        })
    }

    /// Verify the full integrity chain before any state transition or effect:
    /// payload → plan_hash, plan fields → token, binding hash.
    fn verify_plan_integrity(
        &self,
        plan: &Plan,
        token: &str,
        binding: &WorkspaceBinding,
    ) -> Result<()> {
        if plan.binding_hash != binding_hash(binding) {
            return Err(Error::new(
                "binding_mismatch",
                "action_ref was sealed for a different workspace binding",
            ));
        }
        let recomputed_plan_hash = sha256_hex(canonical_payload(&plan.payload)?.as_bytes());
        if recomputed_plan_hash != plan.plan_hash {
            return Err(Error::new(
                "plan_tampered",
                "plan payload does not match the sealed plan_hash",
            ));
        }
        let expected_token = derive_token(
            &plan.binding_hash,
            &plan.operation,
            &plan.plan_hash,
            &plan.nonce,
        );
        if expected_token != token {
            return Err(Error::new(
                "plan_tampered",
                "plan fields do not recompute the action_ref token",
            ));
        }
        Ok(())
    }

    /// Sealed apply: verify integrity, run the domain effect exactly once,
    /// then receipt. Any effect failure parks the plan at `risk-escalated`.
    pub fn apply<F>(&self, token: &str, binding: &WorkspaceBinding, effect: F) -> Result<Receipt>
    where
        F: FnOnce(&Plan) -> Result<Vec<String>>,
    {
        self.apply_with_result(token, binding, |plan| effect(plan).map(Into::into))
    }

    pub fn apply_with_result<F>(
        &self,
        token: &str,
        binding: &WorkspaceBinding,
        effect: F,
    ) -> Result<Receipt>
    where
        F: FnOnce(&Plan) -> Result<ApplyOutput>,
    {
        let path = self.plan_path(token);
        let text =
            fs::read_to_string(&path).map_err(|e| crate::error::io("action_ref_unknown", &e))?;
        let mut plan: Plan =
            serde_json::from_str(&text).map_err(|e| Error::new("plan_corrupted", e.to_string()))?;
        self.verify_plan_integrity(&plan, token, binding)?;
        if plan.state != "planned" {
            return Err(Error::new(
                "action_ref_already_consumed",
                format!("plan state is `{}`, not `planned`", plan.state),
            ));
        }
        fs::create_dir_all(self.applied_path(token).parent().unwrap())
            .map_err(|e| crate::error::io("seal_dir_create_failed", &e))?;
        // Atomic single-consumer claim: the O_EXCL create is the one gate, so
        // two racing applies can never both run the effect.
        let claim = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.applied_path(token));
        if claim.is_err() {
            return Err(Error::new(
                "action_ref_already_consumed",
                "token already claimed",
            ));
        }
        plan.state = "applying".to_string();
        write_json_atomic(&path, &plan)?;
        match effect(&plan) {
            Ok(output) => {
                plan.state = "receipted".to_string();
                write_json_atomic(&path, &plan)?;
                let receipt_id = format!("receipt-v3-{}", &plan.plan_hash[..16]);
                Ok(Receipt {
                    receipt_id,
                    operation: plan.operation.clone(),
                    plan_hash: plan.plan_hash.clone(),
                    token: token.to_string(),
                    binding_hash: plan.binding_hash.clone(),
                    state: "receipted".to_string(),
                    observed_write_set: output.observed_write_set,
                    result: output.result,
                })
            }
            Err(e) => {
                plan.state = "risk-escalated".to_string();
                let _ = write_json_atomic(&path, &plan);
                Err(Error::new(
                    "apply_risk_escalated",
                    format!("{}; plan parked at risk-escalated", e.message),
                ))
            }
        }
    }

    pub fn load_plan(&self, token: &str) -> Result<Plan> {
        let text = fs::read_to_string(self.plan_path(token))
            .map_err(|e| crate::error::io("action_ref_unknown", &e))?;
        serde_json::from_str(&text).map_err(|e| Error::new("plan_corrupted", e.to_string()))
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| Error::new("plan_encode_failed", e.to_string()))?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp).map_err(|e| crate::error::io("plan_write_failed", &e))?;
        f.write_all(text.as_bytes())
            .and_then(|_| f.flush())
            .map_err(|e| crate::error::io("plan_write_failed", &e))?;
    }
    fs::rename(&tmp, path).map_err(|e| crate::error::io("plan_write_failed", &e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::bind;
    use serde_json::json;

    fn binding_for(tmp: &tempfile::TempDir) -> WorkspaceBinding {
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        let slug = root.file_name().unwrap().to_string_lossy().to_string();
        let toml =
            format!("[workspace]\nslug = \"{slug}\"\nrole = \"A\"\n[sealed]\nops = [\"update\"]\n");
        fs::write(root.join("ags.toml"), toml).unwrap();
        bind(&root).unwrap()
    }

    #[test]
    fn sealed_apply_is_single_use() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store
            .seal_plan("update", &json!({"channel": "stable"}), &binding)
            .unwrap();
        let r1 = store
            .apply(&action.token, &binding, |_| {
                Ok(vec!["state/updated.json".to_string()])
            })
            .unwrap();
        assert_eq!(r1.state, "receipted");
        assert_eq!(r1.observed_write_set.len(), 1);
        let err = store
            .apply(&action.token, &binding, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "action_ref_already_consumed");
    }

    #[test]
    fn cross_binding_apply_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store.seal_plan("update", &json!({}), &binding).unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let other = binding_for(&tmp2);
        let err = store
            .apply(&action.token, &other, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "binding_mismatch");
    }

    #[test]
    fn tampered_payload_is_rejected_before_effect() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store
            .seal_plan("update", &json!({"channel": "stable"}), &binding)
            .unwrap();
        // Attacker rewrites the payload in the plan file.
        let path = store.plan_path(&action.token);
        let mut plan: Plan = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        plan.payload = json!({"channel": "evil"});
        fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).unwrap();
        let err = store
            .apply(&action.token, &binding, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "plan_tampered");
        assert_eq!(store.load_plan(&action.token).unwrap().state, "planned");
    }

    #[test]
    fn tampered_plan_hash_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store
            .seal_plan("update", &json!({"channel": "stable"}), &binding)
            .unwrap();
        let path = store.plan_path(&action.token);
        let mut plan: Plan = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        plan.plan_hash = "sha256:deadbeef".to_string();
        fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).unwrap();
        let err = store
            .apply(&action.token, &binding, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "plan_tampered");
    }

    #[test]
    fn tampered_nonce_breaks_token() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store
            .seal_plan("update", &json!({"channel": "stable"}), &binding)
            .unwrap();
        let path = store.plan_path(&action.token);
        let mut plan: Plan = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        plan.nonce = "0".to_string();
        fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).unwrap();
        let err = store
            .apply(&action.token, &binding, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "plan_tampered");
    }

    #[test]
    fn concurrent_apply_consumes_exactly_once() {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;
        let tmp = tempfile::tempdir().unwrap();
        let binding = Arc::new(binding_for(&tmp));
        let store = Arc::new(SealStore::new(&binding));
        let action = store.seal_plan("update", &json!({}), &binding).unwrap();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            let binding = binding.clone();
            let token = action.token.clone();
            handles.push(thread::spawn(move || {
                thread::sleep(Duration::from_millis(5));
                store.apply(&token, &binding, |_| {
                    thread::sleep(Duration::from_millis(20));
                    Ok(vec!["state/x".to_string()])
                })
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok_count, 1, "exactly one racing apply must win");
        let consumed = results
            .iter()
            .filter(|r| matches!(r, Err(e) if e.code == "action_ref_already_consumed"));
        assert_eq!(consumed.count(), 7);
    }

    #[test]
    fn effect_failure_parks_risk_escalated() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store.seal_plan("update", &json!({}), &binding).unwrap();
        let err = store
            .apply(&action.token, &binding, |_| {
                Err(Error::new("effect_failed", "boom"))
            })
            .unwrap_err();
        assert_eq!(err.code, "apply_risk_escalated");
        assert_eq!(
            store.load_plan(&action.token).unwrap().state,
            "risk-escalated"
        );
    }

    #[test]
    fn unknown_token_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let err = store
            .apply("deadbeef", &binding, |_| Ok(vec![]))
            .unwrap_err();
        assert_eq!(err.code, "action_ref_unknown");
    }

    #[test]
    fn effect_can_return_structured_result() {
        let tmp = tempfile::tempdir().unwrap();
        let binding = binding_for(&tmp);
        let store = SealStore::new(&binding);
        let action = store.seal_plan("demo", &json!({"x": 1}), &binding).unwrap();
        let receipt = store
            .apply_with_result(&action.token, &binding, |_| {
                Ok(ApplyOutput::with_result(
                    vec!["evidence:ev-1".to_string()],
                    json!({"state": "dispatch_ready", "grant_id": "dg-1"}),
                ))
            })
            .unwrap();
        assert_eq!(receipt.result.unwrap()["grant_id"], "dg-1");
    }

    #[test]
    fn seal_states_are_exactly_five() {
        assert_eq!(SEAL_STATES.len(), 5);
    }
}
