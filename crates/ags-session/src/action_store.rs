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
    pub connection_nonce: String,
    pub generation: u64,
    pub actions: HashMap<String, T>,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_isolate_action_namespaces() {
        let first = SessionActionStore::<()>::for_session("first");
        let second = SessionActionStore::<()>::for_session("second");
        assert_ne!(first.connection_nonce, second.connection_nonce);
    }

    #[test]
    fn invalidation_advances_generation_and_drops_actions() {
        let mut session = SessionActionStore::for_session("session");
        session.actions.insert("action".to_string(), ());
        session.invalidate();
        assert_eq!(session.generation, 1);
        assert!(session.actions.is_empty());
    }
}
