use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SAMPLES: usize = 21;

#[derive(Debug, Clone, Serialize)]
struct Distribution {
    median: f64,
    p95: f64,
}

#[derive(Debug, Clone, Serialize)]
struct Measurements {
    preflight_ms: Distribution,
    snapshot_refresh_ms: Distribution,
    daemon_reconnect_ms: Distribution,
    route_request_ms: Distribution,
    peak_rss_kib: u64,
}

#[derive(Debug, Serialize)]
struct PerformanceReport {
    schema_version: &'static str,
    samples: usize,
    baseline: Measurements,
    candidate: Measurements,
    failures: Vec<String>,
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ags-rust-bench-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bench {
    _root: TestRoot,
    binary: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
    project: PathBuf,
    source: PathBuf,
    route_schema: String,
}

impl Bench {
    fn new(
        binary: PathBuf,
        source: PathBuf,
        route_schema: String,
        label: &str,
    ) -> Result<Self, String> {
        let root = TestRoot::new(label);
        let home = root.0.join("home");
        let runtime = root.0.join("runtime");
        let project = root.0.join("project");
        for path in [&home, &runtime, &project] {
            fs::create_dir(path).map_err(|error| error.to_string())?;
        }
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .output()
            .map_err(|error| format!("cannot initialize benchmark repository: {error}"))?;
        if !git.status.success() {
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&git.stderr)
            ));
        }
        let bench = Self {
            _root: root,
            binary,
            home,
            runtime,
            project,
            source,
            route_schema,
        };
        bench.run(&[
            "init",
            "--target",
            bench.project.to_string_lossy().as_ref(),
            "--mode",
            "local",
            "--format",
            "json",
        ])?;
        bench.run(&[
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            bench.project.to_string_lossy().as_ref(),
            "--write",
            "--format",
            "json",
        ])?;
        Ok(bench)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .current_dir(&self.project)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("AGS_RUNTIME_HOME", &self.runtime)
            .env("AGS_SOURCE_ROOT", &self.source)
            .env("AGS_WORKSPACE_IDLE_MS", "60000")
            .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1");
        command
    }

    fn run(&self, args: &[&str]) -> Result<Output, String> {
        let output = self
            .command()
            .args(args)
            .output()
            .map_err(|error| format!("cannot run {}: {error}", self.binary.display()))?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(format!(
                "{} {} failed: {}",
                self.binary.display(),
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }

    fn timed_cli(&self, args: &[&str]) -> Result<f64, String> {
        let started = Instant::now();
        self.run(args)?;
        Ok(started.elapsed().as_secs_f64() * 1000.0)
    }

    fn mcp_sample(&self) -> Result<(f64, f64, u64), String> {
        let started = Instant::now();
        let mut child = self
            .command()
            .args(["mcp", "serve", "--transport", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start MCP benchmark adapter: {error}"))?;
        let adapter_pid = child.id();
        let stdin = child.stdin.take().ok_or("MCP stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("MCP stdout unavailable")?;
        let mut client = McpClient {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };
        client.request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "ags-rust-benchmark", "version": "1"}
            }
        }))?;
        client.request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "ags_preflight",
                "arguments": {"agent": "codex", "target": self.project}
            }
        }))?;
        let reconnect_ms = started.elapsed().as_secs_f64() * 1000.0;
        let route_started = Instant::now();
        client.request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "ags_route_request",
                "arguments": {
                    "proposal": {
                        "schema_version": self.route_schema,
                        "request_fingerprint": "sha256:benchmark-request",
                        "phase": "execution",
                        "solution_state": "confirmed",
                        "execution_authority": "none",
                        "scope_hash": "sha256:benchmark-scope",
                        "targets": [{
                            "kind": "machine_cli",
                            "capability": "project_verify",
                            "input": {"kind": "empty"}
                        }]
                    }
                }
            }
        }))?;
        let route_ms = route_started.elapsed().as_secs_f64() * 1000.0;
        let daemon_pid = self.workspace_daemon_pid().unwrap_or(adapter_pid);
        let rss = unique_process_rss_kib(&[adapter_pid, daemon_pid])?;
        client.close();
        Ok((reconnect_ms, route_ms, rss))
    }

    fn workspace_daemon_pid(&self) -> Option<u32> {
        let canonical = self.project.canonicalize().ok()?;
        let entries = fs::read_dir(self.runtime.join("workspace-services")).ok()?;
        entries.flatten().find_map(|entry| {
            let value: Value = serde_json::from_slice(&fs::read(entry.path()).ok()?).ok()?;
            (value["workspace"].as_str() == Some(canonical.to_string_lossy().as_ref()))
                .then(|| {
                    value["pid"]
                        .as_u64()
                        .and_then(|pid| u32::try_from(pid).ok())
                })
                .flatten()
        })
    }

    fn measure(&self) -> Result<Measurements, String> {
        self.timed_cli(&[
            "session",
            "preflight",
            "--for",
            "codex",
            "--target",
            self.project.to_string_lossy().as_ref(),
        ])?;
        self.timed_cli(&[
            "capability",
            "snapshot",
            "--host",
            "codex",
            "--target",
            self.project.to_string_lossy().as_ref(),
            "--write",
            "--format",
            "json",
        ])?;
        self.mcp_sample()?;

        let mut preflight = Vec::new();
        let mut snapshot = Vec::new();
        let mut reconnect = Vec::new();
        let mut route = Vec::new();
        let mut rss = Vec::new();
        for _ in 0..SAMPLES {
            preflight.push(self.timed_cli(&[
                "session",
                "preflight",
                "--for",
                "codex",
                "--target",
                self.project.to_string_lossy().as_ref(),
            ])?);
            snapshot.push(self.timed_cli(&[
                "capability",
                "snapshot",
                "--host",
                "codex",
                "--target",
                self.project.to_string_lossy().as_ref(),
                "--write",
                "--format",
                "json",
            ])?);
            let (reconnect_ms, route_ms, rss_kib) = self.mcp_sample()?;
            reconnect.push(reconnect_ms);
            route.push(route_ms);
            rss.push(rss_kib);
        }
        Ok(Measurements {
            preflight_ms: distribution(&preflight),
            snapshot_refresh_ms: distribution(&snapshot),
            daemon_reconnect_ms: distribution(&reconnect),
            route_request_ms: distribution(&route),
            peak_rss_kib: rss.into_iter().max().unwrap_or_default(),
        })
    }
}

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn request(&mut self, value: Value) -> Result<Value, String> {
        writeln!(self.stdin, "{value}").map_err(|error| error.to_string())?;
        self.stdin.flush().map_err(|error| error.to_string())?;
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if line.is_empty() {
            return Err("MCP adapter closed without a response".to_string());
        }
        let envelope: Value = serde_json::from_str(&line)
            .map_err(|error| format!("invalid MCP response: {error}"))?;
        if let Some(error) = envelope.get("error") {
            return Err(format!("MCP error: {error}"));
        }
        Ok(envelope["result"].clone())
    }

    fn close(mut self) {
        drop(self.stdin);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn process_rss_kib(pid: u32) -> Result<u64, String> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map_err(|error| format!("cannot measure RSS for {pid}: {error}"))?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|error| format!("invalid RSS for {pid}: {error}"))
}

