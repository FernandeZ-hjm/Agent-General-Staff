//! Transactional projection of suite-required Skill bodies into supported hosts.
//!
//! The public kernel owns containment, ownership, rename migration, rollback,
//! and selected-host verification. A machine-local caller may additionally pin the
//! authority root (for example, to the local stable checkout); that policy is
//! data supplied by the caller and is never hard-coded into the public build.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const SUITE_SKILL_PROJECTION_SCHEMA: &str = "0.4.0-suite-skill-projection";
pub fn supported_suite_skill_hosts() -> Vec<&'static str> {
    ags_host_integration::supported_skill_hosts().collect()
}

fn managed_host_skill_root(home: &Path, host: &str) -> Option<PathBuf> {
    ags_host_integration::managed_skill_root(home, host)
}

fn legacy_native_skill_root(home: &Path, host: &str) -> Option<PathBuf> {
    let spec = ags_host_integration::platform_spec(host)?;
    let native = home.join(spec.native_skill_subdir?);
    (Some(native.clone()) != managed_host_skill_root(home, host)).then_some(native)
}

fn projection_roots(home: &Path, host: &str) -> Vec<PathBuf> {
    let mut roots = managed_host_skill_root(home, host)
        .into_iter()
        .collect::<Vec<_>>();
    roots.extend(legacy_native_skill_root(home, host));
    roots.sort();
    roots.dedup();
    roots
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteSkillProjectionPolicy {
    /// When present, both manifest loading and every resulting Skill target are
    /// confined to this authority root. Private/local tooling uses this seam to
    /// require stable; public installations normally leave it unset.
    pub required_authority_root: Option<PathBuf>,
    /// Exact non-empty Host set approved for this installation.
    pub target_hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperationKind {
    Create,
    Replace,
    RemoveRenamed,
    RemoveRetired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteSkillProjectionOperation {
    pub kind: ProjectionOperationKind,
    pub host: String,
    pub skill_id: String,
    pub link_path: PathBuf,
    pub desired_target: Option<PathBuf>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteSkillProjectionTarget {
    pub link_path: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedSuiteSkillProjection {
    pub authority_root: PathBuf,
    pub required_skills: Vec<String>,
    pub hosts: Vec<String>,
    /// Hosts previously owned by this projection whose links and sealed
    /// snapshots must be retired by this transaction.
    pub deactivated_hosts: Vec<String>,
    pub projected_links: BTreeMap<String, SuiteSkillProjectionTarget>,
    pub operations: Vec<SuiteSkillProjectionOperation>,
    pub blocking_findings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteSkillMutationResult {
    pub schema_version: String,
    pub transaction_id: String,
    pub authority_root: PathBuf,
    pub projected_links: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ManifestDocument {
    suite: ManifestSuite,
}

#[derive(Debug, Deserialize)]
struct ManifestSuite {
    #[serde(default)]
    required: Vec<ManifestSkill>,
}

#[derive(Debug, Deserialize)]
struct ManifestSkill {
    name: String,
    source: String,
    hash: String,
    #[serde(default)]
    renamed_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionState {
    schema_version: String,
    authority_root: PathBuf,
    required_skills: Vec<String>,
    projected_links: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum PreviousEntry {
    Absent,
    Symlink(PathBuf),
    DirectoryBackup(PathBuf),
}

const PROJECTION_RECOVERY_SCHEMA: &str = "0.4.13-suite-skill-recovery";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionRecoveryRecord {
    schema_version: String,
    transaction_id: String,
    previous_entries: Vec<(PathBuf, PreviousEntry)>,
    previous_state: Option<Vec<u8>>,
    state_path: PathBuf,
}

#[derive(Debug)]
pub struct AppliedSuiteSkillProjection {
    receipt: SuiteSkillMutationResult,
    recovery: ProjectionRecoveryRecord,
    recovery_path: PathBuf,
}

impl AppliedSuiteSkillProjection {
    pub fn receipt(&self) -> &SuiteSkillMutationResult {
        &self.receipt
    }

    /// Restore every link and the projection ownership record captured before
    /// apply. Recovery refuses to delete a real directory or file that appeared
    /// concurrently at an owned link path.
    pub fn recover(self) -> Result<(), String> {
        recover_record(&self.recovery)?;
        remove_recovery_record(&self.recovery_path)
    }
}

pub fn plan_required_suite_skill_projection(
    source_root: &Path,
    runtime_home: &Path,
    home: &Path,
    policy: &SuiteSkillProjectionPolicy,
) -> Result<PreparedSuiteSkillProjection, String> {
    let authority_input = policy
        .required_authority_root
        .as_deref()
        .unwrap_or(source_root);
    reject_symlink_root(authority_input, "suite Skill authority root")?;
    let authority_root = authority_input.canonicalize().map_err(|error| {
        format!(
            "cannot canonicalize suite Skill authority root {}: {error}",
            authority_input.display()
        )
    })?;
    if !authority_root.is_dir() {
        return Err("suite Skill authority root is not a directory".to_string());
    }

    let manifest = load_manifest(&authority_root)?;
    let supported_hosts = supported_suite_skill_hosts()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut target_hosts = policy.target_hosts.clone();
    target_hosts.sort();
    target_hosts.dedup();
    if target_hosts.is_empty() {
        return Err("required suite Skill projection needs at least one selected Host".to_string());
    }
    if let Some(host) = target_hosts
        .iter()
        .find(|host| !supported_hosts.contains(host.as_str()))
    {
        return Err(format!("unsupported suite Skill host `{host}`"));
    }
    let prior = load_state(runtime_home)?;
    let mut owned_roots = vec![authority_root.clone()];
    if let Some(state) = &prior {
        if let Ok(root) = state.authority_root.canonicalize() {
            owned_roots.push(root);
        }
    }
    owned_roots.sort();
    owned_roots.dedup();

    let mut required = Vec::new();
    let mut desired = BTreeMap::new();
    let mut renamed = BTreeMap::<String, String>::new();
    let mut blocking = Vec::new();
    for skill in manifest.suite.required {
        if !safe_component(&skill.name) {
            blocking.push(format!("invalid required Skill id `{}`", skill.name));
            continue;
        }
        if desired.contains_key(&skill.name) {
            blocking.push(format!("duplicate required Skill id `{}`", skill.name));
            continue;
        }
        let source = match confined_skill_source(&authority_root, &skill) {
            Ok(source) => source,
            Err(error) => {
                blocking.push(error);
                continue;
            }
        };
        for old in skill.renamed_from {
            if !safe_component(&old) || old == skill.name {
                blocking.push(format!(
                    "invalid renamed_from `{old}` for required Skill `{}`",
                    skill.name
                ));
            } else if let Some(existing) = renamed.insert(old.clone(), skill.name.clone()) {
                blocking.push(format!(
                    "renamed Skill `{old}` maps to both `{existing}` and `{}`",
                    skill.name
                ));
            }
        }
        required.push(skill.name.clone());
        desired.insert(skill.name, source);
    }
    required.sort();

    let mut operations = Vec::new();
    let mut desired_links = BTreeSet::new();
    let mut projected_links = BTreeMap::new();
    for host in &target_hosts {
        let Some(host_root) = managed_host_skill_root(home, host) else {
            blocking.push(format!(
                "required Skill host `{host}` has no managed Skill root"
            ));
            continue;
        };
        for (skill_id, source) in &desired {
            let link = host_root.join(skill_id);
            desired_links.insert(link.clone());
            projected_links.insert(
                format!("{host}/{skill_id}"),
                SuiteSkillProjectionTarget {
                    link_path: link.clone(),
                    target: source.clone(),
                },
            );
            match inspect_projection_entry(&link, source, &owned_roots, prior.as_ref()) {
                Ok(LinkDisposition::Current) => {}
                Ok(LinkDisposition::Create) => operations.push(operation(
                    ProjectionOperationKind::Create,
                    host,
                    skill_id,
                    link,
                    Some(source.clone()),
                    "required suite Skill is absent",
                )),
                Ok(LinkDisposition::Replace) => operations.push(operation(
                    ProjectionOperationKind::Replace,
                    host,
                    skill_id,
                    link,
                    Some(source.clone()),
                    "AGS-owned required suite Skill points to an older authority",
                )),
                Err(error) => blocking.push(error),
            }
        }

        for (old, new) in &renamed {
            let link = host_root.join(old);
            plan_removal(
                &mut operations,
                &mut blocking,
                prior.as_ref(),
                &owned_roots,
                host,
                old,
                &link,
                ProjectionOperationKind::RemoveRenamed,
                &format!("upstream renamed `{old}` to `{new}`"),
            );
        }

        // Pre-0.5 projections wrote shared-loading Hosts into both their
        // native root and ~/.agents/skills. Retire only an owned native link;
        // a user entry with the same id remains a blocking conflict.
        if let Some(legacy_root) = legacy_native_skill_root(home, host) {
            for skill_id in desired.keys() {
                plan_removal(
                    &mut operations,
                    &mut blocking,
                    prior.as_ref(),
                    &owned_roots,
                    host,
                    skill_id,
                    &legacy_root.join(skill_id),
                    ProjectionOperationKind::RemoveRetired,
                    "Host loads the canonical shared Skill index",
                );
            }
            for (old, new) in &renamed {
                plan_removal(
                    &mut operations,
                    &mut blocking,
                    prior.as_ref(),
                    &owned_roots,
                    host,
                    old,
                    &legacy_root.join(old),
                    ProjectionOperationKind::RemoveRenamed,
                    &format!("upstream renamed `{old}` to `{new}`"),
                );
            }
        }
    }

    // Pre-0.5 setup projected recommended third-party Skills directly from
    // the suite checkout and, on some machines, never persisted projection
    // ownership state. Recover ownership from two typed facts only: the
    // canonical catalog id and an exact symlink target under this authority.
    // Unowned or user-managed entries are deliberately ignored here.
    if let Ok(catalog) =
        ags_capability_governance::third_party_manifest::read_third_party_manifest(&authority_root)
    {
        let current = required.iter().cloned().collect::<BTreeSet<_>>();
        let retired_catalog = catalog
            .capabilities
            .into_iter()
            .filter(|capability| {
                capability.kind
                    == ags_capability_governance::third_party_manifest::CapabilityKind::Skill
                    && !current.contains(&capability.id)
            })
            .map(|capability| capability.id)
            .collect::<BTreeSet<_>>();
        for host in supported_suite_skill_hosts() {
            for root in projection_roots(home, host) {
                for skill_id in &retired_catalog {
                    let link = root.join(skill_id);
                    let owned = fs::symlink_metadata(&link)
                        .is_ok_and(|metadata| metadata.file_type().is_symlink())
                        && fs::read_link(&link).is_ok_and(|raw| {
                            raw_link_is_owned(
                                &link,
                                &raw,
                                std::slice::from_ref(&authority_root),
                                None,
                            )
                        });
                    if owned {
                        operations.push(operation(
                            ProjectionOperationKind::RemoveRetired,
                            host,
                            skill_id,
                            link,
                            None,
                            "catalog Skill is migrating from retired suite ownership",
                        ));
                    }
                }
            }
        }
    }

    if let Some(state) = &prior {
        let current = required.iter().cloned().collect::<BTreeSet<_>>();
        for (key, target) in &state.projected_links {
            let Some((host, skill_id)) = key.split_once('/') else {
                blocking.push(format!("invalid prior projection key `{key}`"));
                continue;
            };
            if target.as_os_str().is_empty() {
                blocking.push(format!("prior projection target is empty for `{key}`"));
                continue;
            }
            for root in projection_roots(home, host) {
                let link = root.join(skill_id);
                if desired_links.contains(&link) || renamed.contains_key(skill_id) {
                    continue;
                }
                plan_removal(
                    &mut operations,
                    &mut blocking,
                    prior.as_ref(),
                    &owned_roots,
                    host,
                    skill_id,
                    &link,
                    ProjectionOperationKind::RemoveRetired,
                    if current.contains(skill_id) {
                        "Host is no longer selected for required suite Skills"
                    } else {
                        "Skill is no longer in suite.required"
                    },
                );
            }
        }
    }

    operations.sort_by(|left, right| {
        left.link_path
            .cmp(&right.link_path)
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });
    operations.dedup_by(|left, right| left.link_path == right.link_path && left.kind == right.kind);
    blocking.sort();
    blocking.dedup();

    let selected = target_hosts.iter().cloned().collect::<BTreeSet<_>>();
    let mut deactivated_hosts = prior
        .iter()
        .flat_map(|state| state.projected_links.keys())
        .filter_map(|key| key.split_once('/').map(|(host, _)| host.to_string()))
        .filter(|host| !selected.contains(host))
        .collect::<Vec<_>>();
    deactivated_hosts.sort();
    deactivated_hosts.dedup();
    let plan = PreparedSuiteSkillProjection {
        authority_root,
        required_skills: required,
        hosts: target_hosts,
        deactivated_hosts,
        projected_links,
        operations,
        blocking_findings: blocking,
    };
    Ok(plan)
}

pub fn apply_required_suite_skill_projection(
    runtime_home: &Path,
    plan: &PreparedSuiteSkillProjection,
    transaction_id: &str,
) -> Result<AppliedSuiteSkillProjection, String> {
    if !plan.blocking_findings.is_empty() {
        return Err(format!(
            "suite Skill projection has blocking findings: {}",
            plan.blocking_findings.join("; ")
        ));
    }
    let state_path = state_path(runtime_home);
    let previous_state = fs::read(&state_path).ok();
    let mut previous_entries = Vec::new();
    for operation in &plan.operations {
        previous_entries.push((
            operation.link_path.clone(),
            capture_previous(&operation.link_path, transaction_id)?,
        ));
    }

    let recovery = ProjectionRecoveryRecord {
        schema_version: PROJECTION_RECOVERY_SCHEMA.to_string(),
        transaction_id: transaction_id.to_string(),
        previous_entries: previous_entries.clone(),
        previous_state,
        state_path: state_path.clone(),
    };
    let recovery_path = suite_skill_recovery_path(runtime_home, transaction_id);
    persist_recovery_record(&recovery_path, &recovery)?;

    let applied = (|| {
        for operation in &plan.operations {
            match operation.kind {
                ProjectionOperationKind::Create | ProjectionOperationKind::Replace => {
                    let target = operation
                        .desired_target
                        .as_deref()
                        .ok_or_else(|| "projection write is missing desired_target".to_string())?;
                    replace_projection_entry(&operation.link_path, target, transaction_id)?;
                }
                ProjectionOperationKind::RemoveRenamed | ProjectionOperationKind::RemoveRetired => {
                    remove_owned_entry(&operation.link_path, transaction_id)?
                }
            }
        }
        verify_required_suite_skill_projection(plan)?;

        let projected_links = expected_links(plan);
        let state = ProjectionState {
            schema_version: SUITE_SKILL_PROJECTION_SCHEMA.to_string(),
            authority_root: plan.authority_root.clone(),
            required_skills: plan.required_skills.clone(),
            projected_links: projected_links.clone(),
        };
        let mut bytes = serde_json::to_vec_pretty(&state)
            .map_err(|error| format!("cannot serialize suite Skill projection state: {error}"))?;
        bytes.push(b'\n');
        ags_platform::atomic_write(&state_path, &bytes)
            .map_err(|error| format!("cannot persist suite Skill projection state: {error}"))?;
        Ok(SuiteSkillMutationResult {
            schema_version: SUITE_SKILL_PROJECTION_SCHEMA.to_string(),
            transaction_id: transaction_id.to_string(),
            authority_root: plan.authority_root.clone(),
            projected_links,
        })
    })();

    match applied {
        Ok(receipt) => Ok(AppliedSuiteSkillProjection {
            receipt,
            recovery,
            recovery_path,
        }),
        Err(error) => {
            let recovered = recover_record(&recovery);
            match recovered {
                Ok(()) => {
                    remove_recovery_record(&recovery_path)?;
                    Err(error)
                }
                Err(recovery_error) => Err(format!(
                    "{error}; suite Skill projection rollback failed: {recovery_error}"
                )),
            }
        }
    }
}

pub fn recover_required_suite_skill_projection(
    runtime_home: &Path,
    plan: &PreparedSuiteSkillProjection,
    transaction_id: &str,
) -> Result<(), String> {
    let path = suite_skill_recovery_path(runtime_home, transaction_id);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "cannot read projection recovery {}: {error}",
            path.display()
        )
    })?;
    let record: ProjectionRecoveryRecord = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "cannot parse projection recovery {}: {error}",
            path.display()
        )
    })?;
    if record.schema_version != PROJECTION_RECOVERY_SCHEMA
        || record.transaction_id != transaction_id
        || record.state_path != state_path(runtime_home)
    {
        return Err("suite Skill projection recovery identity mismatch".to_string());
    }
    let planned_paths = plan
        .operations
        .iter()
        .map(|operation| operation.link_path.as_path())
        .collect::<BTreeSet<_>>();
    if record
        .previous_entries
        .iter()
        .any(|(path, _)| !planned_paths.contains(path.as_path()))
    {
        return Err("suite Skill projection recovery path is not in the sealed plan".to_string());
    }
    recover_record(&record)?;
    remove_recovery_record(&path)
}

pub fn verify_required_suite_skill_projection(
    plan: &PreparedSuiteSkillProjection,
) -> Result<(), String> {
    verify_required_suite_skill_projection_with_runtime(plan, None)
}

pub fn verify_required_suite_skill_projection_with_runtime(
    plan: &PreparedSuiteSkillProjection,
    runtime_home: Option<&Path>,
) -> Result<(), String> {
    let authority = plan.authority_root.canonicalize().map_err(|error| {
        format!("cannot canonicalize projection authority during verify: {error}")
    })?;
    for (key, projection) in &plan.projected_links {
        if !key.contains('/') {
            return Err(format!("invalid expected projection key `{key}`"));
        }
        verify_projection_entry(&projection.link_path, &projection.target, &authority)?;
    }
    for operation in &plan.operations {
        if matches!(
            operation.kind,
            ProjectionOperationKind::RemoveRenamed | ProjectionOperationKind::RemoveRetired
        ) && fs::symlink_metadata(&operation.link_path).is_ok()
            && !runtime_home.is_some_and(|runtime_home| {
                operation.kind == ProjectionOperationKind::RemoveRetired
                    && link_is_current_installed_skill(runtime_home, operation)
            })
        {
            return Err(format!(
                "retired suite Skill projection remains: {}",
                operation.link_path.display()
            ));
        }
    }
    if let Some(runtime_home) = runtime_home {
        for host in &plan.deactivated_hosts {
            let snapshot = ags_capability_governance::snapshot_path(runtime_home, host);
            if snapshot.exists() {
                return Err(format!(
                    "deactivated Host `{host}` still has a sealed capability snapshot: {}",
                    snapshot.display()
                ));
            }
        }
    }
    Ok(())
}

fn link_is_current_installed_skill(
    runtime_home: &Path,
    operation: &SuiteSkillProjectionOperation,
) -> bool {
    let Ok(index) = ags_capability_governance::skill_adoption::load_installed_skills(runtime_home)
    else {
        return false;
    };
    let Some(record) = index.skills.get(&operation.skill_id) else {
        return false;
    };
    let body = ags_capability_governance::skill_adoption::body_path(runtime_home, record);
    let Ok(raw) = fs::read_link(&operation.link_path) else {
        return false;
    };
    let actual = if raw.is_absolute() {
        raw
    } else {
        operation
            .link_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(raw)
    };
    match (actual.canonicalize(), body.canonicalize()) {
        (Ok(actual), Ok(body)) => actual == body,
        _ => false,
    }
}

fn expected_links(plan: &PreparedSuiteSkillProjection) -> BTreeMap<String, PathBuf> {
    plan.projected_links
        .iter()
        .map(|(key, projection)| (key.clone(), projection.target.clone()))
        .collect()
}

fn load_manifest(root: &Path) -> Result<ManifestDocument, String> {
    let path = root.join("manifests/suite.yaml");
    let body = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_yaml::from_str(&body).map_err(|error| format!("invalid {}: {error}", path.display()))
}

fn load_state(runtime_home: &Path) -> Result<Option<ProjectionState>, String> {
    let path = state_path(runtime_home);
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let state: ProjectionState = serde_json::from_str(&body)
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
    if state.schema_version != SUITE_SKILL_PROJECTION_SCHEMA {
        return Err(format!(
            "unsupported suite Skill projection state schema `{}`",
            state.schema_version
        ));
    }
    Ok(Some(state))
}

fn state_path(runtime_home: &Path) -> PathBuf {
    ags_platform::RuntimeLayout::new(runtime_home).suite_projection_state()
}

fn confined_skill_source(root: &Path, skill: &ManifestSkill) -> Result<PathBuf, String> {
    let relative = Path::new(&skill.source);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "required Skill `{}` has unsafe source `{}`",
            skill.name, skill.source
        ));
    }
    let input = root.join(relative);
    reject_relative_symlinks(
        root,
        relative,
        &format!("required Skill `{}` source", skill.name),
    )?;
    let source = input.canonicalize().map_err(|error| {
        format!(
            "cannot resolve required Skill `{}` source {}: {error}",
            skill.name,
            input.display()
        )
    })?;
    if !source.starts_with(root) || !source.join("SKILL.md").is_file() {
        return Err(format!(
            "required Skill `{}` source escapes authority or lacks SKILL.md: {}",
            skill.name,
            source.display()
        ));
    }
    let body = fs::read_to_string(source.join("SKILL.md"))
        .map_err(|error| format!("cannot read required Skill `{}`: {error}", skill.name))?;
    if frontmatter_name(&body) != Some(skill.name.as_str()) {
        return Err(format!(
            "required Skill `{}` SKILL.md identity does not match its manifest name",
            skill.name
        ));
    }
    let observed_hash = ags_capability_governance::hash_skill_source(&source)?;
    let expected_hash = skill.hash.strip_prefix("sha256:").unwrap_or(&skill.hash);
    let observed_hash = observed_hash
        .strip_prefix("sha256:")
        .unwrap_or(&observed_hash);
    if expected_hash.len() != 64 || observed_hash != expected_hash {
        return Err(format!(
            "required Skill `{}` content hash mismatch: expected {}, observed {}",
            skill.name, skill.hash, observed_hash
        ));
    }
    Ok(source)
}

