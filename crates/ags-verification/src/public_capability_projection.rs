//! Typed, deterministic projection of public capability declarations.
//!
//! The catalog is discovery metadata. It never becomes machine installation or
//! routing state. Bundled Skill routing is projected from the canonical source
//! registry; the public projector never owns a second routing table.

use ags_capability_governance::third_party_manifest::{
    read_third_party_manifest, CapabilityKind, ThirdPartyCapability,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const PUBLIC_CAPABILITY_PROJECTION_PATH: &str = "manifests/public-capability-projection.yaml";
pub const PUBLIC_CAPABILITY_PROJECTION_SCHEMA: &str = "1.0";
pub const PUBLIC_CAPABILITY_GENERATED_FILES: &[&str] = &[
    "manifests/mcp-registry.yaml",
    "manifests/skills-registry.yaml",
    "manifests/suite.yaml",
    "templates/command-skills/ags-agents/SKILL.md",
    "templates/command-skills/ags-doctor/SKILL.md",
    "templates/command-skills/ags-init/SKILL.md",
    "templates/command-skills/ags-setup/SKILL.md",
    "templates/command-skills/ags-skill/SKILL.md",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionSpec {
    schema_version: String,
    product: ProductSpec,
    generated_files: Vec<String>,
    bundled_skills: Vec<BundledSkillSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductSpec {
    name: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundledSkillSpec {
    id: String,
    source_path: String,
    public_path: String,
    public_sha256: String,
}

#[derive(Debug, Deserialize)]
struct SourceSuiteDocument {
    suite: SourceSuite,
}

#[derive(Debug, Deserialize)]
struct SourceSuite {
    #[serde(default)]
    required: Vec<SourceSuiteSkill>,
}

#[derive(Debug, Deserialize)]
struct SourceSuiteSkill {
    name: String,
    source: String,
}

#[derive(Debug, Deserialize)]
struct BundledRegistryContract {
    name: String,
    profile: String,
    local_path: String,
    routing: BundledRoutingContract,
}

#[derive(Debug, Deserialize)]
struct BundledRoutingContract {
    route_state: String,
    routing_surface: String,
    invoke_hint: String,
}

#[derive(Debug, Serialize)]
struct PublicSuiteDocument {
    schema_version: &'static str,
    suite: PublicSuite,
}

#[derive(Debug, Serialize)]
struct PublicSuite {
    name: String,
    version: String,
    description: String,
    required: Vec<PublicBundledSkill>,
    optional: Vec<PublicCatalogSkill>,
    personal: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize)]
struct PublicBundledSkill {
    name: String,
    version: String,
    source: String,
    hash: String,
}

#[derive(Debug, Serialize)]
struct PublicCatalogSkill {
    name: String,
    version: String,
    source: String,
    description: String,
    repository: String,
    subdirectory: String,
    resolved_commit: String,
    body_hash: String,
    license: String,
    catalog_review_status: &'static str,
    install_state: &'static str,
    route_state: &'static str,
}

#[derive(Debug, Serialize)]
struct PublicRegistryDocument {
    registry: PublicRegistryMetadata,
    skills: Vec<serde_yaml::Value>,
}

#[derive(Debug, Serialize)]
struct PublicRegistryMetadata {
    schema_version: &'static str,
    description: &'static str,
    catalog_manifest: &'static str,
    installation_authority: &'static str,
    activation_authority: &'static str,
}

#[derive(Debug, Serialize)]
struct PublicMcpRegistryDocument {
    schema_version: &'static str,
    registry: PublicRegistryMetadata,
    suite_interfaces: Vec<PublicSuiteInterface>,
    mcps: Vec<serde_yaml::Value>,
}

#[derive(Debug, Serialize)]
struct PublicSuiteInterface {
    name: &'static str,
    role: &'static str,
    governed: bool,
    package: PublicSuitePackage,
    install: PublicSuiteInstall,
}

#[derive(Debug, Serialize)]
struct PublicSuitePackage {
    manager: &'static str,
    name: &'static str,
    version: String,
    repository: &'static str,
    license: String,
}

#[derive(Debug, Serialize)]
struct PublicSuiteInstall {
    transport: &'static str,
    command: &'static str,
    args: [&'static str; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicProjectionFile {
    pub path: String,
    pub content_sha256: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCapabilityProjectionPlan {
    pub schema_version: String,
    pub source_root: PathBuf,
    pub target_root: PathBuf,
    pub plan_hash: String,
    pub generated_files: Vec<PublicProjectionFile>,
    pub bundled_skill_ids: Vec<String>,
    pub catalog_skill_ids: Vec<String>,
    pub blocking_findings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCapabilityProjectionReceipt {
    pub schema_version: String,
    pub plan_hash: String,
    pub target_root: PathBuf,
    pub written_files: Vec<String>,
}

struct RenderedProjection {
    plan: PublicCapabilityProjectionPlan,
    contents: BTreeMap<String, Vec<u8>>,
}

pub fn plan_public_capability_projection(
    source_root: &Path,
    target_root: &Path,
) -> PublicCapabilityProjectionPlan {
    render_projection(source_root, target_root).plan
}

pub fn apply_public_capability_projection(
    source_root: &Path,
    target_root: &Path,
    approved_plan_hash: &str,
) -> Result<PublicCapabilityProjectionReceipt, String> {
    let rendered = render_projection(source_root, target_root);
    if !rendered.plan.blocking_findings.is_empty() {
        return Err(format!(
            "public capability projection is blocked: {}",
            rendered.plan.blocking_findings.join("; ")
        ));
    }
    if approved_plan_hash != rendered.plan.plan_hash {
        return Err("public capability projection plan_hash changed; re-plan and approve".into());
    }

    let mut previous = Vec::new();
    let mut written = Vec::new();
    for (relative, bytes) in &rendered.contents {
        let destination = target_root.join(relative);
        previous.push((destination.clone(), fs::read(&destination).ok()));
        if let Err(error) = ags_platform::atomic_write(&destination, bytes) {
            for (path, old) in previous.into_iter().rev() {
                let recovery = match old {
                    Some(old) => ags_platform::atomic_write(&path, &old),
                    None => fs::remove_file(&path).map_err(|remove_error| remove_error.to_string()),
                };
                if recovery.is_err() {
                    return Err(format!(
                        "projection failed ({error}) and recovery failed for {}",
                        path.display()
                    ));
                }
            }
            return Err(format!("cannot write {}: {error}", destination.display()));
        }
        written.push(relative.clone());
    }

    Ok(PublicCapabilityProjectionReceipt {
        schema_version: "1.0-public-capability-projection-receipt".into(),
        plan_hash: rendered.plan.plan_hash,
        target_root: target_root.to_path_buf(),
        written_files: written,
    })
}

pub fn verify_public_capability_projection(source_root: &Path, target_root: &Path) -> Vec<String> {
    let rendered = render_projection(source_root, target_root);
    let mut errors = rendered.plan.blocking_findings;
    for (path, expected) in rendered.contents {
        match fs::read(target_root.join(&path)) {
            Ok(actual) if actual == expected => {}
            Ok(_) => errors.push(format!("generated public capability file differs: {path}")),
            Err(error) => errors.push(format!("cannot read generated file {path}: {error}")),
        }
    }
    errors.sort();
    errors.dedup();
    errors
}

fn render_projection(source_root: &Path, target_root: &Path) -> RenderedProjection {
    let mut blocking = Vec::new();
    let enforce_source_binding = source_root != target_root;
    let spec = load_spec(source_root, &mut blocking);
    let product = workspace_product(source_root, &mut blocking);
    let source_suite = enforce_source_binding
        .then(|| load_source_suite(source_root, &mut blocking))
        .flatten();
    let catalog = read_third_party_manifest(source_root).map_err(|error| {
        blocking.push(error);
    });

    let mut contents = BTreeMap::new();
    let mut bundled_ids = Vec::new();
    let mut catalog_ids = Vec::new();

    if let (Some(spec), Some(product), Ok(catalog)) = (spec.as_ref(), product.as_ref(), catalog) {
        let version = &product.version;
        validate_generated_paths(spec, &mut blocking);
        let bundled = render_bundled_skills(
            BundledRenderContext {
                spec,
                version,
                source_suite: source_suite.as_ref(),
                source_root,
                target_root,
                enforce_source_binding,
            },
            &mut contents,
            &mut blocking,
        );
        bundled_ids = bundled.iter().map(|skill| skill.name.clone()).collect();
        let recommendations = render_catalog_skills(&catalog.capabilities, &mut blocking);
        catalog_ids = recommendations
            .iter()
            .map(|skill| skill.name.clone())
            .collect();

        let suite = PublicSuiteDocument {
            schema_version: "2.0-skill",
            suite: PublicSuite {
                name: spec.product.name.clone(),
                version: version.clone(),
                description: spec.product.description.trim().to_string(),
                required: bundled,
                optional: recommendations,
                personal: BTreeMap::new(),
            },
        };
        let bundled_routes = render_bundled_registry(
            source_root,
            target_root,
            spec,
            enforce_source_binding,
            &mut blocking,
        );
        let registry = PublicRegistryDocument {
            registry: PublicRegistryMetadata {
                schema_version: "2.0-layered-projection",
                description: "Static routing contains activated bodies only; recommendations are never installed or routable by declaration.",
                catalog_manifest: "manifests/third-party-capabilities.yaml",
                installation_authority: "machine-local InstalledSkillRecord",
                activation_authority: "machine-local ActivatedCapability after exact route verification",
            },
            skills: bundled_routes,
        };
        let mcp_registry = PublicMcpRegistryDocument {
            schema_version: "2.0-layered-projection",
            registry: PublicRegistryMetadata {
                schema_version: "2.0-layered-projection",
                description: "The suite interface is bundled. Third-party MCP catalog entries are discovery metadata and machine activation is observed from Host state.",
                catalog_manifest: "manifests/third-party-capabilities.yaml",
                installation_authority: "machine-local Host MCP configuration",
                activation_authority: "machine-local capability snapshot after live Host verification",
            },
            suite_interfaces: vec![PublicSuiteInterface {
                name: "ags",
                role: "host-initialization-adapter",
                governed: false,
                package: PublicSuitePackage {
                    manager: "cargo",
                    name: "ags-mcp",
                    version: version.clone(),
                    repository: "this-repository",
                    license: product.license.clone(),
                },
                install: PublicSuiteInstall {
                    transport: "stdio",
                    command: "ags",
                    args: ["mcp", "serve", "--transport", "stdio"],
                },
            }],
            mcps: Vec::new(),
        };
        insert_yaml(
            &mut contents,
            "manifests/mcp-registry.yaml",
            &mcp_registry,
            &mut blocking,
        );
        insert_yaml(&mut contents, "manifests/suite.yaml", &suite, &mut blocking);
        insert_yaml(
            &mut contents,
            "manifests/skills-registry.yaml",
            &registry,
            &mut blocking,
        );
        let declared = spec
            .generated_files
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let rendered = contents.keys().cloned().collect::<BTreeSet<_>>();
        if declared != rendered || spec.generated_files.len() != rendered.len() {
            blocking.push(format!(
                "typed capability projection did not render its exact declared output set: declared={declared:?}, rendered={rendered:?}"
            ));
        }
    }

    let generated_files = contents
        .iter()
        .map(|(path, bytes)| PublicProjectionFile {
            path: path.clone(),
            content_sha256: ags_platform::sha256(bytes),
            changed: fs::read(target_root.join(path)).ok().as_deref() != Some(bytes.as_slice()),
        })
        .collect::<Vec<_>>();
    blocking.sort();
    blocking.dedup();
    let plan_hash = projection_plan_hash(
        source_root,
        target_root,
        &generated_files,
        &bundled_ids,
        &catalog_ids,
        &blocking,
    );
    RenderedProjection {
        plan: PublicCapabilityProjectionPlan {
            schema_version: "1.0-public-capability-projection-plan".into(),
            source_root: source_root.to_path_buf(),
            target_root: target_root.to_path_buf(),
            plan_hash,
            generated_files,
            bundled_skill_ids: bundled_ids,
            catalog_skill_ids: catalog_ids,
            blocking_findings: blocking,
        },
        contents,
    }
}

fn load_spec(source_root: &Path, errors: &mut Vec<String>) -> Option<ProjectionSpec> {
    let path = source_root.join(PUBLIC_CAPABILITY_PROJECTION_PATH);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            errors.push(format!("cannot read {}: {error}", path.display()));
            return None;
        }
    };
    match serde_yaml::from_str::<ProjectionSpec>(&content) {
        Ok(spec) if spec.schema_version == PUBLIC_CAPABILITY_PROJECTION_SCHEMA => Some(spec),
        Ok(spec) => {
            errors.push(format!(
                "unsupported public capability projection schema: {}",
                spec.schema_version
            ));
            None
        }
        Err(error) => {
            errors.push(format!("cannot parse {}: {error}", path.display()));
            None
        }
    }
}

struct WorkspaceProduct {
    version: String,
    license: String,
}

fn workspace_product(source_root: &Path, errors: &mut Vec<String>) -> Option<WorkspaceProduct> {
    let path = source_root.join("Cargo.toml");
    let value = match fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|content| {
            toml::from_str::<toml::Value>(&content).map_err(|error| error.to_string())
        }) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!("cannot read workspace version: {error}"));
            return None;
        }
    };
    let package = value
        .get("workspace")
        .and_then(|value| value.get("package"))
        .and_then(toml::Value::as_table);
    let version = package
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str);
    let license = package
        .and_then(|package| package.get("license"))
        .and_then(toml::Value::as_str);
    match (version, license) {
        (Some(version), Some(license)) => Some(WorkspaceProduct {
            version: version.to_string(),
            license: license.to_string(),
        }),
        _ => {
            errors.push("Cargo.toml workspace.package version/license is missing".into());
            None
        }
    }
}

