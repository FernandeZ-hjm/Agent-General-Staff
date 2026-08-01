use ags_host_integration::{HostLifecycleCodec, HostLifecycleSpec, LifecycleProjectionFamily};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const LIFECYCLE_MANIFEST_SCHEMA_VERSION: &str = "0.4.0-workspace-lifecycle-manifest";
type LifecycleConfigs = BTreeMap<PathBuf, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleAdapterRecord {
    pub adapter_id: String,
    pub config_path: String,
    pub projection_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleManifest {
    pub schema_version: String,
    pub producer_version: String,
    pub canonical_workspace: String,
    pub workspace_identity: String,
    pub enabled_hosts: BTreeMap<String, LifecycleAdapterRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostLifecycleProjection {
    pub host: String,
    pub adapter_id: String,
    pub config_path: PathBuf,
    pub desired_hash: String,
    pub observed_hash: Option<String>,
    pub file_present: bool,
    pub events_complete: bool,
    pub canonical_target: bool,
    pub current: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLifecycleObservation {
    pub canonical_workspace: PathBuf,
    pub enabled_hosts: Vec<String>,
    pub effective_hosts: Vec<String>,
    pub projections: Vec<HostLifecycleProjection>,
    pub manifest_current: bool,
    pub legacy_markers: Vec<String>,
    pub duplicate_events: Vec<String>,
    pub global_ags_owned_hosts: Vec<String>,
    pub current: bool,
    pub clean: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionInstallOutcome {
    AlreadyCurrent,
    Installed,
}

#[derive(Debug, Clone)]
pub struct LifecycleProjection {
    codec: HostLifecycleCodec,
}

impl LifecycleProjection {
    pub fn new(workspace: &Path, host: &str) -> Result<Self, String> {
        Ok(Self {
            codec: HostLifecycleCodec::new(workspace, host)?,
        })
    }

    pub fn adapter_id(&self) -> &'static str {
        self.codec.spec().adapter_id
    }

    pub fn path(&self) -> PathBuf {
        self.codec.path()
    }

    pub fn desired_hash(&self) -> String {
        self.codec.desired_hash()
    }

    pub fn observe(&self) -> HostLifecycleProjection {
        let spec = self.codec.spec();
        let path = self.path();
        let desired_hash = self.desired_hash();
        let Ok(body) = std::fs::read_to_string(&path) else {
            return HostLifecycleProjection {
                host: spec.host_id.to_string(),
                adapter_id: spec.adapter_id.to_string(),
                config_path: path,
                desired_hash,
                observed_hash: None,
                file_present: false,
                events_complete: false,
                canonical_target: false,
                current: false,
                detail: "projection file missing".to_string(),
            };
        };
        match self.codec.observe_body(&body) {
            Ok(observation) => HostLifecycleProjection {
                host: spec.host_id.to_string(),
                adapter_id: spec.adapter_id.to_string(),
                config_path: path,
                desired_hash: observation.desired_hash,
                observed_hash: observation.observed_hash,
                file_present: true,
                events_complete: observation.events.complete(),
                canonical_target: observation.canonical_target,
                current: observation.current,
                detail: observation.detail,
            },
            Err(error) => HostLifecycleProjection {
                host: spec.host_id.to_string(),
                adapter_id: spec.adapter_id.to_string(),
                config_path: path,
                desired_hash,
                observed_hash: None,
                file_present: true,
                events_complete: false,
                canonical_target: false,
                current: false,
                detail: error,
            },
        }
    }

    pub fn install(&self) -> Result<ProjectionInstallOutcome, String> {
        self.install_inner(None)
    }

    pub(crate) fn install_with_backup(
        &self,
        backup: &Path,
    ) -> Result<ProjectionInstallOutcome, String> {
        self.install_inner(Some(backup))
    }

    fn install_inner(&self, backup: Option<&Path>) -> Result<ProjectionInstallOutcome, String> {
        let primary_current = self.observe().current;
        let path = self.path();
        let primary = if primary_current {
            None
        } else {
            let current = if path.exists() {
                Some(
                    std::fs::read_to_string(&path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
                )
            } else {
                None
            };
            let rendered = match self.render(current.as_deref()) {
                Ok(rendered) => rendered,
                Err(error) => {
                    if let (Some(_), Some(backup)) = (current.as_ref(), backup) {
                        back_up_lifecycle_config_if_missing(&path, backup)?;
                    }
                    return Err(error);
                }
            };
            Some((current, rendered))
        };
        let alternates = match self.alternate_migrations() {
            Ok(alternates) => alternates,
            Err((alternate, error)) => {
                back_up_lifecycle_config_if_missing(
                    &alternate,
                    &migration_backup_path(&alternate),
                )?;
                return Err(error);
            }
        };
        if primary.is_none() && alternates.is_empty() {
            return Ok(ProjectionInstallOutcome::AlreadyCurrent);
        }

        if let Some((Some(_), _)) = primary.as_ref() {
            if let Some(backup) = backup {
                back_up_lifecycle_config_if_missing(&path, backup)?;
            }
        }
        for (alternate, _) in &alternates {
            back_up_lifecycle_config_if_missing(alternate, &migration_backup_path(alternate))?;
        }

        if let Some((_, rendered)) = primary {
            write_lifecycle_config(&path, &rendered)?;
            let observed = self.observe();
            if !observed.current {
                return Err(format!(
                    "installed lifecycle projection failed verification: {}",
                    observed.detail
                ));
            }
        }
        for (alternate, rendered) in alternates {
            write_lifecycle_config(&alternate, &rendered)?;
            let body = std::fs::read_to_string(&alternate)
                .map_err(|error| format!("cannot verify {}: {error}", alternate.display()))?;
            let mut value = serde_json::from_str::<serde_json::Value>(&body).map_err(|error| {
                format!(
                    "invalid lifecycle projection JSON in {} after migration: {error}",
                    alternate.display()
                )
            })?;
            if ags_host_integration::remove_owned_lifecycle_entries(
                &mut value,
                self.codec.spec().host_id,
            ) {
                return Err(format!(
                    "alternate lifecycle projection still contains AGS-owned hooks: {}",
                    alternate.display()
                ));
            }
        }
        Ok(ProjectionInstallOutcome::Installed)
    }

    pub(crate) fn ready_after_install(&self) -> bool {
        let path = self.path();
        let primary_ready = self.observe().current
            || if path.exists() {
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|body| self.render(Some(&body)).ok())
                    .is_some()
            } else {
                self.render(None).is_ok()
            };
        primary_ready && self.alternate_migrations().is_ok()
    }

    fn alternate_migrations(&self) -> Result<Vec<(PathBuf, String)>, (PathBuf, String)> {
        let spec = self.codec.spec();
        spec.alternate_workspace_configs
            .iter()
            .filter_map(|relative| {
                let path = self.codec.workspace().join(relative);
                path.exists().then_some(path)
            })
            .filter_map(|path| {
                let result = (|| {
                    let body = std::fs::read_to_string(&path)
                        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                    let mut value =
                        serde_json::from_str::<serde_json::Value>(&body).map_err(|error| {
                            format!(
                                "invalid alternate lifecycle projection JSON in {}: {error}",
                                path.display()
                            )
                        })?;
                    if !value.is_object() {
                        return Err(format!(
                            "alternate lifecycle projection root must be a JSON object: {}",
                            path.display()
                        ));
                    }
                    if !ags_host_integration::remove_owned_lifecycle_entries(
                        &mut value,
                        spec.host_id,
                    ) {
                        return Ok(None);
                    }
                    let mut rendered =
                        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
                    rendered.push('\n');
                    Ok(Some(rendered))
                })();
                match result {
                    Ok(Some(rendered)) => Some(Ok((path, rendered))),
                    Ok(None) => None,
                    Err(error) => Some(Err((path, error))),
                }
            })
            .collect()
    }

    pub fn render(&self, existing: Option<&str>) -> Result<String, String> {
        match self.codec.spec().projection_family {
            LifecycleProjectionFamily::OmpExtension => Ok(self.codec.desired_omp_body()),
            LifecycleProjectionFamily::CommandHooks
            | LifecycleProjectionFamily::CursorCommandHooks => {
                let mut value = match existing {
                    Some(body) => serde_json::from_str::<serde_json::Value>(body)
                        .map_err(|error| format!("invalid lifecycle projection JSON: {error}"))?,
                    None => serde_json::json!({}),
                };
                merge_command_projection(&mut value, &self.codec)?;
                let mut rendered =
                    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
                rendered.push('\n');
                Ok(rendered)
            }
        }
    }
}

pub(crate) fn migration_backup_path(path: &Path) -> PathBuf {
    path.with_extension(format!(
        "{}ags-v0.4.11.bak",
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ))
}

fn back_up_lifecycle_config(path: &Path, backup: &Path) -> Result<(), String> {
    std::fs::copy(path, backup).map_err(|error| {
        format!(
            "cannot back up lifecycle projection {} to {}: {error}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(())
}

fn back_up_lifecycle_config_if_missing(path: &Path, backup: &Path) -> Result<(), String> {
    if backup.exists() {
        return Ok(());
    }
    back_up_lifecycle_config(path, backup)
}

fn write_lifecycle_config(path: &Path, rendered: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    ags_platform::atomic_write(path, rendered.as_bytes())
}

pub fn lifecycle_manifest_path(workspace: &Path) -> PathBuf {
    workspace.join(".ags/state/lifecycle/manifest.json")
}

pub fn load_lifecycle_manifest(path: &Path) -> Result<LifecycleManifest, String> {
    serde_json::from_slice(
        &std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("invalid lifecycle manifest: {error}"))
}

pub fn record_lifecycle_manifest(workspace: &Path) -> Result<PathBuf, String> {
    let workspace = ags_platform::canonical_workspace_root(workspace)?;
    let path = lifecycle_manifest_path(&workspace);
    let enabled_hosts = ags_host_integration::lifecycle_specs()
        .filter_map(|spec| {
            let projection = LifecycleProjection::new(&workspace, spec.host_id).ok()?;
            let observed = projection.observe();
            observed.current.then(|| {
                (
                    spec.host_id.to_string(),
                    LifecycleAdapterRecord {
                        adapter_id: projection.adapter_id().to_string(),
                        config_path: projection.path().to_string_lossy().to_string(),
                        projection_hash: projection.desired_hash(),
                    },
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    if enabled_hosts.is_empty() {
        return Err("cannot record lifecycle manifest without a current projection".to_string());
    }
    let manifest = LifecycleManifest {
        schema_version: LIFECYCLE_MANIFEST_SCHEMA_VERSION.to_string(),
        producer_version: env!("CARGO_PKG_VERSION").to_string(),
        canonical_workspace: workspace.to_string_lossy().to_string(),
        workspace_identity: crate::workspace_lifecycle::workspace_identity(&workspace),
        enabled_hosts,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut rendered = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    rendered.push(b'\n');
    ags_platform::atomic_write(&path, &rendered)?;
    Ok(path)
}

pub fn workspace_adapter_path(workspace: &Path, host: &str) -> Option<PathBuf> {
    ags_host_integration::platform_spec(host)
        .and_then(|platform| platform.lifecycle)
        .map(|spec| spec.workspace_config_path(workspace))
}

pub fn global_adapter_path(home: &Path, host: &str) -> Option<PathBuf> {
    ags_host_integration::platform_spec(host)
        .and_then(|platform| platform.lifecycle)
        .map(|spec| spec.global_config_path(home))
}

pub fn observe_workspace_lifecycle(
    workspace: &Path,
    home: &Path,
    required_hosts: &[String],
) -> Result<WorkspaceLifecycleObservation, String> {
    let canonical = ags_platform::canonical_workspace_root(workspace)?;
    let manifest_path = lifecycle_manifest_path(&canonical);
    let manifest_result = load_lifecycle_manifest(&manifest_path);
    let manifest = manifest_result.as_ref().ok();
    let enabled_hosts = manifest
        .as_ref()
        .map(|value| value.enabled_hosts.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let configs = read_lifecycle_configs(&canonical, home);
    let mut effective = enabled_hosts.iter().cloned().collect::<BTreeSet<_>>();
    effective.extend(required_hosts.iter().cloned());
    for spec in ags_host_integration::lifecycle_specs() {
        if lifecycle_is_configured(&configs, &canonical, home, spec) {
            effective.insert(spec.host_id.to_string());
        }
    }
    let effective_hosts = effective.into_iter().collect::<Vec<_>>();
    let projections = effective_hosts
        .iter()
        .filter_map(|host| LifecycleProjection::new(&canonical, host).ok())
        .map(|projection| projection.observe())
        .collect::<Vec<_>>();
    let manifest_current = manifest_result.is_ok()
        && manifest_matches(manifest, &canonical, &effective_hosts, &projections)
        || !manifest_path.exists()
            && manifest_matches(None, &canonical, &effective_hosts, &projections);
    let legacy_markers = detect_legacy_markers(&configs);
    let duplicate_events = detect_duplicate_events(&configs);
    let global_ags_owned_hosts = ags_host_integration::lifecycle_specs()
        .filter(|spec| {
            configs
                .get(&spec.global_config_path(home))
                .is_some_and(|body| body_contains_ags_lifecycle(body, *spec))
        })
        .map(|spec| spec.host_id.to_string())
        .collect::<Vec<_>>();
    let current = manifest_current
        && projections.len() == effective_hosts.len()
        && projections.iter().all(|projection| projection.current);
    let clean = current
        && legacy_markers.is_empty()
        && duplicate_events.is_empty()
        && global_ags_owned_hosts.is_empty();
    Ok(WorkspaceLifecycleObservation {
        canonical_workspace: canonical,
        enabled_hosts,
        effective_hosts,
        projections,
        manifest_current,
        legacy_markers,
        duplicate_events,
        global_ags_owned_hosts,
        current,
        clean,
    })
}

pub fn remove_global_ags_owned(home: &Path, host: &str) -> Result<bool, String> {
    let spec = ags_host_integration::platform_spec(host)
        .and_then(|platform| platform.lifecycle)
        .ok_or_else(|| format!("unsupported lifecycle host `{host}`"))?;
    let path = spec.global_config_path(home);
    if !config_contains_ags_lifecycle(&path, spec) {
        return Ok(false);
    }
    if spec.projection_family == LifecycleProjectionFamily::OmpExtension {
        std::fs::remove_file(&path)
            .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
        return Ok(true);
    }
    let body = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|error| format!("invalid lifecycle projection JSON: {error}"))?;
    let changed = ags_host_integration::remove_owned_lifecycle_entries(&mut value, host);
    if !changed {
        return Err(format!(
            "{} contains a mixed or non-canonical AGS lifecycle command; split it into a standalone hook before migration",
            path.display()
        ));
    }
    let mut rendered = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
    rendered.push(b'\n');
    ags_platform::atomic_write(&path, &rendered)?;
    Ok(changed)
}

pub fn global_ags_owned(home: &Path, host: &str) -> Result<bool, String> {
    let spec = ags_host_integration::platform_spec(host)
        .and_then(|platform| platform.lifecycle)
        .ok_or_else(|| format!("unsupported lifecycle host `{host}`"))?;
    Ok(config_contains_ags_lifecycle(
        &spec.global_config_path(home),
        spec,
    ))
}

fn manifest_matches(
    manifest: Option<&LifecycleManifest>,
    workspace: &Path,
    effective_hosts: &[String],
    projections: &[HostLifecycleProjection],
) -> bool {
    if effective_hosts.is_empty() {
        return manifest.is_none_or(|value| value.enabled_hosts.is_empty());
    }
    let Some(manifest) = manifest else {
        return false;
    };
    if manifest.schema_version != LIFECYCLE_MANIFEST_SCHEMA_VERSION
        || manifest.producer_version != env!("CARGO_PKG_VERSION")
        || manifest.canonical_workspace != workspace.to_string_lossy()
        || manifest.workspace_identity != crate::workspace_lifecycle::workspace_identity(workspace)
    {
        return false;
    }
    effective_hosts.iter().all(|host| {
        let Some(record) = manifest.enabled_hosts.get(host) else {
            return false;
        };
        projections
            .iter()
            .find(|item| &item.host == host)
            .is_some_and(|projection| {
                record.adapter_id == projection.adapter_id
                    && record.config_path == projection.config_path.to_string_lossy()
                    && record.projection_hash == projection.desired_hash
            })
    })
}

fn merge_command_projection(
    value: &mut serde_json::Value,
    codec: &HostLifecycleCodec,
) -> Result<(), String> {
    let spec = codec.spec();
    {
        let root = value
            .as_object_mut()
            .ok_or_else(|| "lifecycle config root must be a JSON object".to_string())?;
        if spec.projection_family == LifecycleProjectionFamily::CursorCommandHooks {
            match root.get("version") {
                Some(version) if version.as_u64() == Some(1) => {}
                None => {
                    root.insert("version".to_string(), serde_json::json!(1));
                }
                Some(version) => {
                    return Err(format!("unsupported Cursor hook schema version {version}"));
                }
            }
        }
    }
    ags_host_integration::remove_owned_lifecycle_entries(value, spec.host_id);
    let hooks = value
        .as_object_mut()
        .expect("validated lifecycle config root")
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "lifecycle config `hooks` must be a JSON object".to_string())?;
    for (native_event, owned_groups) in codec.desired_owned_projection() {
        let groups = hooks
            .entry(&native_event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hook event `{native_event}` must be an array"))?;
        groups.extend(owned_groups);
    }
    Ok(())
}

fn lifecycle_is_configured(
    configs: &LifecycleConfigs,
    workspace: &Path,
    home: &Path,
    spec: HostLifecycleSpec,
) -> bool {
    let configured = |path: PathBuf| {
        configs
            .get(&path)
            .is_some_and(|body| body_contains_ags_lifecycle(body, spec))
    };
    configured(spec.workspace_config_path(workspace))
        || spec
            .alternate_workspace_configs
            .iter()
            .any(|relative| configured(workspace.join(relative)))
        || configured(spec.global_config_path(home))
}

fn config_contains_ags_lifecycle(path: &Path, spec: HostLifecycleSpec) -> bool {
    let Ok(body) = std::fs::read_to_string(path) else {
        return false;
    };
    body_contains_ags_lifecycle(&body, spec)
}

fn body_contains_ags_lifecycle(body: &str, spec: HostLifecycleSpec) -> bool {
    ags_host_integration::lifecycle_body_contains_owned(spec, body)
}

fn read_lifecycle_configs(workspace: &Path, home: &Path) -> LifecycleConfigs {
    let mut paths = BTreeSet::new();
    for spec in ags_host_integration::lifecycle_specs() {
        paths.insert(spec.workspace_config_path(workspace));
        paths.extend(
            spec.alternate_workspace_configs
                .iter()
                .map(|relative| workspace.join(relative)),
        );
        paths.insert(spec.global_config_path(home));
    }
    paths
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(&path).ok().map(|body| (path, body)))
        .collect()
}

fn detect_legacy_markers(configs: &LifecycleConfigs) -> Vec<String> {
    let mut found = Vec::new();
    for (path, body) in configs {
        for spec in
            ags_host_integration::lifecycle_specs().filter(|spec| path_matches_spec(path, *spec))
        {
            for marker in ags_host_integration::lifecycle_config_drift_markers(spec, body) {
                found.push(format!("{}:{marker}", path.display()));
            }
        }
    }
    found
}

fn detect_duplicate_events(configs: &LifecycleConfigs) -> Vec<String> {
    let mut counts = BTreeMap::<(String, String), usize>::new();
    for (path, body) in configs {
        for spec in ags_host_integration::lifecycle_specs() {
            if spec.projection_family == LifecycleProjectionFamily::OmpExtension
                && !path_matches_spec(path, spec)
            {
                continue;
            }
            for (event, count) in ["session-start", "stop-guard", "session-end"]
                .into_iter()
                .zip(ags_host_integration::lifecycle_owned_event_counts(
                    spec, body,
                ))
            {
                if count > 0 {
                    *counts
                        .entry((spec.host_id.to_string(), event.to_string()))
                        .or_default() += count;
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|((host, event), count)| format!("{host}:{event}={count}"))
        .collect()
}

fn path_matches_spec(path: &Path, spec: HostLifecycleSpec) -> bool {
    path.ends_with(Path::new(spec.workspace_config))
        || path.ends_with(Path::new(spec.global_config))
        || spec
            .alternate_workspace_configs
            .iter()
            .any(|relative| path.ends_with(Path::new(relative)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(tag: &str) -> tempfile::TempDir {
        let root = tempfile::Builder::new()
            .prefix(&format!("ags-projection-{tag}-"))
            .tempdir()
            .unwrap();
        std::fs::create_dir_all(root.path().join(".git")).unwrap();
        root
    }

    #[test]
    fn five_hosts_render_observe_and_diff_cleanly() {
        let root = workspace("matrix");
        for spec in ags_host_integration::lifecycle_specs() {
            let projection = LifecycleProjection::new(root.path(), spec.host_id).unwrap();
            assert_eq!(
                projection.install().unwrap(),
                ProjectionInstallOutcome::Installed
            );
            let observed = projection.observe();
            assert!(observed.current, "{}: {}", spec.host_id, observed.detail);
            assert!(observed.events_complete);
            assert!(observed.canonical_target);
        }
    }

    #[test]
    fn command_projection_preserves_user_hooks_and_rejects_owned_schema_drift() {
        let root = workspace("preserve");
        let projection = LifecycleProjection::new(root.path(), "claude-code").unwrap();
        let path = projection.path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "mcpServers": {"user": {"command": "keep"}},
                "hooks": {
                    "Stop": [{
                        "hooks": [
                            {"command": "node keep.js"},
                            {"command": format!(
                                "ags host lifecycle --event session-end --host claude-code --target ."
                            )},
                            {"command":
                                "ags host lifecycle --event stop-guard --host claude-code --target . && ./notify.sh"
                            }
                        ]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        projection.install().unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("node keep.js"));
        assert!(body.contains("&& ./notify.sh"));
        assert!(body.contains("\"mcpServers\""));
        assert_eq!(body.matches("--target .").count(), 1);
        assert!(projection.observe().current);
    }

    #[test]
    fn manifest_receipt_cannot_make_a_drifted_projection_current() {
        let root = workspace("receipt");
        let projection = LifecycleProjection::new(root.path(), "codebuddy-code").unwrap();
        projection.install().unwrap();
        record_lifecycle_manifest(root.path()).unwrap();
        let path = projection.path();
        let body = std::fs::read_to_string(&path)
            .unwrap()
            .replace("session-end", "stop-guard");
        std::fs::write(&path, body).unwrap();
        let observed = observe_workspace_lifecycle(
            root.path(),
            &root.path().join("home"),
            &["codebuddy-code".to_string()],
        )
        .unwrap();
        assert!(!observed.current);
        assert!(!observed.projections[0].current);
    }

    #[test]
    fn invalid_manifest_and_global_json_fail_closed() {
        let root = workspace("invalid-state");
        let home = root.path().join("home");
        let manifest = lifecycle_manifest_path(root.path());
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(&manifest, "{not-json").unwrap();
        let empty = observe_workspace_lifecycle(root.path(), &home, &[]).unwrap();
        assert!(!empty.manifest_current);
        assert!(!empty.current);
        assert_eq!(
            crate::conformance::conformance_checks(&empty)
                .iter()
                .find(|check| check.check_name == "workspace-lifecycle-manifest-current")
                .unwrap()
                .status,
            crate::conformance::ConformanceStatus::Fail
        );

        let global = home.join(".claude/settings.json");
        std::fs::create_dir_all(global.parent().unwrap()).unwrap();
        std::fs::write(
            &global,
            "{ \"hooks\": \"ags host lifecycle --event session-start --host claude-code\" ",
        )
        .unwrap();
        let observed = observe_workspace_lifecycle(root.path(), &home, &[]).unwrap();
        assert!(observed
            .global_ags_owned_hosts
            .contains(&"claude-code".to_string()));
        assert!(!observed.clean);
        assert!(remove_global_ags_owned(&home, "claude-code").is_err());
    }

    #[test]
    fn cursor_unknown_schema_is_never_rewritten() {
        let root = workspace("cursor-schema");
        let projection = LifecycleProjection::new(root.path(), "cursor").unwrap();
        let path = projection.path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"{"version":"1","hooks":{},"user":"keep"}"#;
        std::fs::write(&path, original).unwrap();
        assert!(projection.install().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }

    #[test]
    fn manifest_is_rebuilt_from_all_current_projections_despite_corrupt_prior_state() {
        let root = workspace("manifest-rebuild");
        for host in ["codex", "claude-code"] {
            LifecycleProjection::new(root.path(), host)
                .unwrap()
                .install()
                .unwrap();
        }
        let path = lifecycle_manifest_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{corrupt").unwrap();

        record_lifecycle_manifest(root.path()).unwrap();
        let first = std::fs::read(&path).unwrap();
        let manifest = load_lifecycle_manifest(&path).unwrap();
        assert_eq!(
            manifest.enabled_hosts.keys().cloned().collect::<Vec<_>>(),
            ["claude-code".to_string(), "codex".to_string()]
        );

        record_lifecycle_manifest(root.path()).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), first);
    }
}