fn reject_symlink_root(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} contains a symlink: {}", path.display()));
    }
    Ok(())
}

fn reject_relative_symlinks(root: &Path, relative: &Path, label: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("{label} contains an unsafe path component"));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("cannot inspect {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("{label} contains a symlink: {}", current.display()));
        }
    }
    Ok(())
}

fn frontmatter_name(body: &str) -> Option<&str> {
    let mut lines = body.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(value) = line.trim().strip_prefix("name:") {
            return Some(value.trim().trim_matches(['\'', '"']));
        }
    }
    None
}

enum LinkDisposition {
    Current,
    Create,
    Replace,
}

fn inspect_projection_entry(
    link: &Path,
    desired: &Path,
    owned_roots: &[PathBuf],
    prior: Option<&ProjectionState>,
) -> Result<LinkDisposition, String> {
    let metadata = match fs::symlink_metadata(link) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LinkDisposition::Create)
        }
        Err(error) => return Err(format!("cannot inspect {}: {error}", link.display())),
    };
    if metadata.is_dir() && cfg!(windows) {
        if !prior_owns_entry(link, prior) {
            return Err(format!(
                "required Skill host directory is not AGS-owned: {}",
                link.display()
            ));
        }
        let actual = ags_capability_governance::hash_skill_source(link)?;
        let expected = ags_capability_governance::hash_skill_source(desired)?;
        return Ok(if actual == expected {
            LinkDisposition::Current
        } else {
            LinkDisposition::Replace
        });
    }
    if !metadata.file_type().is_symlink() {
        return Err(format!(
            "required Skill host entry is not an AGS-owned symlink: {}",
            link.display()
        ));
    }
    let raw = fs::read_link(link)
        .map_err(|error| format!("cannot read symlink {}: {error}", link.display()))?;
    match resolve_link_target(link) {
        Ok(actual) if actual == desired => Ok(LinkDisposition::Current),
        Ok(actual) if link_is_owned(link, &actual, owned_roots, prior) => {
            Ok(LinkDisposition::Replace)
        }
        Ok(actual) => Err(format!(
            "refusing to replace unowned Skill symlink {} -> {}",
            link.display(),
            actual.display()
        )),
        Err(_) if raw_link_is_owned(link, &raw, owned_roots, prior) => Ok(LinkDisposition::Replace),
        Err(error) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_removal(
    operations: &mut Vec<SuiteSkillProjectionOperation>,
    blocking: &mut Vec<String>,
    prior: Option<&ProjectionState>,
    owned_roots: &[PathBuf],
    host: &str,
    skill_id: &str,
    link: &Path,
    kind: ProjectionOperationKind,
    reason: &str,
) {
    let metadata = match fs::symlink_metadata(link) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            blocking.push(format!("cannot inspect {}: {error}", link.display()));
            return;
        }
    };
    if metadata.is_dir() && cfg!(windows) {
        if prior_owns_entry(link, prior) {
            operations.push(operation(
                kind,
                host,
                skill_id,
                link.to_path_buf(),
                None,
                reason,
            ));
        } else {
            blocking.push(format!(
                "refusing to retire unowned Skill directory: {}",
                link.display()
            ));
        }
        return;
    }
    if !metadata.file_type().is_symlink() {
        blocking.push(format!(
            "refusing to retire non-symlink Skill entry: {}",
            link.display()
        ));
        return;
    }
    let raw = match fs::read_link(link) {
        Ok(raw) => raw,
        Err(error) => {
            blocking.push(format!("cannot read symlink {}: {error}", link.display()));
            return;
        }
    };
    if raw_link_is_owned(link, &raw, owned_roots, prior) {
        operations.push(operation(
            kind,
            host,
            skill_id,
            link.to_path_buf(),
            None,
            reason,
        ));
        return;
    }
    match resolve_link_target(link) {
        Ok(actual) if link_is_owned(link, &actual, owned_roots, prior) => operations.push(
            operation(kind, host, skill_id, link.to_path_buf(), None, reason),
        ),
        Ok(actual) => blocking.push(format!(
            "refusing to retire unowned Skill symlink {} -> {}",
            link.display(),
            actual.display()
        )),
        Err(error) => blocking.push(error),
    }
}

