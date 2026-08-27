//! Sensitivity scan for projected content (v0.4.21).
//!
//! Every byte projected A→B is checked against the fixed pattern list; a
//! hit becomes a blocking finding on the plan, so private paths, machine
//! identities and retired runtime mentions can never reach B.

use crate::spec;

/// Scan one projected file. Returns `Some(finding)` on the first hit.
pub fn scan(rel_path: &str, bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    for pattern in spec::SENSITIVE_PATTERNS {
        if text.contains(pattern) {
            return Some(format!(
                "sensitive pattern `{pattern}` in projected file {rel_path}"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_private_paths() {
        let vol = concat!("/Volumes/", "My ", "Passport");
        let home = concat!("/Users/", "hu", "jiaming");
        let em = concat!("Evo", "Map");
        let ws = concat!("agent-governance-suite-", "private");
        assert!(scan("README.md", format!("{vol}/AI Project/x").as_bytes()).is_some());
        assert!(scan("a.rs", format!("{home}/.config").as_bytes()).is_some());
        assert!(scan("a.rs", format!("{em} runtime").as_bytes()).is_some());
        assert!(scan("a.rs", ws.as_bytes()).is_some());
    }

    #[test]
    fn passes_clean_content() {
        assert!(scan("README.md", b"# Agent Governance Suite\n").is_none());
        assert!(scan("a.rs", b"fn main() {}\n").is_none());
    }
}
