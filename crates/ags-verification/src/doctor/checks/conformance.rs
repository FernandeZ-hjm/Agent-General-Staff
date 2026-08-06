use super::*;
use std::collections::{BTreeMap, BTreeSet};

struct SystemObservation {
    lifecycle: Result<ags_lifecycle::lifecycle_projection::WorkspaceLifecycleObservation, String>,
    enabled_hosts: Vec<String>,
    approved_hosts: Result<Vec<String>, String>,
    mcp_reports: BTreeMap<String, ags_host_integration::HostMcpReport>,
    daemon_status: Option<Result<ags_session::WorkspaceServiceStatus, String>>,
    daemon: Option<Result<Option<ags_session::WorkspaceServiceInspection>, String>>,
}

impl SystemObservation {
    fn collect(repo_root: &Path, home: &Path) -> Self {
        let mcp_reports = collect_mcp_reports(repo_root, home);
        let approved_hosts =
            ags_lifecycle::setup::approved_lifecycle_hosts(&ags_platform::runtime_home());
        let mut required_hosts = mcp_reports
            .iter()
            // A disabled or stale AGS registration still enables conformance
            // inspection. Filtering to `active` would let the exact drift that
            // Doctor must report turn the whole host into a false `skip`.
            .filter(|(_, report)| report.find("ags").is_some())
            .map(|(host, _)| host.clone())
            .collect::<Vec<_>>();
        if let Ok(approved) = &approved_hosts {
            required_hosts.extend(approved.iter().cloned());
            required_hosts.sort();
            required_hosts.dedup();
        }
        let lifecycle = ags_lifecycle::lifecycle_projection::observe_workspace_lifecycle(
            repo_root,
            home,
            &required_hosts,
        );
        let enabled_hosts = lifecycle
            .as_ref()
            .map(|observation| observation.effective_hosts.clone())
            .unwrap_or(required_hosts);
        let daemon_status =
            (!enabled_hosts.is_empty()).then(|| ags_session::workspace_service_status(repo_root));
        let daemon = (!enabled_hosts.is_empty())
            .then(|| ags_session::inspect_existing_workspace_service(repo_root));
        Self {
            lifecycle,
            enabled_hosts,
            approved_hosts,
            mcp_reports,
            daemon_status,
            daemon,
        }
    }
}

pub(super) fn canonical_conformance_checks(repo_root: &Path) -> Vec<Finding> {
    let home = ags_platform::home_dir_or_temp();
    let observation = SystemObservation::collect(repo_root, &home);
    let mut findings = match observation.lifecycle.as_ref() {
        Ok(lifecycle) => ags_lifecycle::conformance::conformance_checks(lifecycle)
            .into_iter()
            .map(map_check)
            .collect(),
        Err(error) => vec![conformance_fail(
            "lifecycle-observation-current",
            "workspace lifecycle state could not be inspected",
            "a readable canonical workspace and lifecycle projection",
            error,
            format!(
                "Run `ags agents govern --apply --target '{}'` after resolving the reported error.",
                repo_root.display()
            ),
        )],
    };
    findings.push(lifecycle_host_approval_current(&observation));
    findings.push(local_overlay_tracking_current(repo_root, &observation));
    findings.push(lifecycle_executable_current(&observation));
    findings.push(workspace_daemon_current(repo_root, &observation));
    findings.push(managed_projection_current(repo_root, &observation));
    findings.push(capability_snapshot_current(repo_root, &home, &observation));
    findings.push(mcp_registration_current(repo_root, &observation));
    findings.push(remote_latest_advisory());
    findings
}