fn raw_link_is_owned(
    link: &Path,
    raw: &Path,
    owned_roots: &[PathBuf],
    prior: Option<&ProjectionState>,
) -> bool {
    let actual = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        link.parent().unwrap_or_else(|| Path::new(".")).join(raw)
    };
    let skill_id = link.file_name().and_then(|name| name.to_str());
    if skill_id.is_some_and(|skill_id| {
        owned_roots
            .iter()
            .any(|root| actual == root.join("global-skills").join(skill_id))
    }) {
        return true;
    }
    prior.is_some_and(|state| {
        state.projected_links.iter().any(|(key, target)| {
            let Some((host, skill)) = key.split_once('/') else {
                return false;
            };
            link_matches_host_skill_path(link, host, skill) && *target == actual
        })
    })
}

fn link_matches_host_skill_path(link: &Path, host: &str, skill: &str) -> bool {
    projection_roots(Path::new(""), host)
        .into_iter()
        .any(|root| link.ends_with(root.join(skill)))
}

fn prior_owns_entry(link: &Path, prior: Option<&ProjectionState>) -> bool {
    prior.is_some_and(|state| {
        state.projected_links.keys().any(|key| {
            let Some((host, skill)) = key.split_once('/') else {
                return false;
            };
            link_matches_host_skill_path(link, host, skill)
        })
    })
}