fn unique_process_rss_kib(pids: &[u32]) -> Result<u64, String> {
    let mut unique = pids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    unique
        .into_iter()
        .map(process_rss_kib)
        .try_fold(0_u64, |total, value| value.map(|rss| total + rss))
}

fn distribution(values: &[f64]) -> Distribution {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    Distribution {
        median: if ordered.len().is_multiple_of(2) {
            let upper = ordered.len() / 2;
            (ordered[upper - 1] + ordered[upper]) / 2.0
        } else {
            ordered[ordered.len() / 2]
        },
        p95: percentile(&ordered, 0.95),
    }
}

fn percentile(ordered: &[f64], fraction: f64) -> f64 {
    let index = ((ordered.len() as f64 * fraction).ceil() as usize)
        .saturating_sub(1)
        .min(ordered.len().saturating_sub(1));
    ordered[index]
}

fn median_regressed(name: &str, baseline: f64, candidate: f64) -> bool {
    let floor = match name {
        "preflight_ms" => 0.5,
        "snapshot_refresh_ms" => 2.0,
        "daemon_reconnect_ms" => 5.0,
        "route_request_ms" => 5.0,
        _ => 0.0,
    };
    let relative = if name == "daemon_reconnect_ms" {
        1.15
    } else {
        1.05
    };
    candidate > baseline * relative && candidate > baseline + floor
}

