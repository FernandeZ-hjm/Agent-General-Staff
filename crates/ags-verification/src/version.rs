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
    "ags-control-plane",
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

pub(super) fn check_npm_product_metadata(
    repo_root: &Path,
    version: &str,
    license: &str,
) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct NpmProductMetadata {
        version: String,
        license: String,
    }

    let mut errors = Vec::new();
    for relative in [
        "packages/ags-cli/package.json",
        "packages/ags-launcher/package.json",
        "packages/ags-mcp/package.json",
    ] {
        match std::fs::read_to_string(repo_root.join(relative))
            .ok()
            .and_then(|content| serde_json::from_str::<NpmProductMetadata>(&content).ok())
        {
            Some(package) => {
                require_exact_field(
                    &mut errors,
                    &format!("{relative} version"),
                    Some(&package.version),
                    version,
                );
                require_exact_field(
                    &mut errors,
                    &format!("{relative} license"),
                    Some(&package.license),
                    license,
                );
            }
            None => errors.push(format!("{relative} is missing or invalid typed metadata")),
        }
    }
    errors
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

pub(super) fn check_public_ci_release_invocation(repo_root: &Path) -> Vec<String> {
    let path = repo_root.join(".github/workflows/ci.yml");
    if !path.is_file() {
        return Vec::new();
    }

    let stale_binary = "./target/release/ags check release";
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let mut errors = Vec::new();
            let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
            for required in [
                "cargo run -q --locked -p ags-cli -- check release",
                "--workspace .",
                "--format json",
                "check bundle create",
                "check bundle validate",
                "--source public-full",
            ] {
                if !compact.contains(required) {
                    errors.push(format!(
                        ".github/workflows/ci.yml must preserve the exact-input public gate marker: {required}"
                    ));
                }
            }
            if content.contains(stale_binary) {
                errors.push(
                    ".github/workflows/ci.yml must not execute a cached target/release/ags for the release gate"
                        .to_string(),
                );
            }
            errors
        }
        Err(_) => vec![".github/workflows/ci.yml is unreadable".to_string()],
    }
}

pub(super) fn check_release_version_surfaces(repo_root: &Path) -> CheckItem {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const LICENSE: &str = "GPL-3.0-only";

    let mut errors = Vec::new();
    errors.extend(check_npm_product_metadata(repo_root, VERSION, LICENSE));

    check_typed_product_metadata(repo_root, VERSION, LICENSE, &mut errors);
    errors.extend(check_public_ci_release_invocation(repo_root));

    for (relative, marker) in [(
        "crates/ags-mcp/src/contract_v2.rs",
        "\"version\": env!(\"CARGO_PKG_VERSION\")",
    )] {
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
        ("SECURITY.md", supported_series),
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
            "global-skills/ags-agent/SKILL.md",
            "global-skills/ags-doctor/SKILL.md",
            "global-skills/ags-init/SKILL.md",
            "global-skills/ags-setup/SKILL.md",
            "global-skills/ags-govern/SKILL.md",
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
            if !content.lines().any(|line| line.trim() == "ags setup") {
                errors.push(format!(
                    "{setup_skill} is missing the contract v2 `ags setup` command"
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

    // Contract v2 surfaces are release-gated together. Legacy contracts are a
    // hard cut and are checked separately by the source-absence gate.
    for (relative, marker) in [
        ("protocol/mcp-server.md", "# AGS MCP contract v2"),
        (
            "protocol/task-routing.md",
            "# AGS contract v2 Operation routing",
        ),
        (
            "protocol/runtime-adapters.md",
            "# Runtime adapters — contract v2",
        ),
        (
            "protocol/agent-task-protocol.md",
            "# Agent task protocol — contract v2",
        ),
        (
            "crates/ags-verification/src/orchestrator.rs",
            "ags://schema/contract/v2/check-report",
        ),
        (
            "crates/ags-verification/src/test_execution.rs",
            "ags://schema/contract/v2/test-receipt",
        ),
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