fn operation(
    kind: ProjectionOperationKind,
    host: &str,
    skill_id: &str,
    link_path: PathBuf,
    desired_target: Option<PathBuf>,
    reason: &str,
) -> SuiteSkillProjectionOperation {
    SuiteSkillProjectionOperation {
        kind,
        host: host.to_string(),
        skill_id: skill_id.to_string(),
        link_path,
        desired_target,
        reason: reason.to_string(),
    }
}

fn resolve_link_target(link: &Path) -> Result<PathBuf, String> {
    let raw = fs::read_link(link)
        .map_err(|error| format!("cannot read symlink {}: {error}", link.display()))?;
    let joined = if raw.is_absolute() {
        raw
    } else {
        link.parent().unwrap_or_else(|| Path::new(".")).join(raw)
    };
    joined.canonicalize().map_err(|error| {
        format!(
            "cannot resolve Skill symlink {} (dangling links are not assumed owned): {error}",
            link.display()
        )
    })
}

fn link_is_owned(
    link: &Path,
    actual: &Path,
    owned_roots: &[PathBuf],
    prior: Option<&ProjectionState>,
) -> bool {
    let skill_id = link.file_name().and_then(|name| name.to_str());
    if skill_id.is_some_and(|skill_id| {
        owned_roots.iter().any(|root| {
            root.join("global-skills")
                .join(skill_id)
                .canonicalize()
                .is_ok_and(|expected| expected == actual)
        })
    }) {
        return true;
    }
    prior.is_some_and(|state| {
        state.projected_links.iter().any(|(key, target)| {
            let Some((host, skill)) = key.split_once('/') else {
                return false;
            };
            link_matches_host_skill_path(link, host, skill)
                && target.canonicalize().is_ok_and(|target| target == actual)
        })
    })
}

