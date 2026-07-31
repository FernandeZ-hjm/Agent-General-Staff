use super::*;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct CargoManifest {
    package: Option<CargoPackage>,
    workspace: Option<CargoWorkspace>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: Option<String>,
    version: Option<InheritedField>,
    license: Option<InheritedField>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum InheritedField {
    Exact(String),
    Workspace { workspace: bool },
}

#[derive(Deserialize)]
struct CargoWorkspace {
    package: Option<ProductMetadata>,
}

#[derive(Deserialize)]
struct ProductMetadata {
    version: Option<String>,
    license: Option<String>,
}

#[derive(Deserialize)]
struct SuiteManifest {
    suite: Option<SuiteMetadata>,
}

#[derive(Deserialize)]
struct SuiteMetadata {
    version: Option<String>,
    #[serde(default)]
    required: Vec<SuiteComponent>,
}

#[derive(Deserialize)]
struct SuiteComponent {
    name: Option<String>,
    version: Option<String>,
    source: Option<String>,
}

const PRODUCT_PACKAGES: &[&str] = &[
    "ags-platform",
    "ags-workspace-facts",
    "ags-host-integration",
    "ags-capability-governance",
    "ags-task-contract",
    "ags-governance-decision",
    "ags-session",
    "ags-evidence",
    "ags-verification",
    "ags-lifecycle",
    "ags-cli",
    "ags-mcp",
];

#[derive(Deserialize)]
struct McpRegistry {
    #[serde(default)]
    suite_interfaces: Vec<SuiteInterface>,
}

#[derive(Deserialize)]
struct SuiteInterface {
    name: Option<String>,
    package: Option<ProductMetadata>,
}

fn require_exact_field(
    errors: &mut Vec<String>,
    surface: &str,
    actual: Option<&str>,
    expected: &str,
) {
    if actual != Some(expected) {
        errors.push(format!(
            "{surface} must be {expected}, found {}",
            actual.unwrap_or("<missing>")
        ));
    }
}

fn check_typed_product_metadata(
    repo_root: &Path,
    version: &str,
    license: &str,
    errors: &mut Vec<String>,
) {
    let cargo_path = repo_root.join("Cargo.toml");
    match std::fs::read_to_string(&cargo_path) {
        Ok(content) => match toml::from_str::<CargoManifest>(&content) {
            Ok(manifest) => {
                let package = manifest
                    .workspace
                    .as_ref()
                    .and_then(|workspace| workspace.package.as_ref());
                require_exact_field(
                    errors,
                    "Cargo.toml workspace.package.version",
                    package.and_then(|metadata| metadata.version.as_deref()),
                    version,
                );
                require_exact_field(
                    errors,
                    "Cargo.toml workspace.package.license",
                    package.and_then(|metadata| metadata.license.as_deref()),
                    license,
                );
            }
            Err(error) => errors.push(format!("Cargo.toml is invalid TOML: {error}")),
        },
        Err(_) => errors.push("Cargo.toml is missing or unreadable".to_string()),
    }

    for package_name in PRODUCT_PACKAGES {
        let path = repo_root
            .join("crates")
            .join(package_name)
            .join("Cargo.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<CargoManifest>(&content) {
                Ok(manifest) => {
                    let package = manifest.package.as_ref();
                    require_exact_field(
                        errors,
                        &format!("crates/{package_name}/Cargo.toml package.name"),
                        package.and_then(|metadata| metadata.name.as_deref()),
                        package_name,
                    );
                    require_inherited_product_field(
                        errors,
                        &format!("crates/{package_name}/Cargo.toml package.version"),
                        package.and_then(|metadata| metadata.version.as_ref()),
                        version,
                    );
                    require_inherited_product_field(
                        errors,
                        &format!("crates/{package_name}/Cargo.toml package.license"),
                        package.and_then(|metadata| metadata.license.as_ref()),
                        license,
                    );
                }
                Err(error) => errors.push(format!(
                    "crates/{package_name}/Cargo.toml is invalid TOML: {error}"
                )),
            },
            Err(_) => errors.push(format!(
                "crates/{package_name}/Cargo.toml is missing or unreadable"
            )),
        }
    }

    let suite_path = repo_root.join("manifests/suite.yaml");
    match std::fs::read_to_string(&suite_path) {
        Ok(content) => match serde_yaml::from_str::<SuiteManifest>(&content) {
            Ok(manifest) => {
                let suite = manifest.suite.as_ref();
                require_exact_field(
                    errors,
                    "manifests/suite.yaml suite.version",
                    suite.and_then(|suite| suite.version.as_deref()),
                    version,
                );
                if let Some(suite) = suite {
                    for component in &suite.required {
                        if component
                            .source
                            .as_deref()
                            .is_some_and(|source| source.starts_with("global-skills/ags-"))
                        {
                            let name = component.name.as_deref().unwrap_or("<missing>");
                            require_exact_field(
                                errors,
                                &format!(
                                    "manifests/suite.yaml suite.required[name={name}].version"
                                ),
                                component.version.as_deref(),
                                version,
                            );
                        }
                    }
                }
            }
            Err(error) => errors.push(format!("manifests/suite.yaml is invalid YAML: {error}")),
        },
        Err(_) => errors.push("manifests/suite.yaml is missing or unreadable".to_string()),
    }

    let registry_path = repo_root.join("manifests/mcp-registry.yaml");
    match std::fs::read_to_string(&registry_path) {
        Ok(content) => match serde_yaml::from_str::<McpRegistry>(&content) {
            Ok(registry) => {
                let mut matching = registry
                    .suite_interfaces
                    .iter()
                    .filter(|interface| interface.name.as_deref() == Some("ags"));
                let interface = matching.next();
                if matching.next().is_some() {
                    errors.push(
                        "manifests/mcp-registry.yaml suite_interfaces[name=ags] must be unique"
                            .to_string(),
                    );
                }
                let package = interface.and_then(|interface| interface.package.as_ref());
                require_exact_field(
                    errors,
                    "manifests/mcp-registry.yaml suite_interfaces[name=ags].package.version",
                    package.and_then(|metadata| metadata.version.as_deref()),
                    version,
                );
                require_exact_field(
                    errors,
                    "manifests/mcp-registry.yaml suite_interfaces[name=ags].package.license",
                    package.and_then(|metadata| metadata.license.as_deref()),
                    license,
                );
            }
            Err(error) => errors.push(format!(
                "manifests/mcp-registry.yaml is invalid YAML: {error}"
            )),
        },
        Err(_) => errors.push("manifests/mcp-registry.yaml is missing or unreadable".to_string()),
    }
}