fn render_bundled_registry(
    source_root: &Path,
    target_root: &Path,
    spec: &ProjectionSpec,
    enforce_source_binding: bool,
    errors: &mut Vec<String>,
) -> Vec<serde_yaml::Value> {
    let registry_root = if enforce_source_binding {
        source_root
    } else {
        target_root
    };
    let path = registry_root.join("manifests/skills-registry.yaml");
    let document = match fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|content| {
            serde_yaml::from_str::<serde_yaml::Value>(&content).map_err(|error| error.to_string())
        }) {
        Ok(document) => document,
        Err(error) => {
            errors.push(format!(
                "cannot load canonical bundled Skill registry {}: {error}",
                path.display()
            ));
            return Vec::new();
        }
    };
    let Some(entries) = document
        .get("skills")
        .and_then(serde_yaml::Value::as_sequence)
    else {
        errors.push(format!(
            "canonical bundled Skill registry has no skills sequence: {}",
            path.display()
        ));
        return Vec::new();
    };

    let mut projected = Vec::new();
    for bundled in &spec.bundled_skills {
        let matches = entries
            .iter()
            .filter(|entry| {
                entry.get("name").and_then(serde_yaml::Value::as_str) == Some(bundled.id.as_str())
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            errors.push(format!(
                "bundled Skill {} must have exactly one canonical registry entry, observed {}",
                bundled.id,
                matches.len()
            ));
            continue;
        }
        let entry = matches[0].clone();
        let contract = match serde_yaml::from_value::<BundledRegistryContract>(entry.clone()) {
            Ok(contract) => contract,
            Err(error) => {
                errors.push(format!(
                    "bundled Skill {} has an invalid canonical routing contract: {error}",
                    bundled.id
                ));
                continue;
            }
        };
        let expected_source = if enforce_source_binding {
            bundled.source_path.as_str()
        } else {
            bundled.public_path.as_str()
        };
        if contract.name != bundled.id
            || contract.profile != "required"
            || contract.local_path != expected_source
        {
            errors.push(format!(
                "bundled Skill {} registry identity/profile/path drift: name={} profile={} path={} expected_path={expected_source}",
                bundled.id, contract.name, contract.profile, contract.local_path
            ));
        }
        let expected_routing = if bundled.id == "ags-skill" {
            ("routable", "skill_target")
        } else {
            ("not-routable", "host_command")
        };
        if contract.routing.route_state != expected_routing.0
            || contract.routing.routing_surface != expected_routing.1
            || contract.routing.invoke_hint.trim().is_empty()
        {
            errors.push(format!(
                "bundled Skill {} routing contract drift: state={} surface={} hint_present={}",
                bundled.id,
                contract.routing.route_state,
                contract.routing.routing_surface,
                !contract.routing.invoke_hint.trim().is_empty()
            ));
        }

        let mut projected_entry = entry;
        let Some(mapping) = projected_entry.as_mapping_mut() else {
            errors.push(format!(
                "bundled Skill {} registry entry is not a mapping",
                bundled.id
            ));
            continue;
        };
        mapping.insert(
            serde_yaml::Value::String("local_path".to_string()),
            serde_yaml::Value::String(bundled.public_path.clone()),
        );
        projected.push(projected_entry);
    }
    projected
}