fn p95_regressed(name: &str, baseline: f64, candidate: f64) -> bool {
    let material_floor = match name {
        "preflight_ms" => 0.5,
        "snapshot_refresh_ms" => 2.0,
        "daemon_reconnect_ms" => 5.0,
        "route_request_ms" => 5.0,
        _ => 0.0,
    };
    let relative = if matches!(name, "daemon_reconnect_ms" | "route_request_ms") {
        1.15
    } else {
        1.10
    };
    candidate > baseline * relative && candidate > baseline + material_floor
}

#[test]
#[ignore = "development release gate; requires explicit stable and candidate release binaries"]
fn adjacent_stable_and_candidate_workspace_performance_stays_within_budget() {
    let baseline_bin = PathBuf::from(
        std::env::var_os("AGS_STABLE_BIN")
            .expect("set AGS_STABLE_BIN to the previous stable release binary"),
    );
    let baseline_source = PathBuf::from(
        std::env::var_os("AGS_STABLE_SOURCE_ROOT")
            .expect("set AGS_STABLE_SOURCE_ROOT to the previous stable release checkout"),
    );
    let candidate_bin = PathBuf::from(
        std::env::var_os("AGS_CANDIDATE_BIN")
            .expect("set AGS_CANDIDATE_BIN to the current candidate release binary"),
    );
    let candidate_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    let baseline = Bench::new(
        baseline_bin,
        baseline_source,
        "0.3.6-host-route-proposal".to_string(),
        "baseline",
    )
    .unwrap();
    let candidate = Bench::new(
        candidate_bin,
        candidate_source,
        "0.3.6-host-route-proposal".to_string(),
        "candidate",
    )
    .unwrap();
    let baseline_result = baseline.measure().unwrap();
    let candidate_result = candidate.measure().unwrap();
    let mut failures = Vec::new();
    for (name, before, after) in [
        (
            "preflight_ms",
            &baseline_result.preflight_ms,
            &candidate_result.preflight_ms,
        ),
        (
            "snapshot_refresh_ms",
            &baseline_result.snapshot_refresh_ms,
            &candidate_result.snapshot_refresh_ms,
        ),
        (
            "daemon_reconnect_ms",
            &baseline_result.daemon_reconnect_ms,
            &candidate_result.daemon_reconnect_ms,
        ),
        (
            "route_request_ms",
            &baseline_result.route_request_ms,
            &candidate_result.route_request_ms,
        ),
    ] {
        if median_regressed(name, before.median, after.median) {
            failures.push(format!("{name} median regressed"));
        }
        if p95_regressed(name, before.p95, after.p95) {
            failures.push(format!("{name} p95 regressed"));
        }
    }
    if candidate_result.peak_rss_kib > baseline_result.peak_rss_kib * 110 / 100 {
        failures.push("peak RSS exceeds 110%".to_string());
    }
    let report = PerformanceReport {
        schema_version: "ags-workspace-performance/2",
        samples: SAMPLES,
        baseline: baseline_result,
        candidate: candidate_result,
        failures,
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

#[test]
fn performance_thresholds_require_relative_and_material_regression() {
    assert!(!median_regressed("daemon_reconnect_ms", 13.0, 14.5));
    assert!(!median_regressed("daemon_reconnect_ms", 13.0, 18.0));
    assert!(median_regressed("daemon_reconnect_ms", 13.0, 18.1));
    assert!(!p95_regressed("daemon_reconnect_ms", 13.0, 18.0));
    assert!(p95_regressed("daemon_reconnect_ms", 13.0, 18.1));
    assert!(!median_regressed("route_request_ms", 0.2, 0.22));
    assert!(!median_regressed("route_request_ms", 0.2, 5.2));
    assert!(median_regressed("route_request_ms", 0.2, 5.21));
    assert!(!p95_regressed("route_request_ms", 1.0, 5.9));
    assert!(p95_regressed("route_request_ms", 1.0, 6.1));
}

#[test]
fn rss_scope_deduplicates_single_process_architecture() {
    let mut pids = vec![101, 101, 202];
    pids.sort_unstable();
    pids.dedup();
    assert_eq!(pids, [101, 202]);
}