fn capture_previous(path: &Path, transaction_id: &str) -> Result<PreviousEntry, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::read_link(path)
            .map(PreviousEntry::Symlink)
            .map_err(|error| format!("cannot read {}: {error}", path.display())),
        Ok(metadata) if metadata.is_dir() && cfg!(windows) => Ok(PreviousEntry::DirectoryBackup(
            projection_backup_path(path, transaction_id)?,
        )),
        Ok(_) => Err(format!(
            "projection target changed to an unsupported entry before apply: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PreviousEntry::Absent),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn replace_projection_entry(
    link: &Path,
    target: &Path,
    transaction_id: &str,
) -> Result<(), String> {
    let parent = link
        .parent()
        .ok_or_else(|| "projection link has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let suffix = transaction_id.trim_start_matches("sha256:");
    let suffix = suffix.get(..12).unwrap_or("invalid-plan");
    let stage = parent.join(format!(
        ".ags-stage-{}-{suffix}",
        link.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
    ));
    if fs::symlink_metadata(&stage).is_ok() {
        return Err(format!(
            "projection stage already exists: {}",
            stage.display()
        ));
    }
    create_projection_stage(target, &stage)?;
    #[cfg(unix)]
    {
        fs::rename(&stage, link).map_err(|error| {
            let _ = fs::remove_file(&stage);
            format!("cannot publish Skill link {}: {error}", link.display())
        })
    }
    #[cfg(not(unix))]
    {
        if let Ok(metadata) = fs::symlink_metadata(link) {
            if metadata.file_type().is_symlink() {
                fs::remove_file(link)
                    .map_err(|error| format!("cannot replace {}: {error}", link.display()))?;
            } else if metadata.is_dir() {
                let backup = projection_backup_path(link, transaction_id)?;
                if fs::symlink_metadata(&backup).is_ok() {
                    let _ = fs::remove_dir_all(&stage);
                    return Err(format!(
                        "projection recovery backup already exists: {}",
                        backup.display()
                    ));
                }
                if let Some(parent) = backup.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("cannot create projection recovery directory: {error}")
                    })?;
                }
                fs::rename(link, &backup).map_err(|error| {
                    let _ = fs::remove_dir_all(&stage);
                    format!("cannot preserve {} for recovery: {error}", link.display())
                })?;
            } else {
                let _ = fs::remove_dir_all(&stage);
                return Err(format!(
                    "cannot replace unsupported projection entry: {}",
                    link.display()
                ));
            }
        }
        fs::rename(&stage, link).map_err(|error| {
            let _ = fs::remove_dir_all(&stage);
            format!("cannot publish Skill link {}: {error}", link.display())
        })
    }
}

