use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use ags_platform::sha256;

/// Per-client-session one-shot action storage.
///
/// `T` is owned by the decision layer; this type owns only session identity,
/// generation invalidation, and isolation. It deliberately has no workspace-
/// global singleton and therefore cannot leak leases across clients.
#[derive(Debug)]
pub struct SessionActionStore<T> {
    connection_nonce: String,
    generation: u64,
    actions: HashMap<String, T>,
}

impl<T> Default for SessionActionStore<T> {
    fn default() -> Self {
        static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
        let sequence = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            connection_nonce: sha256(
                format!("connection\n{}\n{now}\n{sequence}", std::process::id()).as_bytes(),
            ),
            generation: 0,
            actions: HashMap::new(),
        }
    }
}

impl<T> SessionActionStore<T> {
    pub fn for_session(session_id: &str) -> Self {
        Self {
            connection_nonce: sha256(format!("workspace-session\n{session_id}").as_bytes()),
            generation: 0,
            actions: HashMap::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.actions.clear();
    }

    pub fn stable_id(&self, prefix: &str, basis: &str) -> String {
        let digest =
            sha256(format!("{}\n{}\n{basis}", self.connection_nonce, self.generation).as_bytes());
        format!(
            "{prefix}-{}",
            digest
                .trim_start_matches("sha256:")
                .get(..20)
                .unwrap_or("invalid")
        )
    }

    pub fn insert(&mut self, action_id: String, action: T) -> &T {
        self.actions.insert(action_id.clone(), action);
        self.actions
            .get(&action_id)
            .expect("inserted session action")
    }

    pub fn get(&self, action_id: &str) -> Option<&T> {
        self.actions.get(action_id)
    }

    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.actions.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.actions.values_mut()
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_isolate_action_namespaces() {
        let first = SessionActionStore::<()>::for_session("first");
        let second = SessionActionStore::<()>::for_session("second");
        assert_ne!(
            first.stable_id("action", "same"),
            second.stable_id("action", "same")
        );
    }

    #[test]
    fn invalidation_expires_held_actions_and_changes_the_namespace() {
        let mut session = SessionActionStore::for_session("session");
        let before = session.stable_id("action", "same");
        session.insert("action".to_string(), ());
        session.invalidate();
        assert!(session.get("action").is_none());
        assert_ne!(session.stable_id("action", "same"), before);
    }
}
