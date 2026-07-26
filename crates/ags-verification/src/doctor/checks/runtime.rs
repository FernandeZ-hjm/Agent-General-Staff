use super::*;

pub(super) fn mcp_registry_entry_status(
    repo_root: &Path,
    mcp_name: &str,
) -> Result<Option<String>, Finding> {
    let path = repo_root.join("manifests/mcp-registry.yaml");
    if !path.exists() {
        if is_public_edition(repo_root) {
            return Err(Finding::info(
                format!("mcp_registry_{mcp_name}_adopted"),
                "public edition does not require private mcp-registry.yaml",
            ));
        }
        return Err(Finding::warn(
            format!("mcp_registry_{mcp_name}_adopted"),
            "manifests/mcp-registry.yaml not found",
            format!("Expected at: {}", path.display()),
        ));
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return Err(Finding::warn(
                format!("mcp_registry_{mcp_name}_adopted"),
                "cannot read manifests/mcp-registry.yaml",
                format!("{e}"),
            ));
        }
    };

    let parsed: YamlValue = match serde_yaml::from_str(&raw) {
        Ok(value) => value,
        Err(e) => {
            return Err(Finding::warn(
                format!("mcp_registry_{mcp_name}_adopted"),
                "manifests/mcp-registry.yaml is not valid YAML",
                format!("{e}"),
            ));
        }
    };

    Ok(yaml_get(&parsed, "mcps")
        .and_then(|mcps| mcps.as_sequence())
        .and_then(|mcps| {
            mcps.iter().find_map(|entry| {
                let name = yaml_get(entry, "name").and_then(|value| value.as_str());
                if name == Some(mcp_name) {
                    yaml_get(entry, "status").and_then(|value| value.as_str())
                } else {
                    None
                }
                .map(|status| status.to_string())
            })
        }))
}

pub(super) fn mcp_registry_adopted_check(
    repo_root: &Path,
    mcp_name: &str,
    display_name: &str,
) -> Finding {
    let check_name = format!("mcp_registry_{mcp_name}_adopted");
    let status = match mcp_registry_entry_status(repo_root, mcp_name) {
        Ok(status) => status,
        Err(finding) => return finding,
    };

    let has_mcp = status.is_some();
    let has_adopted = status.as_deref() == Some("adopted");

    if has_mcp && has_adopted {
        Finding::info(
            check_name,
            format!("{display_name} MCP registered and adopted in mcp-registry.yaml"),
        )
    } else if has_mcp {
        Finding::warn(
            check_name,
            format!("{display_name} MCP found but status is not adopted"),
            format!("Review manifests/mcp-registry.yaml {mcp_name} entry"),
        )
    } else {
        Finding::info(
            check_name,
            format!("{display_name} MCP not registered in mcp-registry.yaml"),
        )
    }
}

/// Check that `manifests/mcp-registry.yaml` has a `codegraph` MCP entry with
/// `status: adopted`. This is an **info** check — host verification checks
/// enforce actual Claude Code registration.
pub fn mcp_registry_codegraph_adopted(repo_root: &Path) -> Finding {
    mcp_registry_adopted_check(repo_root, "codegraph", "CodeGraph")
}