fn remove_owned_entry(link: &Path, transaction_id: &str) -> Result<(), String> {
    match fs::symlink_metadata(link) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(link)
            .map_err(|error| format!("cannot unlink {}: {error}", link.display())),
        Ok(metadata) if metadata.is_dir() && cfg!(windows) => {
            let backup = projection_backup_path(link, transaction_id)?;
            if fs::symlink_metadata(&backup).is_ok() {
                return Err(format!(
                    "projection recovery backup already exists: {}",
                    backup.display()
                ));
            }
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("cannot create projection recovery directory: {error}")
                })?;
            }
            fs::rename(link, &backup).map_err(|error| {
                format!("cannot preserve {} for recovery: {error}", link.display())
            })
        }
        Ok(_) => Err(format!(
            "refusing to remove unsupported entry during projection apply: {}",
            link.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot inspect {}: {error}", link.display())),
    }
}

fn verify_projection_entry(link: &Path, desired: &Path, authority: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(link).map_err(|error| {
        format!(
            "required Skill projection missing {}: {error}",
            link.display()
        )
    })?;
    if metadata.is_dir() && cfg!(windows) {
        let actual = ags_capability_governance::hash_skill_source(link)?;
        let expected = ags_capability_governance::hash_skill_source(desired)?;
        if actual != expected {
            return Err(format!(
                "required Skill projection content mismatch {}: expected {}, observed {}",
                link.display(),
                expected,
                actual
            ));
        }
        return Ok(());
    }
    if !metadata.file_type().is_symlink() {
        return Err(format!(
            "required Skill projection has an unsupported entry type: {}",
            link.display()
        ));
    }
    let actual = resolve_link_target(link)?;
    if actual != desired || !actual.starts_with(authority) {
        return Err(format!(
            "required Skill projection target mismatch {} -> {} (expected {})",
            link.display(),
            actual.display(),
            desired.display()
        ));
    }
    Ok(())
}

fn recover_entries(entries: &[(PathBuf, PreviousEntry)]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (path, previous) in entries.iter().rev() {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if let Err(error) = fs::remove_file(path) {
                    errors.push(format!("cannot remove {}: {error}", path.display()));
                    continue;
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                if let PreviousEntry::DirectoryBackup(backup) = previous {
                    if !backup.exists() {
                        continue;
                    }
                }
                if let Err(error) = fs::remove_dir_all(path) {
                    errors.push(format!("cannot remove {}: {error}", path.display()));
                    continue;
                }
            }
            Ok(_) => {
                errors.push(format!(
                    "refusing to overwrite non-symlink during recovery: {}",
                    path.display()
                ));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                errors.push(format!("cannot inspect {}: {error}", path.display()));
                continue;
            }
        }
        match previous {
            PreviousEntry::Absent => {}
            PreviousEntry::Symlink(target) => {
                if let Some(parent) = path.parent() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        errors.push(format!("cannot create {}: {error}", parent.display()));
                        continue;
                    }
                }
                if let Err(error) = create_dir_symlink(target, path) {
                    errors.push(error);
                }
            }
            PreviousEntry::DirectoryBackup(backup) => {
                if !backup.exists() {
                    if !path.is_dir() {
                        errors.push(format!(
                            "projection recovery lost both entry and backup: {}",
                            path.display()
                        ));
                    }
                    continue;
                }
                if let Some(parent) = path.parent() {
                    if let Err(error) = fs::create_dir_all(parent) {
                        errors.push(format!("cannot create {}: {error}", parent.display()));
                        continue;
                    }
                }
                if let Err(error) = fs::rename(backup, path) {
                    errors.push(format!(
                        "cannot restore directory projection {}: {error}",
                        path.display()
                    ));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn suite_skill_recovery_path(runtime_home: &Path, transaction_id: &str) -> PathBuf {
    let identity = crate::maintenance::recovery_file_identity(transaction_id);
    ags_platform::RuntimeLayout::new(runtime_home)
        .maintenance()
        .join("recovery")
        .join(format!("suite-skills-{identity}.json"))
}

fn persist_recovery_record(path: &Path, record: &ProjectionRecoveryRecord) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(record)
        .map_err(|error| format!("cannot serialize suite Skill recovery: {error}"))?;
    bytes.push(b'\n');
    ags_platform::atomic_write(path, &bytes)
        .map_err(|error| format!("cannot persist suite Skill recovery: {error}"))
}

fn recover_record(record: &ProjectionRecoveryRecord) -> Result<(), String> {
    recover_entries(&record.previous_entries)?;
    match &record.previous_state {
        Some(bytes) => ags_platform::atomic_write(&record.state_path, bytes)
            .map_err(|error| format!("cannot restore projection state: {error}")),
        None => match fs::remove_file(&record.state_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot remove projection state: {error}")),
        },
    }
}

fn remove_recovery_record(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot remove projection recovery {}: {error}",
            path.display()
        )),
    }
}

#[cfg(unix)]
fn create_dir_symlink(target: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|error| format!("cannot create symlink {}: {error}", link.display()))
}

#[cfg(not(unix))]
fn create_dir_symlink(target: &Path, link: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(target, link)
        .map_err(|error| format!("cannot create symlink {}: {error}", link.display()))
}

#[cfg(unix)]
fn create_projection_stage(target: &Path, stage: &Path) -> Result<(), String> {
    create_dir_symlink(target, stage)
}

#[cfg(not(unix))]
fn create_projection_stage(target: &Path, stage: &Path) -> Result<(), String> {
    copy_directory_tree(target, stage)
}

