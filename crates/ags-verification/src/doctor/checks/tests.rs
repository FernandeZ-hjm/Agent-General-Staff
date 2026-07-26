#[cfg(test)]
mod tests {
    use super::super::types::CheckStatus;
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("ags-public-doctor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn public_edition_is_detected_from_the_public_workspace_contract() {
        let root = fixture("edition");
        std::fs::write(root.join("WORKSPACE.md"), "# AGS Public Edition\n").unwrap();
        assert!(is_public_edition(&root));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn generic_registry_check_is_informational_when_entry_is_absent() {
        let root = fixture("registry");
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::write(
            root.join("manifests/mcp-registry.yaml"),
            "schema_version: 1\nmcps: []\n",
        )
        .unwrap();
        assert_eq!(
            mcp_registry_codegraph_adopted(&root).status,
            CheckStatus::Pass
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn memory_script_check_reports_missing_assets_without_panicking() {
        let root = fixture("memory");
        assert_eq!(
            memory_capture_scripts_present_at(&root).status,
            CheckStatus::Warn
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