fn require_inherited_product_field(
    errors: &mut Vec<String>,
    surface: &str,
    actual: Option<&InheritedField>,
    expected: &str,
) {
    match actual {
        Some(InheritedField::Exact(value)) if value == expected => {}
        Some(InheritedField::Workspace { workspace: true }) => {}
        Some(InheritedField::Exact(value)) => {
            errors.push(format!("{surface} must be {expected}, found {value}"));
        }
        Some(InheritedField::Workspace { workspace: false }) => {
            errors.push(format!("{surface} must inherit workspace metadata"));
        }
        None => errors.push(format!("{surface} is missing")),
    }
}

pub(super) fn check_release_version_surfaces(repo_root: &Path) -> CheckItem {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const LICENSE: &str = "GPL-3.0-only";

    let mut errors = Vec::new();
    let package_path = repo_root.join("packages/ags-mcp/package.json");
    match std::fs::read_to_string(&package_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
    {
        Some(package) => {
            if package.get("version").and_then(|value| value.as_str()) != Some(VERSION) {
                errors.push(format!(
                    "packages/ags-mcp/package.json version must be {VERSION}"
                ));
            }
            if package.get("license").and_then(|value| value.as_str()) != Some(LICENSE) {
                errors.push(format!(
                    "packages/ags-mcp/package.json license must be {LICENSE}"
                ));
            }
        }
        None => errors.push("packages/ags-mcp/package.json is missing or invalid JSON".to_string()),
    }

    check_typed_product_metadata(repo_root, VERSION, LICENSE, &mut errors);

    for (relative, marker) in [
        (
            "crates/ags-mcp/src/protocol.rs",
            "pub const SERVER_VERSION: &str = env!(\"CARGO_PKG_VERSION\");",
        ),
        (
            "crates/ags-mcp/src/server.rs",
            "version: SERVER_VERSION.to_string()",
        ),
    ] {
        match std::fs::read_to_string(repo_root.join(relative)) {
            Ok(content) if content.contains(marker) => {}
            Ok(_) => errors.push(format!(
                "{relative} must derive MCP serverInfo.version from the product package version"
            )),
            Err(_) => errors.push(format!("{relative} is missing or unreadable")),
        }
    }

    let supported_series = VERSION
        .rsplit_once('.')
        .map(|(series, _)| format!("| {series}.x | Yes |"))
        .unwrap_or_else(|| format!("| {VERSION} | Yes |"));
    let required_text = [
        (
            "AGENT_SUITE_PROTOCOL.md",
            format!("Current product version: **{VERSION}**."),
        ),
        ("RELEASE_NOTES.md", format!("## Release {VERSION}")),
        (
            "WORKSPACE.md",
            format!("Current source candidate: **v{VERSION}**."),
        ),
        ("README.md", format!("当前源码发布候选是 **v{VERSION}**")),
        (
            "README_EN.md",
            format!("current source candidate is **v{VERSION}**"),
        ),
        (
            "docs/architecture.md",
            format!("# AGS v{VERSION} Architecture"),
        ),
        ("packages/ags-mcp/README.md", format!("`v{VERSION}` GitHub")),
        ("SECURITY.md", supported_series),
        (
            "SECURITY.md",
            "AGS v0.3.6 hashes the complete running MCP executable".to_string(),
        ),
        ("protocol/mcp-server.md", format!("AGS {VERSION} MCP")),
    ];
    for (relative, marker) in required_text {
        match std::fs::read_to_string(repo_root.join(relative)) {
            Ok(content) if content.contains(&marker) => {}
            Ok(_) => errors.push(format!("{relative} is missing marker: {marker}")),
            Err(_) => errors.push(format!("{relative} is missing or unreadable")),
        }
    }

    // Command skill bodies are private/stable installation surfaces. Public
    // source trees deliberately omit and forbid `global-skills/`; their absence
    // must not make the public release version gate impossible. When the
    // directory exists, however, every AGS-owned command skill must stay aligned.
    if repo_root.join("global-skills").is_dir() {
        for relative in [
            "global-skills/ags-agents/SKILL.md",
            "global-skills/ags-doctor/SKILL.md",
            "global-skills/ags-init/SKILL.md",
            "global-skills/ags-setup/SKILL.md",
            "global-skills/ags-skill/SKILL.md",
        ] {
            let marker = format!("AGS 产品版本：{VERSION}");
            match std::fs::read_to_string(repo_root.join(relative)) {
                Ok(content) if content.contains(&marker) => {}
                Ok(_) => errors.push(format!("{relative} is missing marker: {marker}")),
                Err(_) => errors.push(format!("{relative} is missing or unreadable")),
            }
        }

        let setup_skill = "global-skills/ags-setup/SKILL.md";
        if let Ok(content) = std::fs::read_to_string(repo_root.join(setup_skill)) {
            if content.contains("--with-evomap") {
                errors.push(format!(
                    "{setup_skill} still references retired flag --with-evomap"
                ));
            }
            if !content.contains("ags setup --yes --force") {
                errors.push(format!(
                    "{setup_skill} is missing current command: ags setup --yes --force"
                ));
            }
        }
    }

    if !repo_root.join("LICENSE").is_file() {
        errors.push("root GPL LICENSE is missing".to_string());
    }
    if !repo_root.join("packages/ags-mcp/LICENSE").is_file() {
        errors.push("npm launcher GPL LICENSE is missing".to_string());
    }

    // The current release owns one current schema set. Removed compatibility
    // protocols are not retained as hidden interfaces.
    for (relative, marker) in [
        (
            "crates/ags-governance-decision/src/lib.rs",
            "0.3.6-host-route-proposal",
        ),
        (
            "crates/ags-governance-decision/src/lib.rs",
            "0.3.6-route-resolution",
        ),
        (
            "crates/ags-task-contract/src/intent.rs",
            "0.3.6-handoff-contract",
        ),
        (
            "crates/ags-task-contract/src/intent.rs",
            "0.3.6-task-contract",
        ),
        (
            "crates/ags-task-contract/src/runner.rs",
            "0.3.6-launch-plan",
        ),
        (
            "crates/ags-lifecycle/src/onboarding/mod.rs",
            "0.3.6-onboarding-plan",
        ),
        (
            "crates/ags-lifecycle/src/init/model.rs",
            "0.4.1-project-init",
        ),
        (
            "crates/ags-lifecycle/src/setup/mod.rs",
            "0.4.1-private-install",
        ),
        (
            "crates/ags-lifecycle/src/workspace_lifecycle.rs",
            "0.4.0-workspace-lifecycle",
        ),
        (
            "crates/ags-lifecycle/src/workspace_lifecycle.rs",
            "0.4.0-closure-pointer",
        ),
        (
            "crates/ags-lifecycle/src/lifecycle_projection.rs",
            "0.4.0-workspace-lifecycle-manifest",
        ),
        (
            "crates/ags-capability-governance/src/authority.rs",
            "0.3.6-host-capability-snapshot",
        ),
        (
            "crates/ags-capability-governance/src/skill_body/console/model.rs",
            "0.3.6-skill-console",
        ),
        (
            "crates/ags-capability-governance/src/skill_body/model.rs",
            "0.3.6-skill-inventory",
        ),
        (
            "crates/ags-governance-decision/src/policy/model.rs",
            "0.3.6-execution-policy",
        ),
        (
            "crates/ags-verification/src/orchestrator.rs",
            "0.3.6-verification-report",
        ),
        (
            "crates/ags-verification/src/bootstrap.rs",
            "0.3.6-bootstrap-plan",
        ),
        (
            "crates/ags-verification/src/release_package.rs",
            "0.4.0-release-plan",
        ),
        (
            "crates/ags-verification/src/release_package.rs",
            "0.4.0-runtime-stage",
        ),
        (
            "crates/ags-session/src/workspace_service.rs",
            "0.4.0-workspace-daemon-status",
        ),
        (
            "crates/ags-session/src/workspace_service/upgrade_recycle.rs",
            "0.4.0-workspace-service-status",
        ),
        ("crates/ags-mcp/src/protocol.rs", "2024-11-05"),
        (
            "crates/ags-evidence/src/receipt_model.rs",
            "0.3.6-task-receipt",
        ),
        (
            "crates/ags-evidence/src/delivery_report.rs",
            "0.3.6-delivery-closure",
        ),
        ("crates/ags-evidence/src/action.rs", "0.3.6-action-receipt"),
    ] {
        match std::fs::read_to_string(repo_root.join(relative)) {
            Ok(content) if content.contains(marker) => {}
            Ok(_) => errors.push(format!(
                "{relative} is missing current schema marker {marker}"
            )),
            Err(_) => errors.push(format!("{relative} is missing or unreadable")),
        }
    }

    let registry_source = "crates/ags-session/src/workspace_service/registry_ownership.rs";
    match std::fs::read_to_string(repo_root.join(registry_source)) {
        Ok(content) if content.contains("ags-workspace-registry/1") => {}
        Ok(_) => errors.push(format!(
            "{registry_source} is missing stable protocol marker ags-workspace-registry/1"
        )),
        Err(_) => errors.push(format!("{registry_source} is missing or unreadable")),
    }
    let wire_source = "crates/ags-session/src/workspace_service/transport_handshake.rs";
    match std::fs::read_to_string(repo_root.join(wire_source)) {
        Ok(content) if content.contains("ags-workspace-service/2") => {}
        Ok(_) => errors.push(format!(
            "{wire_source} is missing current protocol marker ags-workspace-service/2"
        )),
        Err(_) => errors.push(format!("{wire_source} is missing or unreadable")),
    }

    if errors.is_empty() {
        CheckItem::pass(
            "release-version-surfaces",
            "release",
            &format!("Product version {VERSION} and {LICENSE} license surfaces are aligned."),
        )
    } else {
        CheckItem::fail(
            "release-version-surfaces",
            "release",
            &errors.join("; "),
            "Align product, license, documentation, and current schema surfaces.",
        )
    }
}