fn load_source_suite(source_root: &Path, errors: &mut Vec<String>) -> Option<SourceSuite> {
    let path = source_root.join("manifests/suite.yaml");
    match fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|content| {
            serde_yaml::from_str::<SourceSuiteDocument>(&content).map_err(|error| error.to_string())
        }) {
        Ok(document) => Some(document.suite),
        Err(error) => {
            errors.push(format!("cannot load source suite manifest: {error}"));
            None
        }
    }
}

fn validate_generated_paths(spec: &ProjectionSpec, errors: &mut Vec<String>) {
    let expected = PUBLIC_CAPABILITY_GENERATED_FILES
        .iter()
        .map(|path| (*path).to_string())
        .collect::<BTreeSet<_>>();
    let actual = spec
        .generated_files
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual != expected || spec.generated_files.len() != expected.len() {
        errors.push(format!(
            "generated_files must exactly match the typed public capability outputs: {actual:?}"
        ));
    }
}

struct BundledRenderContext<'a> {
    spec: &'a ProjectionSpec,
    version: &'a str,
    source_suite: Option<&'a SourceSuite>,
    source_root: &'a Path,
    target_root: &'a Path,
    enforce_source_binding: bool,
}

fn render_bundled_skills(
    context: BundledRenderContext<'_>,
    contents: &mut BTreeMap<String, Vec<u8>>,
    errors: &mut Vec<String>,
) -> Vec<PublicBundledSkill> {
    let required = context
        .source_suite
        .into_iter()
        .flat_map(|suite| suite.required.iter())
        .map(|skill| (skill.name.as_str(), skill.source.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut bundled = Vec::new();
    for skill in &context.spec.bundled_skills {
        if !stable_id(&skill.id) || !seen.insert(skill.id.as_str()) {
            errors.push(format!(
                "invalid or duplicate bundled Skill id: {}",
                skill.id
            ));
            continue;
        }
        if context.enforce_source_binding
            && required.get(skill.id.as_str()).copied() != Some(skill.source_path.as_str())
        {
            errors.push(format!(
                "bundled Skill {} is not bound to source suite path {}",
                skill.id, skill.source_path
            ));
        }
        validate_relative(&skill.source_path, "source_path", errors);
        validate_relative(&skill.public_path, "public_path", errors);
        let generated_path = format!("{}/SKILL.md", skill.public_path);
        let authority_path = if context.enforce_source_binding {
            context
                .source_root
                .join(&skill.source_path)
                .join("SKILL.md")
        } else {
            context.target_root.join(&generated_path)
        };
        match fs::read(&authority_path) {
            Ok(body) => {
                let actual = ags_platform::sha256(&body);
                let projected_body_hash =
                    ags_capability_governance::hash_single_file_skill_source("SKILL.md", &body)
                        .unwrap_or_else(|error| {
                            errors.push(format!(
                                "cannot hash projected Skill body for {}: {error}",
                                skill.id
                            ));
                            String::new()
                        });
                if actual != skill.public_sha256 {
                    errors.push(format!(
                        "bundled authority body hash mismatch for {}: expected {}, observed {}",
                        skill.id, skill.public_sha256, actual
                    ));
                } else if contents.insert(generated_path.clone(), body).is_some() {
                    errors.push(format!("duplicate generated public path: {generated_path}"));
                }
                bundled.push(PublicBundledSkill {
                    name: skill.id.clone(),
                    version: context.version.to_string(),
                    source: skill.public_path.clone(),
                    hash: projected_body_hash
                        .trim_start_matches("sha256:")
                        .to_string(),
                });
            }
            Err(error) => {
                errors.push(format!(
                    "bundled authority body is unavailable for {} at {}: {error}",
                    skill.id,
                    authority_path.display()
                ));
                continue;
            }
        }
    }
    bundled
}

fn render_catalog_skills(
    capabilities: &[ThirdPartyCapability],
    errors: &mut Vec<String>,
) -> Vec<PublicCatalogSkill> {
    let mut seen = BTreeSet::new();
    let mut rendered = Vec::new();
    for capability in capabilities.iter().filter(|capability| {
        capability.kind == CapabilityKind::Skill && capability.applies_to("public")
    }) {
        let id = &capability.id;
        if !stable_id(id) || !seen.insert(id.as_str()) {
            errors.push(format!("invalid or duplicate catalog Skill id: {id}"));
            continue;
        }
        let source = &capability.source;
        let repository = source.repository.as_deref().unwrap_or_default();
        let revision = source.revision.as_deref().unwrap_or_default();
        let subdir = source.subdir.as_deref().unwrap_or_default();
        let license = source.license.as_deref().unwrap_or_default();
        let body_hash = source.integrity.as_deref().unwrap_or_default();
        if source.manager != "git"
            || !repository.starts_with("https://github.com/")
            || !ags_platform::is_git_commit(revision)
            || license.trim().is_empty()
            || !ags_platform::is_sha256(body_hash)
        {
            errors.push(format!(
                "catalog Skill {id} requires git GitHub repository, immutable commit, license, and reviewed sha256 body hash"
            ));
        }
        validate_relative(subdir, "catalog subdirectory", errors);
        rendered.push(PublicCatalogSkill {
            name: id.clone(),
            version: revision.to_string(),
            source: format!(
                "{}/tree/{}/{}",
                repository.trim_end_matches('/'),
                revision,
                subdir
            ),
            description: capability.purpose.trim().to_string(),
            repository: repository.to_string(),
            subdirectory: subdir.to_string(),
            resolved_commit: revision.to_string(),
            body_hash: body_hash.to_string(),
            license: license.to_string(),
            catalog_review_status: "reviewed",
            install_state: "not-installed",
            route_state: "not-routable-until-activated",
        });
    }
    rendered
}

fn insert_yaml<T: Serialize>(
    contents: &mut BTreeMap<String, Vec<u8>>,
    path: &str,
    value: &T,
    errors: &mut Vec<String>,
) {
    match serde_yaml::to_string(value) {
        Ok(mut content) => {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            contents.insert(path.to_string(), content.into_bytes());
        }
        Err(error) => errors.push(format!("cannot render {path}: {error}")),
    }
}

fn projection_plan_hash(
    source_root: &Path,
    target_root: &Path,
    files: &[PublicProjectionFile],
    bundled_ids: &[String],
    catalog_ids: &[String],
    blocking: &[String],
) -> String {
    let value = serde_json::json!({
        "schema_version": "1.0-public-capability-projection-plan",
        "source_root": source_root,
        "target_root": target_root,
        "generated_files": files,
        "bundled_skill_ids": bundled_ids,
        "catalog_skill_ids": catalog_ids,
        "blocking_findings": blocking,
    });
    ags_platform::sha256(serde_json::to_vec(&value).expect("projection plan is serializable"))
}

fn validate_relative(path: &str, label: &str, errors: &mut Vec<String>) {
    let candidate = Path::new(path);
    if path.is_empty()
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        errors.push(format!("unsafe {label}: {path}"));
    }
}

fn stable_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn fixture() -> (TempDir, TempDir) {
        let source = TempDir::new().unwrap();
        let target = TempDir::new().unwrap();
        write(
            &source.path().join("Cargo.toml"),
            "[workspace]\n[workspace.package]\nversion = \"0.4.13\"\nlicense = \"GPL-3.0-only\"\n",
        );
        let ids = [
            "ags-agents",
            "ags-doctor",
            "ags-init",
            "ags-setup",
            "ags-skill",
        ];
        for id in ids {
            write(
                &source.path().join(format!("global-skills/{id}/SKILL.md")),
                "---\nname: command\n---\n",
            );
        }
        let public_hash =
            ags_platform::sha256_file(&source.path().join("global-skills/ags-setup/SKILL.md"))
                .unwrap();
        write(
            &source.path().join("manifests/suite.yaml"),
            &format!(
                "suite:\n  required:\n{}",
                ids.iter()
                    .map(|id| format!("    - name: {id}\n      source: global-skills/{id}\n"))
                    .collect::<String>()
            ),
        );
        write(
            &source.path().join("manifests/skills-registry.yaml"),
            &format!(
                "registry:\n  schema_version: test\nskills:\n{}",
                ids.iter()
                    .map(|id| {
                        let (state, surface) = if *id == "ags-skill" {
                            ("routable", "skill_target")
                        } else {
                            ("not-routable", "host_command")
                        };
                        format!(
                            "  - name: {id}\n    profile: required\n    local_path: global-skills/{id}\n    routing:\n      route_state: {state}\n      routing_surface: {surface}\n      invoke_hint: ags {id}\n"
                        )
                    })
                    .collect::<String>()
            ),
        );
        let generated = PUBLIC_CAPABILITY_GENERATED_FILES
            .iter()
            .map(|path| format!("  - {path}\n"))
            .collect::<String>();
        let bundled = ids
            .iter()
            .map(|id| {
                format!(
                    "  - id: {id}\n    source_path: global-skills/{id}\n    public_path: templates/command-skills/{id}\n    public_sha256: \"{public_hash}\"\n"
                )
            })
            .collect::<String>();
        write(
            &source.path().join(PUBLIC_CAPABILITY_PROJECTION_PATH),
            &format!(
                "schema_version: \"1.0\"\nproduct:\n  name: ags\n  description: public\ngenerated_files:\n{generated}bundled_skills:\n{bundled}"
            ),
        );
        write(
            &source
                .path()
                .join("manifests/third-party-capabilities.yaml"),
            "schema_version: \"1.0\"\nprinciple: reviewed\ncapabilities:\n  - id: diagnosing-bugs\n    kind: skill\n    profiles: [public]\n    purpose: diagnose\n    source:\n      manager: git\n      revision: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n      tracking_ref: main\n      integrity: sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n      repository: https://github.com/example/skills\n      license: MIT\n      subdir: skills/diagnosing-bugs\n    install:\n      strategy: external-manager\n    routing:\n      route_state: routable\n      invoke_hint: diagnose\n      intent_tags: [diagnosis]\n      positive_examples: [diagnose a failure]\n      negative_examples: [implement a feature]\n",
        );
        (source, target)
    }

    #[test]
    fn recommendation_is_not_routable_but_bundled_routes_are_projected() {
        let (source, target) = fixture();
        let plan = plan_public_capability_projection(source.path(), target.path());
        assert!(
            plan.blocking_findings.is_empty(),
            "{:?}",
            plan.blocking_findings
        );
        let receipt =
            apply_public_capability_projection(source.path(), target.path(), &plan.plan_hash)
                .unwrap();
        assert_eq!(receipt.written_files.len(), 8);
        let suite = fs::read_to_string(target.path().join("manifests/suite.yaml")).unwrap();
        let registry =
            fs::read_to_string(target.path().join("manifests/skills-registry.yaml")).unwrap();
        assert!(suite.contains("name: diagnosing-bugs"));
        assert!(suite.contains("install_state: not-installed"));
        assert!(registry.contains("name: ags-setup"));
        assert!(registry.contains("routing_surface: host_command"));
        assert!(registry.contains("name: ags-skill"));
        assert!(registry.contains("routing_surface: skill_target"));
        assert!(!registry.contains("name: diagnosing-bugs"));
        let mcp_registry =
            fs::read_to_string(target.path().join("manifests/mcp-registry.yaml")).unwrap();
        assert!(mcp_registry.contains("name: ags"));
        assert!(mcp_registry.contains("mcps: []"));
        assert!(!mcp_registry.contains("name: codegraph"));
        assert_eq!(
            fs::read(
                target
                    .path()
                    .join("templates/command-skills/ags-setup/SKILL.md")
            )
            .unwrap(),
            fs::read(source.path().join("global-skills/ags-setup/SKILL.md")).unwrap()
        );
        let projected_body = target.path().join("templates/command-skills/ags-setup");
        let projected_hash = ags_capability_governance::hash_skill_source(&projected_body)
            .unwrap()
            .trim_start_matches("sha256:")
            .to_string();
        assert!(suite.contains(&format!("hash: {projected_hash}")));
        assert!(verify_public_capability_projection(source.path(), target.path()).is_empty());
    }

    #[test]
    fn apply_rejects_stale_plan_hash() {
        let (source, target) = fixture();
        let error = apply_public_capability_projection(source.path(), target.path(), "sha256:old")
            .unwrap_err();
        assert!(error.contains("plan_hash changed"));
    }

    #[test]
    fn changed_authority_body_blocks_projection() {
        let (source, target) = fixture();
        fs::write(
            source.path().join("global-skills/ags-setup/SKILL.md"),
            "changed",
        )
        .unwrap();
        let plan = plan_public_capability_projection(source.path(), target.path());
        assert!(plan
            .blocking_findings
            .iter()
            .any(|finding| finding.contains("authority body hash mismatch")));
    }

    #[test]
    fn stale_target_body_is_replaced_by_the_authority_projection() {
        let (source, target) = fixture();
        let target_body = target
            .path()
            .join("templates/command-skills/ags-setup/SKILL.md");
        write(&target_body, "stale\n");
        let plan = plan_public_capability_projection(source.path(), target.path());
        assert!(
            plan.blocking_findings.is_empty(),
            "{:?}",
            plan.blocking_findings
        );
        assert!(plan
            .generated_files
            .iter()
            .any(|file| file.path.ends_with("ags-setup/SKILL.md") && file.changed));
        apply_public_capability_projection(source.path(), target.path(), &plan.plan_hash).unwrap();
        assert_eq!(
            fs::read(target_body).unwrap(),
            fs::read(source.path().join("global-skills/ags-setup/SKILL.md")).unwrap()
        );
    }

    #[test]
    fn floating_or_unhashed_catalog_source_is_rejected() {
        let (source, target) = fixture();
        let manifest = source
            .path()
            .join("manifests/third-party-capabilities.yaml");
        let content = fs::read_to_string(&manifest)
            .unwrap()
            .replace("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "main")
            .replace(
                "      integrity: sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
                "",
            );
        fs::write(manifest, content).unwrap();
        let plan = plan_public_capability_projection(source.path(), target.path());
        assert!(!plan.blocking_findings.is_empty());
    }
}