#[cfg(any(not(unix), test))]
fn copy_directory_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("cannot inspect Skill source {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Skill projection source must be a real directory: {}",
            source.display()
        ));
    }
    fs::create_dir(target).map_err(|error| {
        format!(
            "cannot create Skill projection stage {}: {error}",
            target.display()
        )
    })?;
    let mut entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read Skill source {}: {error}", source.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "cannot enumerate Skill source {}: {error}",
                source.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let input = entry.path();
        let output = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&input)
            .map_err(|error| format!("cannot inspect Skill source {}: {error}", input.display()))?;
        if metadata.file_type().is_symlink() {
            let _ = fs::remove_dir_all(target);
            return Err(format!(
                "Skill projection refuses source symlink: {}",
                input.display()
            ));
        }
        let result = if metadata.is_dir() {
            copy_directory_tree(&input, &output)
        } else if metadata.is_file() {
            fs::copy(&input, &output)
                .map(|_| ())
                .map_err(|error| format!("cannot copy Skill file {}: {error}", input.display()))
        } else {
            Err(format!(
                "Skill projection refuses special file: {}",
                input.display()
            ))
        };
        if let Err(error) = result {
            let _ = fs::remove_dir_all(target);
            return Err(error);
        }
    }
    Ok(())
}

fn projection_backup_path(link: &Path, transaction_id: &str) -> Result<PathBuf, String> {
    let parent = link
        .parent()
        .ok_or_else(|| "projection entry has no parent".to_string())?;
    let name = link
        .file_name()
        .ok_or_else(|| "projection entry has no file name".to_string())?;
    let suffix = transaction_id.trim_start_matches("sha256:");
    let suffix = suffix.get(..12).unwrap_or("invalid-plan");
    Ok(parent.join(".ags-recovery").join(suffix).join(name))
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(root: &Path, name: &str) {
        let dir = root.join("global-skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: \"{name}\"\ndescription: fixture\n---\n"),
        )
        .unwrap();
    }

    fn manifest(root: &Path, required: &str) {
        fs::create_dir_all(root.join("manifests")).unwrap();
        let mut document: serde_yaml::Value = serde_yaml::from_str(&format!(
            "schema_version: \"1.0\"\nsuite:\n  name: fixture\n  required:\n{required}"
        ))
        .unwrap();
        for entry in document["suite"]["required"].as_sequence_mut().unwrap() {
            let source = entry["source"].as_str().unwrap();
            let hash = ags_capability_governance::hash_skill_source(&root.join(source)).unwrap();
            entry["hash"] = serde_yaml::Value::String(
                hash.strip_prefix("sha256:").unwrap_or(&hash).to_string(),
            );
        }
        fs::write(
            root.join("manifests/suite.yaml"),
            serde_yaml::to_string(&document).unwrap(),
        )
        .unwrap();
    }

    fn plan(root: &Path, runtime: &Path, home: &Path) -> PreparedSuiteSkillProjection {
        plan_required_suite_skill_projection(
            root,
            runtime,
            home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: None,
                target_hosts: supported_suite_skill_hosts()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            },
        )
        .unwrap()
    }

    #[test]
    fn required_skills_use_one_physical_index_for_shared_loading_hosts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&root, "alpha");
        manifest(
            &root,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );

        let plan = plan(&root, &runtime, &home);
        assert!(plan.blocking_findings.is_empty());
        let physical_roots = supported_suite_skill_hosts()
            .into_iter()
            .map(|host| managed_host_skill_root(&home, host).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(plan.operations.len(), physical_roots.len());
        let applied = apply_required_suite_skill_projection(&runtime, &plan, "test-plan").unwrap();
        assert_eq!(
            applied.receipt().projected_links.len(),
            supported_suite_skill_hosts().len()
        );
        for host in supported_suite_skill_hosts() {
            let link = managed_host_skill_root(&home, host).unwrap().join("alpha");
            assert!(fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                link.canonicalize().unwrap(),
                root.join("global-skills/alpha").canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn required_skills_project_only_to_one_selected_host() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&root, "alpha");
        manifest(
            &root,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );

        let plan = plan_required_suite_skill_projection(
            &root,
            &runtime,
            &home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: None,
                target_hosts: vec!["codex".to_string()],
            },
        )
        .unwrap();
        assert_eq!(plan.hosts, vec!["codex"]);
        assert_eq!(plan.operations.len(), 1);
        assert!(plan
            .projected_links
            .keys()
            .all(|key| key.starts_with("codex/")));
        assert_eq!(
            plan.projected_links["codex/alpha"].link_path,
            home.join(".agents/skills/alpha")
        );
    }

    #[test]
    fn required_skill_integrity_covers_metadata_and_assets_not_only_skill_markdown() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&root, "alpha");
        manifest(
            &root,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );
        let metadata = root.join("global-skills/alpha/agents/openai.yaml");
        fs::create_dir_all(metadata.parent().unwrap()).unwrap();
        fs::write(metadata, "interface:\n  display_name: tampered\n").unwrap();

        let plan = plan_required_suite_skill_projection(
            &root,
            &runtime,
            &home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: None,
                target_hosts: vec!["codex".to_string()],
            },
        )
        .unwrap();
        assert!(plan
            .blocking_findings
            .iter()
            .any(|finding| finding.contains("content hash mismatch")));
    }

    #[test]
    fn shared_host_migrates_owned_native_link_without_leaving_a_duplicate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&root, "alpha");
        manifest(
            &root,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );
        let native = home.join(".codex/skills/alpha");
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        create_dir_symlink(&root.join("global-skills/alpha"), &native).unwrap();

        let plan = plan_required_suite_skill_projection(
            &root,
            &runtime,
            &home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: None,
                target_hosts: vec!["codex".to_string()],
            },
        )
        .unwrap();
        assert!(plan.operations.iter().any(|operation| {
            operation.kind == ProjectionOperationKind::RemoveRetired
                && operation.link_path == native
        }));
        drop(apply_required_suite_skill_projection(&runtime, &plan, "test-plan").unwrap());
        assert!(fs::symlink_metadata(&native).is_err());
        assert_eq!(
            home.join(".agents/skills/alpha").canonicalize().unwrap(),
            root.join("global-skills/alpha").canonicalize().unwrap()
        );
    }

    #[test]
    fn rename_migrates_only_owned_symlinks_and_removes_old_name() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&root, "old-name");
        manifest(
            &root,
            "    - name: old-name\n      source: global-skills/old-name\n",
        );
        let first = plan(&root, &runtime, &home);
        drop(apply_required_suite_skill_projection(&runtime, &first, "test-plan").unwrap());

        fs::rename(
            root.join("global-skills/old-name"),
            root.join("global-skills/new-name"),
        )
        .unwrap();
        fs::write(
            root.join("global-skills/new-name/SKILL.md"),
            "---\nname: new-name\ndescription: fixture\n---\n",
        )
        .unwrap();
        manifest(
            &root,
            "    - name: new-name\n      source: global-skills/new-name\n      renamed_from: [old-name]\n",
        );
        let second = plan(&root, &runtime, &home);
        let physical_roots = supported_suite_skill_hosts()
            .into_iter()
            .map(|host| managed_host_skill_root(&home, host).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            second
                .operations
                .iter()
                .filter(|operation| operation.kind == ProjectionOperationKind::RemoveRenamed)
                .count(),
            physical_roots.len()
        );
        drop(apply_required_suite_skill_projection(&runtime, &second, "test-plan").unwrap());
        for host in supported_suite_skill_hosts() {
            let host_root = managed_host_skill_root(&home, host).unwrap();
            assert!(fs::symlink_metadata(host_root.join("old-name")).is_err());
            assert_eq!(
                host_root.join("new-name").canonicalize().unwrap(),
                root.join("global-skills/new-name").canonicalize().unwrap()
            );
        }
    }

    #[test]
    fn unowned_symlink_and_real_directory_are_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        let outside = temp.path().join("outside");
        skill(&root, "alpha");
        skill(&outside, "alpha");
        manifest(
            &root,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );
        let codex = home.join(".agents/skills/alpha");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        create_dir_symlink(&outside.join("global-skills/alpha"), &codex).unwrap();
        let claude = home.join(".claude/skills/alpha");
        fs::create_dir_all(&claude).unwrap();

        let plan = plan(&root, &runtime, &home);
        assert!(plan
            .blocking_findings
            .iter()
            .any(|finding| finding.contains("unowned Skill symlink")));
        assert!(plan
            .blocking_findings
            .iter()
            .any(|finding| finding.contains(if cfg!(windows) {
                "not AGS-owned"
            } else {
                "not an AGS-owned symlink"
            })));
        assert!(apply_required_suite_skill_projection(&runtime, &plan, "test-plan").is_err());
    }

    #[test]
    fn directory_projection_copy_is_content_exact_and_rejects_links() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir_all(source.join("assets")).unwrap();
        fs::write(source.join("SKILL.md"), "---\nname: alpha\n---\n").unwrap();
        fs::write(source.join("assets/data.txt"), "bounded body\n").unwrap();
        copy_directory_tree(&source, &target).unwrap();
        assert_eq!(
            ags_capability_governance::hash_skill_source(&source).unwrap(),
            ags_capability_governance::hash_skill_source(&target).unwrap()
        );

        #[cfg(unix)]
        {
            let linked_source = temp.path().join("linked-source");
            let linked_target = temp.path().join("linked-target");
            fs::create_dir_all(&linked_source).unwrap();
            std::os::unix::fs::symlink(&source, linked_source.join("escape")).unwrap();
            let error = copy_directory_tree(&linked_source, &linked_target).unwrap_err();
            assert!(error.contains("refuses source symlink"));
            assert!(!linked_target.exists());
        }
    }

    #[test]
    fn directory_projection_recovery_restores_the_previous_body() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("skills/alpha");
        let backup = temp.path().join("skills/.ags-recovery/plan/alpha");
        fs::create_dir_all(&link).unwrap();
        fs::write(link.join("SKILL.md"), "old\n").unwrap();
        fs::create_dir_all(backup.parent().unwrap()).unwrap();
        fs::rename(&link, &backup).unwrap();
        fs::create_dir_all(&link).unwrap();
        fs::write(link.join("SKILL.md"), "new\n").unwrap();

        recover_entries(&[(link.clone(), PreviousEntry::DirectoryBackup(backup.clone()))]).unwrap();
        assert_eq!(fs::read_to_string(link.join("SKILL.md")).unwrap(), "old\n");
        assert!(!backup.exists());
    }

    #[test]
    fn local_authority_policy_requires_every_target_under_selected_root() {
        let temp = tempfile::tempdir().unwrap();
        let private = temp.path().join("private");
        let stable = temp.path().join("stable");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&private, "private-only");
        manifest(
            &private,
            "    - name: private-only\n      source: global-skills/private-only\n",
        );
        skill(&stable, "stable-only");
        manifest(
            &stable,
            "    - name: stable-only\n      source: global-skills/stable-only\n",
        );

        let plan = plan_required_suite_skill_projection(
            &private,
            &runtime,
            &home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: Some(stable.clone()),
                target_hosts: vec!["codex".to_string()],
            },
        )
        .unwrap();
        assert_eq!(plan.authority_root, stable.canonicalize().unwrap());
        assert_eq!(plan.required_skills, vec!["stable-only"]);
        assert!(plan.operations.iter().all(|operation| operation
            .desired_target
            .as_ref()
            .is_none_or(|target| target.starts_with(&plan.authority_root))));
    }

    #[test]
    fn explicit_recovery_restores_previous_links_and_state() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old-suite");
        let new = temp.path().join("new-suite");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        for root in [&old, &new] {
            skill(root, "alpha");
            manifest(
                root,
                "    - name: alpha\n      source: global-skills/alpha\n",
            );
        }
        let first = plan(&old, &runtime, &home);
        drop(apply_required_suite_skill_projection(&runtime, &first, "test-plan").unwrap());
        let before_state = fs::read(state_path(&runtime)).unwrap();

        let second = plan(&new, &runtime, &home);
        let applied =
            apply_required_suite_skill_projection(&runtime, &second, "test-plan").unwrap();
        applied.recover().unwrap();
        assert_eq!(fs::read(state_path(&runtime)).unwrap(), before_state);
        for host in supported_suite_skill_hosts() {
            let host_root = managed_host_skill_root(&home, host).unwrap();
            assert_eq!(
                host_root.join("alpha").canonicalize().unwrap(),
                old.join("global-skills/alpha").canonicalize().unwrap()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_authority_and_skill_sources_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        let linked = temp.path().join("linked");
        let runtime = temp.path().join("runtime");
        let home = temp.path().join("home");
        skill(&real, "alpha");
        manifest(
            &real,
            "    - name: alpha\n      source: global-skills/alpha\n",
        );
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let error = plan_required_suite_skill_projection(
            &linked,
            &runtime,
            &home,
            &SuiteSkillProjectionPolicy {
                required_authority_root: None,
                target_hosts: vec!["codex".to_string()],
            },
        )
        .unwrap_err();
        assert!(error.contains("contains a symlink"));
    }
}
