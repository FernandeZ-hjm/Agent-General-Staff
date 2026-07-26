use std::path::Path;

/// Identify the intentionally reduced public source checkout from its
/// repository-owned entry documents, not from machine paths or missing private
/// directories.
pub(crate) fn is_public_edition(repo_root: &Path) -> bool {
    let mut declared_public = false;
    for path in [repo_root.join("WORKSPACE.md"), repo_root.join("CLAUDE.md")] {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if raw.contains("Public Edition") || raw.contains("public distributable edition") {
                declared_public = true;
                break;
            }
        }
    }
    declared_public && crate::sync::manifest::verify_release_manifest(repo_root).passed
}

#[cfg(test)]
mod tests {
    use super::is_public_edition;
    use std::path::Path;

    #[test]
    fn documentation_claim_without_closed_payload_is_not_a_public_edition() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("WORKSPACE.md"),
            "# AGS Public Edition Workspace\n",
        )
        .unwrap();
        assert!(!is_public_edition(root.path()));
    }

    #[test]
    fn current_checkout_identity_matches_its_verified_public_payload() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let verified = crate::sync::manifest::verify_release_manifest(root).passed;
        assert_eq!(is_public_edition(root), verified);
    }
}