fn lifecycle_host_approval_current(observation: &SystemObservation) -> Finding {
    let approved = match &observation.approved_hosts {
        Ok(hosts) => hosts,
        Err(error) => {
            return conformance_fail(
                "lifecycle-host-approval-current",
                "installed lifecycle host approval is invalid",
                "supported host ids in the current install manifest",
                error,
                "Run `ags setup --yes` to select detected Hosts, or override with `--lifecycle-hosts <ids|detected>`; at least one Host is required.",
            )
        }
    };
    let enabled = observation
        .lifecycle
        .as_ref()
        .map(|lifecycle| lifecycle.effective_hosts.as_slice())
        .unwrap_or_default();
    let missing = approved
        .iter()
        .filter(|host| {
            !observation.lifecycle.as_ref().is_ok_and(|lifecycle| {
                lifecycle.projections.iter().any(|projection| {
                    projection.host.as_str() == host.as_str() && projection.current
                })
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    conformance_verdict(
        missing.is_empty(),
        "lifecycle-host-approval-current",
        "approved lifecycle hosts are enabled in this workspace",
        format!("approved hosts present exactly once: {approved:?}"),
        if missing.is_empty() {
            format!("enabled={enabled:?}")
        } else {
            format!("enabled={enabled:?}; missing={missing:?}")
        },
        "Run `ags init --target <workspace>` or `ags agents govern --agent <host> --target <workspace> --apply`.",
    )
}

fn local_overlay_tracking_current(repo_root: &Path, observation: &SystemObservation) -> Finding {
    if !ags_lifecycle::init::overlay::local_overlay_active(repo_root) {
        return Finding::skip(
            "local-overlay-pure-files-untracked",
            "workspace does not declare the AGS local overlay",
        );
    }
    let indexed = ags_lifecycle::init::overlay::git_tracked_set(repo_root);
    let tracked = observation
        .approved_hosts
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|host| {
            let projection =
                ags_lifecycle::lifecycle_projection::LifecycleProjection::new(repo_root, host)
                    .ok()?;
            let path = projection.path();
            let pure = projection
                .render(None)
                .ok()
                .is_some_and(|rendered| std::fs::read_to_string(&path).ok() == Some(rendered));
            let relative = path.strip_prefix(repo_root).ok()?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            (pure && indexed.contains(&relative)).then_some(relative)
        })
        .collect::<Vec<_>>();
    conformance_verdict(
        tracked.is_empty(),
        "local-overlay-pure-files-untracked",
        "pure AGS lifecycle adapters are absent from the Git index",
        "no pure AGS adapter tracked in local mode",
        format!("tracked={tracked:?}"),
        "Run `ags init --mode local --target <workspace>`.",
    )
}

fn collect_mcp_reports(
    repo_root: &Path,
    home: &Path,
) -> BTreeMap<String, ags_host_integration::HostMcpReport> {
    let detected = ags_host_integration::cross_platform_init_plan(home, &|command| {
        ags_platform::is_on_path(command)
    })
    .platforms
    .into_iter()
    .filter(|host| host.detected)
    .map(|host| host.id)
    .collect::<BTreeSet<_>>();
    let candidates = ags_host_integration::AGENT_PLATFORM_SPECS
        .iter()
        .filter_map(|platform| {
            let lifecycle = platform.lifecycle?;
            platform.mcp_probe?;
            let workspace_configured = lifecycle.workspace_config_path(repo_root).exists()
                || lifecycle
                    .alternate_workspace_configs
                    .iter()
                    .any(|path| repo_root.join(path).exists());
            let configured = workspace_configured
                || lifecycle.global_config_path(home).exists()
                || detected.contains(platform.id);
            configured.then(|| platform.id.to_string())
        })
        .collect::<Vec<_>>();

    std::thread::scope(|scope| {
        let handles = candidates
            .into_iter()
            .map(|host| {
                let worker_host = host.clone();
                (
                    host,
                    scope.spawn(move || {
                        let mut report =
                            ags_host_integration::inspect_host_mcp_at(&worker_host, repo_root);
                        if worker_host == "codebuddy-code"
                            && report.status
                                == ags_host_integration::HostProbeStatus::HostUnavailable
                        {
                            report = ags_host_integration::inspect_codebuddy_mcp_config_at(
                                repo_root, home,
                            )
                            .unwrap_or(report);
                        }
                        match ags_host_integration::inspect_exact_mcp_registration_at(
                            &worker_host,
                            "ags",
                            repo_root,
                            home,
                        ) {
                            Ok(Some(exact)) => {
                                report
                                    .servers
                                    .retain(|registration| registration.name != exact.name);
                                report.servers.push(exact);
                            }
                            Ok(None) => {}
                            Err(error) => {
                                report
                                    .servers
                                    .retain(|registration| registration.name != "ags");
                                report.status =
                                    ags_host_integration::HostProbeStatus::ConnectionFailed;
                                report.evidence = error;
                            }
                        }
                        report
                    }),
                )
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .filter_map(|(host, handle)| handle.join().ok().map(|report| (host, report)))
            .collect()
    })
}

fn map_check(check: ags_lifecycle::conformance::ConformanceCheck) -> Finding {
    use ags_lifecycle::conformance::ConformanceStatus;
    let expected = check.expected.clone();
    let observed = check.observed.clone();
    let remediation = check.remediation.clone();
    let finding = match check.status {
        ConformanceStatus::Pass => Finding::pass(check.check_name, check.message),
        ConformanceStatus::Skip => Finding::skip(check.check_name, check.message),
        ConformanceStatus::Fail => Finding::fail(
            check.check_name,
            check.message,
            format!(
                "expected: {}; observed: {}; remediation: {}",
                check.expected, check.observed, check.remediation
            ),
        ),
    };
    finding.with_conformance(expected, observed, remediation)
}

fn lifecycle_executable_current(observation: &SystemObservation) -> Finding {
    if observation.enabled_hosts.is_empty() {
        return disabled_host_skip(
            "lifecycle-executable-current",
            "no enabled workspace lifecycle adapter requires the AGS executable",
            "the `ags` resolved by workspace hooks equals the running Doctor binary",
        );
    }
    let current = std::env::current_exe();
    let resolved = ags_platform::find_in_path("ags");
    let observed = format!("running={current:?}; PATH={resolved:?}");
    let current_hash = current
        .as_deref()
        .map_err(|error| error.to_string())
        .and_then(ags_platform::executable_content_hash);
    let resolved_hash = resolved
        .as_deref()
        .ok_or_else(|| "`ags` is not on PATH".to_string())
        .and_then(ags_platform::executable_content_hash);
    let passed = matches!(
        (&current_hash, &resolved_hash),
        (Ok(current), Ok(resolved)) if current == resolved
    );
    conformance_verdict(
        passed,
        "lifecycle-executable-current",
        "workspace lifecycle hooks resolve the current AGS executable",
        "the `ags` command on the host PATH has the same complete-file hash as this Doctor process",
        match (current_hash, resolved_hash) {
            (Ok(current), Ok(resolved)) => {
                format!("{observed}; running_hash={current}; PATH_hash={resolved}")
            }
            (current, resolved) => {
                format!("{observed}; running_hash={current:?}; PATH_hash={resolved:?}")
            }
        },
        "Install the current AGS executable on the host PATH, then restart the host.",
    )
}

fn workspace_daemon_current(repo_root: &Path, observation: &SystemObservation) -> Finding {
    if observation.enabled_hosts.is_empty() {
        return disabled_host_skip(
            "workspace-daemon-current",
            "no enabled workspace lifecycle adapter requires a daemon",
            "one current daemon for an enabled workspace",
        );
    }
    let expected_identity = ags_lifecycle::workspace_lifecycle::workspace_identity(repo_root);
    match (
        observation.daemon_status.as_ref(),
        observation.daemon.as_ref(),
    ) {
        (Some(Ok(status)), Some(Ok(Some(inspection))))
            if status.state == "running"
                && status.current_binary
                && inspection.workspace_identity == expected_identity =>
        {
            Finding::pass(
                "workspace-daemon-current",
                format!(
                    "workspace daemon is current with identity {} and executable {}",
                    inspection.workspace_identity, status.current_executable_hash
                ),
            )
            .with_conformance(
                "one running daemon from the current AGS binary with matching workspace identity",
                format!(
                    "state={} current_binary={} identity={}",
                    status.state, status.current_binary, inspection.workspace_identity
                ),
                "none",
            )
        }
        (status, daemon) => conformance_fail(
            "workspace-daemon-current",
            "workspace daemon is absent, stale, unreachable, or not current",
            format!("state=running current_binary=true identity={expected_identity}"),
            format!("status={status:?}; inspection={daemon:?}"),
            format!("Run `ags mcp restart --target '{}'`.", repo_root.display()),
        ),
    }
}

fn managed_projection_current(repo_root: &Path, observation: &SystemObservation) -> Finding {
    let identity = ags_workspace_facts::detect_project(repo_root);
    if identity.is_ags_suite {
        return Finding::pass(
            "managed-projection-current",
            "suite authority is the canonical project projection source",
        )
        .with_conformance(
            "suite source is its own canonical projection",
            "suite authority",
            "none",
        );
    }
    let runtime = ags_platform::runtime_home();
    let source = match capability_authority_root(repo_root, &runtime) {
        Ok(path) => path,
        Err(error) => {
            return conformance_fail(
                "managed-projection-current",
                "cannot resolve the canonical AGS projection authority",
                "a current installed AGS projection authority",
                error.to_string(),
                "Run Doctor from an installed AGS environment.",
            )
        }
    };
    let slug = ags_host_integration::resolve_project_slug(repo_root);
    let approved_hosts = observation.approved_hosts.as_deref().unwrap_or_default();
    let refresh = ags_lifecycle::init::refresh_managed_project(
        repo_root,
        &slug,
        &source,
        approved_hosts,
        false,
    );
    conformance_verdict(
        !refresh.drift,
        "managed-projection-current",
        "managed project projection matches the current AGS generator",
        "managed project files equal the current generator dry-run",
        format!(
            "changed={:?}; blocked={:?}",
            refresh.changed_files, refresh.blocked_reasons
        ),
        format!(
            "Run `ags init --target '{}'` to refresh this managed project.",
            repo_root.display()
        ),
    )
}

fn capability_snapshot_current(
    repo_root: &Path,
    home: &Path,
    observation: &SystemObservation,
) -> Finding {
    if observation.enabled_hosts.is_empty() {
        return disabled_host_skip(
            "capability-snapshot-current",
            "no enabled workspace host requires a capability snapshot",
            "current static snapshots for every enabled host",
        );
    }
    let runtime = ags_platform::runtime_home();
    let source = match capability_authority_root(repo_root, &runtime) {
        Ok(path) => path,
        Err(error) => {
            return conformance_fail(
                "capability-snapshot-current",
                "canonical capability source cannot be resolved",
                "a current capability authority root",
                error.to_string(),
                "Restore the AGS runtime install manifest or set AGS_SOURCE_ROOT.",
            )
        }
    };
    let mut expected_hashes = BTreeMap::new();
    let mut failures = Vec::new();
    let host_home = ags_platform::home_dir().unwrap_or_else(|| PathBuf::from("."));
    for host in &observation.enabled_hosts {
        let expected = ags_capability_governance::build_capability_snapshot_with_live_roots_at(
            &source, host, &runtime, &host_home, repo_root,
        );
        let observed = ags_capability_governance::load_static_snapshot(&runtime, host);
        match (expected, observed) {
            (Ok(expected), Ok((observed, _)))
                if expected.snapshot_hash == observed.snapshot_hash
                    && expected.schema_version == observed.schema_version =>
            {
                expected_hashes.insert(host.clone(), expected.snapshot_hash);
            }
            (Ok(expected), Ok((observed, _))) => failures.push(format!(
                "{host}: expected={} observed={}",
                expected.snapshot_hash, observed.snapshot_hash
            )),
            (Err(error), _) => failures.push(format!("{host}: rebuild failed: {error:?}")),
            (_, Err(error)) => failures.push(format!("{host}: load failed: {error:?}")),
        }
    }
    match observation.daemon.as_ref() {
        Some(Ok(Some(daemon))) => {
            for (host, expected) in &expected_hashes {
                match daemon.loaded_snapshot_hashes.get(host) {
                    Some(loaded) if loaded == expected => {}
                    Some(loaded) => failures.push(format!(
                        "{host}: daemon loaded {loaded}, expected {expected}"
                    )),
                    None => failures.push(format!(
                        "{host}: daemon has not loaded the canonical snapshot"
                    )),
                }
            }
        }
        Some(Ok(None)) => failures.push("workspace daemon is not running".to_string()),
        Some(Err(error)) => failures.push(format!("daemon inspection failed: {error}")),
        None => failures.push("daemon snapshot state is unavailable".to_string()),
    }
    conformance_verdict(
        failures.is_empty(),
        "capability-snapshot-current",
        "enabled host snapshots equal a fresh canonical rebuild and daemon state",
        format!(
            "fresh canonical snapshot and matching daemon-loaded hash for {:?}",
            observation.enabled_hosts
        ),
        if failures.is_empty() {
            format!("{expected_hashes:?}")
        } else {
            failures.join("; ")
        },
        format!(
            "Rebuild each failing host snapshot for target '{}', then restart or reconnect its workspace daemon session (home {}).",
            repo_root.display(),
            home.display()
        ),
    )
}

fn capability_authority_root(repo_root: &Path, runtime: &Path) -> Result<PathBuf, String> {
    let explicit = std::env::var_os("AGS_SOURCE_ROOT").map(PathBuf::from);
    ags_capability_governance::resolve_capability_authority_root(repo_root, runtime, explicit)
        .map_err(|error| error.to_string())
}

fn mcp_registration_current(repo_root: &Path, observation: &SystemObservation) -> Finding {
    if observation.enabled_hosts.is_empty() {
        return disabled_host_skip(
            "mcp-registration-current",
            "no enabled workspace host requires MCP registration inspection",
            "current AGS MCP registration for every enabled probeable host",
        );
    }
    let current_executable = std::env::current_exe().ok();
    let current_hash = current_executable
        .as_deref()
        .and_then(|path| ags_platform::executable_content_hash(path).ok());
    let mut failures = Vec::new();
    let mut absent = Vec::new();
    let mut probed = 0;
    for host in &observation.enabled_hosts {
        let Some(spec) = ags_host_integration::platform_spec(host) else {
            failures.push(format!("{host}: unsupported host"));
            continue;
        };
        if spec.mcp_probe.is_none() {
            continue;
        }
        let Some(report) = observation.mcp_reports.get(host) else {
            failures.push(format!("{host}: MCP probe produced no observation"));
            continue;
        };
        if report.status == ags_host_integration::HostProbeStatus::HostUnavailable {
            absent.push(format!("{host}: {}", report.evidence));
            continue;
        }
        probed += 1;
        let current = report.find("ags").is_some_and(|registration| {
            mcp_registration_matches(registration, current_hash.as_deref())
        });
        if !current {
            failures.push(format!("{host}: {}", report.evidence));
        }
    }
    if probed == 0 {
        return Finding::skip(
            "mcp-registration-current",
            if absent.is_empty() {
                "enabled hosts do not expose a read-only MCP registration probe".to_string()
            } else {
                format!(
                    "selected host CLIs are absent; native MCP probes are not applicable ({})",
                    absent.join(", ")
                )
            },
        )
        .with_conformance(
            "current AGS MCP registration for every probeable enabled host",
            "zero probeable enabled hosts",
            "verify the host registration manually",
        );
    }
    if failures.is_empty() && !absent.is_empty() {
        return Finding::warn(
            "mcp-registration-current",
            "available host MCP registrations are current; absent host CLIs were not probed",
            absent.join(", "),
        )
        .with_conformance(
            "current AGS MCP registration for every installed probeable host",
            format!("{probed} installed host probes passed; absent={absent:?}"),
            "Install an absent host CLI before requiring its native MCP probe.",
        );
    }
    conformance_verdict(
        failures.is_empty(),
        "mcp-registration-current",
        "probeable enabled hosts use the current AGS MCP registration",
        "active stdio registration with exact `mcp serve --transport stdio` arguments and current executable hash",
        if failures.is_empty() {
            format!("{probed} current registration(s)")
        } else {
            failures.join("; ")
        },
        format!(
            "Re-register AGS MCP for each failing host and restart target '{}'.",
            repo_root.display()
        ),
    )
}

fn mcp_registration_matches(
    server: &ags_host_integration::McpServerRegistration,
    current_executable_hash: Option<&str>,
) -> bool {
    let canonical_args =
        server.args == ["mcp", "serve"] || server.args == ["mcp", "serve", "--transport", "stdio"];
    if !server.active
        || server.transport.as_deref() != Some("stdio")
        || !canonical_args
        // Claude and Codex both prove that this registration is effective in
        // the current workspace, but only Claude's detail command exposes the
        // origin scope. Reject a scope when one is reported and invalid; do
        // not turn a native protocol omission into a permanent false failure.
        || server
            .scope
            .as_deref()
            .is_some_and(|scope| !matches!(scope, "user" | "project" | "workspace" | "local"))
    {
        return false;
    }
    let registered = server.command.as_deref().and_then(|command| {
        let path = PathBuf::from(command);
        if path.is_absolute() && path.file_stem().is_some_and(|name| name == "ags") {
            Some(path)
        } else if command == "ags" {
            ags_platform::find_in_path("ags")
        } else {
            None
        }
    });
    registered
        .as_deref()
        .and_then(|registered| ags_platform::executable_content_hash(registered).ok())
        .zip(current_executable_hash)
        .is_some_and(|(registered, current)| registered == current)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RemoteLatestCache {
    checked_at_unix: u64,
    version: String,
}

fn remote_latest_advisory() -> Finding {
    const CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;
    let current = env!("CARGO_PKG_VERSION");
    if cfg!(test)
        || std::env::var_os("AGS_REMOTE_LATEST_OFFLINE").is_some()
        || std::env::var_os("AGS_THIRD_PARTY_MANIFEST_OFFLINE").is_some()
    {
        return remote_latest_finding(current, None, true);
    }
    let cache = ags_platform::runtime_home()
        .join("cache")
        .join("remote-latest.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let cached = std::fs::read(&cache)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RemoteLatestCache>(&bytes).ok())
        .filter(|entry| now.saturating_sub(entry.checked_at_unix) <= CACHE_TTL_SECONDS);
    let latest = if let Some(cached) = cached {
        Some(cached.version)
    } else {
        let output = std::process::Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--connect-timeout",
                "1",
                "--max-time",
                "2",
                "https://registry.npmjs.org/@agent-governance-suite%2Fmcp/latest",
            ])
            .output();
        let version = output
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
            .and_then(|value| {
                value
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        if let Some(version) = &version {
            let entry = RemoteLatestCache {
                checked_at_unix: now,
                version: version.clone(),
            };
            if let Ok(body) = serde_json::to_vec_pretty(&entry) {
                let _ = ags_platform::atomic_write(&cache, &body);
            }
        }
        version
    };
    remote_latest_finding(current, latest.as_deref(), false)
}

fn remote_latest_finding(current: &str, latest: Option<&str>, offline: bool) -> Finding {
    const CHECK: &str = "remote-latest-advisory";
    if offline {
        return remote_latest_skip("remote latest check is offline by configuration", "offline");
    }
    let Some(latest) = latest else {
        return remote_latest_skip(
            "remote latest version is unavailable; local conformance is unaffected",
            "unavailable",
        );
    };
    if version_is_newer(latest, current) {
        Finding::warn(
            CHECK,
            format!("a newer published AGS version is available: {latest}"),
            format!("current={current}; latest={latest}; advisory only"),
        )
        .with_conformance(
            "local version may lag upstream without failing local conformance",
            format!("current={current}; latest={latest}"),
            "review the release before updating",
        )
    } else {
        Finding::pass(
            CHECK,
            format!("published latest {latest} does not supersede local {current}"),
        )
        .with_conformance(
            "cached or freshly probed upstream version metadata",
            format!("current={current}; latest={latest}"),
            "none",
        )
    }
}

fn remote_latest_skip(message: &str, observed: &str) -> Finding {
    Finding::skip("remote-latest-advisory", message).with_conformance(
        "remote latest is advisory only",
        observed,
        "none; retry when network access is available",
    )
}

fn conformance_verdict(
    passed: bool,
    name: impl Into<String>,
    message: impl Into<String>,
    expected: impl Into<String>,
    observed: impl Into<String>,
    remediation: impl Into<String>,
) -> Finding {
    let name = name.into();
    let message = message.into();
    let expected = expected.into();
    let observed = observed.into();
    let remediation = remediation.into();
    if passed {
        Finding::pass(name, message).with_conformance(expected, observed, "none")
    } else {
        Finding::fail(
            name,
            message,
            format!("expected: {expected}; observed: {observed}; remediation: {remediation}"),
        )
        .with_conformance(expected, observed, remediation)
    }
}

fn conformance_fail(
    name: impl Into<String>,
    message: impl Into<String>,
    expected: impl Into<String>,
    observed: impl Into<String>,
    remediation: impl Into<String>,
) -> Finding {
    conformance_verdict(false, name, message, expected, observed, remediation)
}

fn disabled_host_skip(
    name: impl Into<String>,
    message: impl Into<String>,
    expected: impl Into<String>,
) -> Finding {
    Finding::skip(name, message).with_conformance(
        expected,
        "no enabled host",
        "none unless a host should be enabled",
    )
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    fn parts(version: &str) -> Option<Vec<u64>> {
        let stable = version
            .trim_start_matches('v')
            .split_once('-')
            .map_or(version.trim_start_matches('v'), |(stable, _)| stable);
        stable
            .split('.')
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    }
    parts(candidate)
        .zip(parts(current))
        .is_some_and(|(candidate, current)| candidate > current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::{CheckStatus, Severity};

    #[test]
    fn remote_latest_newer_and_offline_remain_non_blocking() {
        let newer = remote_latest_finding("0.4.0", Some("0.4.1"), false);
        assert_eq!(newer.severity, Severity::Warn);
        let offline = remote_latest_finding("0.4.0", None, true);
        assert_eq!(offline.status, CheckStatus::Skip);
    }

    #[test]
    fn mcp_registration_requires_exact_transport_args_and_binary() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("ags");
        let old = root.path().join("ags-old");
        std::fs::write(&current, b"current").unwrap();
        std::fs::write(&old, b"old").unwrap();
        let current_hash = ags_platform::executable_content_hash(&current).unwrap();
        let registration = |command: &Path, args: &[&str]| {
            serde_json::from_value::<ags_host_integration::McpServerRegistration>(
                serde_json::json!({
                    "name": "ags",
                    "command": command,
                    "args": args,
                    "transport": "stdio",
                    "scope": "workspace",
                    "active": true,
                    "evidence": "test"
                }),
            )
            .unwrap()
        };
        assert!(mcp_registration_matches(
            &registration(&current, &["mcp", "serve", "--transport", "stdio"]),
            Some(&current_hash)
        ));
        assert!(!mcp_registration_matches(
            &registration(&old, &["mcp", "serve", "--transport", "stdio"]),
            Some(&current_hash)
        ));
        assert!(!mcp_registration_matches(
            &registration(&current, &["mcp", "start"]),
            Some(&current_hash)
        ));
        let mut disabled = registration(&current, &["mcp", "serve", "--transport", "stdio"]);
        disabled.active = false;
        assert!(!mcp_registration_matches(&disabled, Some(&current_hash)));

        let unreadable_registered = root.path().join("missing-registered/ags");
        assert!(!mcp_registration_matches(
            &registration(
                &unreadable_registered,
                &["mcp", "serve", "--transport", "stdio"]
            ),
            None
        ));
    }

    #[test]
    fn absent_host_probe_skips_but_installed_broken_probe_fails() {
        let root = tempfile::tempdir().unwrap();
        let report = |status| ags_host_integration::HostMcpReport {
            host: "claude-code".to_string(),
            status,
            evidence_source: "fixture".to_string(),
            servers: Vec::new(),
            evidence: "fixture host probe".to_string(),
        };
        let observation = |report| SystemObservation {
            lifecycle: Err("unused fixture".to_string()),
            enabled_hosts: vec!["claude-code".to_string()],
            approved_hosts: Err("unused fixture".to_string()),
            mcp_reports: BTreeMap::from([("claude-code".to_string(), report)]),
            daemon_status: None,
            daemon: None,
        };
        let absent = mcp_registration_current(
            root.path(),
            &observation(report(
                ags_host_integration::HostProbeStatus::HostUnavailable,
            )),
        );
        assert_eq!(absent.status, CheckStatus::Skip);
        let broken = mcp_registration_current(
            root.path(),
            &observation(report(
                ags_host_integration::HostProbeStatus::ConnectionFailed,
            )),
        );
        assert_eq!(broken.status, CheckStatus::Fail);
    }
}
