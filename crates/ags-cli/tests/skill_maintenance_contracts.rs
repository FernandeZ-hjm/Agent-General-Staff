use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const UNLISTED_GITHUB_URL: &str =
    "https://github.com/ags-contract-fixtures/not-in-recommendations.git";

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ags-skill-maintenance-contract-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create skill maintenance fixture directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn run_ags(args: &[&str], fixture: &TestDir) -> Output {
    let home = fixture.path().join("home");
    let runtime = home.join(".ags/private-runtime");
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .current_dir(repo_root())
        .env("HOME", home)
        .env("AGS_RUNTIME_HOME", runtime)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run isolated ags CLI")
}

fn assert_success(output: &Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_json(output: &Output, label: &str) -> Value {
    assert_success(output, label);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{label} did not emit JSON: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn has_subcommand(help: &str, expected: &str) -> bool {
    help.lines()
        .filter_map(|line| line.split_whitespace().next())
        .any(|name| name == expected)
}

fn find_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(object) => object
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| object.values().find_map(|child| find_string(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find_string(child, key)),
        _ => None,
    }
}

fn find_value<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(object) => object
            .get(key)
            .or_else(|| object.values().find_map(|child| find_value(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find_value(child, key)),
        _ => None,
    }
}

fn contains_string(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(actual) => actual.contains(expected),
        Value::Array(items) => items.iter().any(|child| contains_string(child, expected)),
        Value::Object(object) => object
            .values()
            .any(|child| contains_string(child, expected)),
        _ => false,
    }
}

fn risk_finding_mentions(value: &Value, expected: &str) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            let risk_key = key.to_ascii_lowercase();
            (risk_key.contains("risk")
                || risk_key.contains("finding")
                || risk_key.contains("warning"))
                && contains_string(child, expected)
                || risk_finding_mentions(child, expected)
        }),
        Value::Array(items) => items
            .iter()
            .any(|child| risk_finding_mentions(child, expected)),
        _ => false,
    }
}

fn plan_hash(value: &Value) -> &str {
    find_string(value, "plan_hash").expect("maintenance plan exposes plan_hash")
}

fn acknowledged_risk_args(plan: &Value) -> Vec<String> {
    find_value(plan, "risks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|finding| {
            finding
                .get("class")
                .and_then(Value::as_str)
                .is_some_and(|class| class == "acknowledgement-required")
        })
        .filter_map(|finding| {
            finding
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flat_map(|id| ["--ack-risk".to_string(), id])
        .collect()
}

fn run_ags_owned(args: &[String], fixture: &TestDir) -> Output {
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_ags(&borrowed, fixture)
}

fn is_plan_hash(value: &str) -> bool {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-C", repo.to_str().expect("UTF-8 fixture path")])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "AGS contract fixture")
        .env("GIT_AUTHOR_EMAIL", "ags-contract-fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "AGS contract fixture")
        .env(
            "GIT_COMMITTER_EMAIL",
            "ags-contract-fixture@example.invalid",
        )
        .output()
        .expect("run local git fixture command");
    assert!(
        output.status.success(),
        "git fixture command {:?} failed:\n{}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn source_fixture(fixture: &TestDir, with_installer: bool) -> (PathBuf, PathBuf) {
    let repo = fixture.path().join("skill-source");
    fs::create_dir_all(&repo).expect("create local skill repository");
    fs::write(
        repo.join("SKILL.md"),
        "---\nname: unlisted-contract-skill\ndescription: A local file-backed contract fixture.\nversion: 1.0.0\n---\n\n# Contract fixture\n",
    )
    .expect("write fixture skill body");
    fs::write(repo.join("LICENSE"), "MIT fixture license\n").expect("write fixture license");

    let marker = fixture.path().join("installer-ran.marker");
    if with_installer {
        fs::write(
            repo.join("install.sh"),
            "#!/bin/sh\nprintf 'installer executed' > \"$AGS_CONTRACT_INSTALL_MARKER\"\n",
        )
        .expect("write installer fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(repo.join("install.sh"))
                .expect("read installer permissions")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(repo.join("install.sh"), permissions)
                .expect("mark installer executable");
        }
    }

    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "AGS contract fixture"]);
    git(
        &repo,
        &[
            "config",
            "user.email",
            "ags-contract-fixture@example.invalid",
        ],
    );
    git(&repo, &["remote", "add", "origin", UNLISTED_GITHUB_URL]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "fixture-v1"]);
    (repo, marker)
}

fn state_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        if !current.exists() {
            return;
        }
        let mut entries = fs::read_dir(current)
            .expect("read maintenance state directory")
            .map(|entry| entry.expect("read maintenance state entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("state path under state root")
                .to_string_lossy()
                .into_owned();
            let metadata = fs::symlink_metadata(&path).expect("inspect maintenance state entry");
            if metadata.file_type().is_symlink() {
                result.insert(
                    relative,
                    format!(
                        "symlink:{}",
                        fs::read_link(path).expect("read state symlink").display()
                    )
                    .into_bytes(),
                );
            } else if metadata.is_dir() {
                visit(root, &path, result);
            } else if metadata.is_file() {
                result.insert(
                    relative,
                    fs::read(path).expect("read maintenance state file"),
                );
            }
        }
    }

    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn skill_maintenance_cli_surface_exposes_the_requested_actions() {
    let fixture = TestDir::new("surface");
    let output = run_ags(&["skill", "--help"], &fixture);
    assert_success(&output, "skill help");
    let help = String::from_utf8_lossy(&output.stdout);
    for action in [
        "recommend",
        "inspect",
        "install",
        "check",
        "update",
        "rollback",
    ] {
        assert!(
            has_subcommand(&help, action),
            "skill CLI is missing requested action `{action}`:\n{help}"
        );
    }
}

#[test]
fn unlisted_github_source_enters_inspect_plan_and_scripts_are_only_risks() {
    let fixture = TestDir::new("inspect-unlisted");
    let (repo, marker) = source_fixture(&fixture, true);
    // The local checkout keeps this test file-backed and offline; its origin
    // remote is the deliberately unlisted GitHub identity under test.
    let marker_arg = marker.to_str().expect("UTF-8 marker path");
    let output = Command::new(env!("CARGO_BIN_EXE_ags"))
        .args([
            "skill",
            "inspect",
            repo.to_str().expect("UTF-8 fixture path"),
            "--host",
            "codex",
            "--format",
            "json",
        ])
        .current_dir(repo_root())
        .env("HOME", fixture.path().join("home"))
        .env(
            "AGS_RUNTIME_HOME",
            fixture.path().join("home/.ags/private-runtime"),
        )
        .env("AGS_CONTRACT_INSTALL_MARKER", marker_arg)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run isolated inspect command");
    let inspected = parse_json(&output, "unlisted local-source inspect");
    assert!(
        contains_string(&inspected, UNLISTED_GITHUB_URL),
        "inspect must preserve the local fixture's unlisted GitHub source identity: {inspected}"
    );
    assert!(
        is_plan_hash(plan_hash(&inspected)),
        "inspect must enter the reviewed plan path: {inspected}"
    );
    assert!(
        risk_finding_mentions(&inspected, "install.sh"),
        "an installer must be reported as a risk finding, not treated as an action: {inspected}"
    );
    assert!(
        !marker.exists(),
        "inspect/install planning must never execute a third-party installer"
    );
}

#[test]
fn adopt_apply_fails_closed_without_the_exact_plan_hash() {
    let fixture = TestDir::new("plan-hash");
    let (repo, _) = source_fixture(&fixture, false);
    let source_arg = repo.to_str().expect("UTF-8 fixture path");
    let state_root = fixture.path();

    let no_target = run_ags(
        &["skill", "adopt", source_arg, "--format", "json"],
        &fixture,
    );
    let no_target_error: Value =
        serde_json::from_slice(&no_target.stdout).unwrap_or_else(|error| {
            panic!(
                "target-host rejection did not emit JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&no_target.stdout),
                String::from_utf8_lossy(&no_target.stderr)
            )
        });
    assert!(
        !no_target.status.success()
            && contains_string(&no_target_error, "skill_install_requires_target_host"),
        "a Skill install must select an approved or explicit target Host: {no_target_error}"
    );

    let plan = parse_json(
        &run_ags(
            &[
                "skill", "adopt", source_arg, "--host", "codex", "--format", "json",
            ],
            &fixture,
        ),
        "local adopt plan",
    );
    let reviewed_hash = plan_hash(&plan).to_string();
    assert!(
        find_string(&plan, "content_hash").is_some(),
        "adopt plans must expose the audited body hash: {plan}"
    );
    let before_apply = state_snapshot(state_root);

    let missing_hash = run_ags(
        &["skill", "adopt", source_arg, "--yes", "--format", "json"],
        &fixture,
    );
    assert!(
        !missing_hash.status.success(),
        "adopt apply without a plan hash must be rejected"
    );
    assert_eq!(
        state_snapshot(state_root),
        before_apply,
        "missing-hash install apply must not mutate maintenance state"
    );

    let stale_hash = run_ags(
        &[
            "skill",
            "adopt",
            source_arg,
            "--yes",
            "--plan-hash",
            "sha256:stale",
            "--format",
            "json",
        ],
        &fixture,
    );
    assert!(
        !stale_hash.status.success(),
        "adopt apply with a stale plan hash must be rejected"
    );
    assert_eq!(
        state_snapshot(state_root),
        before_apply,
        "stale-hash install apply must not mutate maintenance state"
    );

    let mut install_apply_args = vec![
        "skill".to_string(),
        "adopt".to_string(),
        source_arg.to_string(),
        "--yes".to_string(),
        "--plan-hash".to_string(),
        reviewed_hash,
        "--format".to_string(),
        "json".to_string(),
    ];
    install_apply_args.extend(acknowledged_risk_args(&plan));
    assert_success(
        &run_ags_owned(&install_apply_args, &fixture),
        "local adopt apply",
    );
    let local_update = run_ags(
        &[
            "skill",
            "update",
            "unlisted-contract-skill",
            "--format",
            "json",
        ],
        &fixture,
    );
    assert!(
        !local_update.status.success()
            && String::from_utf8_lossy(&local_update.stderr)
                .contains("local_source_has_no_upstream_update_candidate"),
        "local adoption must not pretend to have an upstream update"
    );
}

#[test]
fn notify_is_the_default_policy_and_pinned_has_no_update_candidate() {
    let notify_fixture = TestDir::new("notify-policy");
    let (notify_repo, _) = source_fixture(&notify_fixture, false);
    let notify_source_arg = notify_repo.to_str().expect("UTF-8 fixture path");
    let notify_plan = parse_json(
        &run_ags(
            &[
                "skill",
                "adopt",
                notify_source_arg,
                "--host",
                "codex",
                "--format",
                "json",
            ],
            &notify_fixture,
        ),
        "default-policy install plan",
    );
    assert_eq!(
        find_string(&notify_plan, "update_policy"),
        Some("notify"),
        "third-party skill updates must default to notify"
    );
    let notify_hash = plan_hash(&notify_plan).to_string();
    let mut notify_apply_args = vec![
        "skill".to_string(),
        "adopt".to_string(),
        notify_source_arg.to_string(),
        "--yes".to_string(),
        "--plan-hash".to_string(),
        notify_hash,
        "--format".to_string(),
        "json".to_string(),
    ];
    notify_apply_args.extend(acknowledged_risk_args(&notify_plan));
    assert_success(
        &run_ags_owned(&notify_apply_args, &notify_fixture),
        "default-policy install apply",
    );
    let notify_check = parse_json(
        &run_ags(
            &[
                "skill",
                "check",
                "unlisted-contract-skill",
                "--format",
                "json",
            ],
            &notify_fixture,
        ),
        "default-policy skill check",
    );
    assert_eq!(find_string(&notify_check, "update_policy"), Some("notify"));
    assert_eq!(
        find_string(&notify_check, "status"),
        Some("local_source_reinstall_required")
    );

    let pinned_fixture = TestDir::new("pinned-policy");
    let (pinned_repo, _) = source_fixture(&pinned_fixture, false);
    let pinned_source_arg = pinned_repo.to_str().expect("UTF-8 fixture path");
    let pinned_plan = parse_json(
        &run_ags(
            &[
                "skill",
                "adopt",
                pinned_source_arg,
                "--host",
                "codex",
                "--update-policy",
                "pinned",
                "--format",
                "json",
            ],
            &pinned_fixture,
        ),
        "pinned-policy install plan",
    );
    assert_eq!(find_string(&pinned_plan, "update_policy"), Some("pinned"));
    let pinned_hash = plan_hash(&pinned_plan).to_string();
    let mut pinned_apply_args = vec![
        "skill".to_string(),
        "adopt".to_string(),
        pinned_source_arg.to_string(),
        "--update-policy".to_string(),
        "pinned".to_string(),
        "--yes".to_string(),
        "--plan-hash".to_string(),
        pinned_hash,
        "--format".to_string(),
        "json".to_string(),
    ];
    pinned_apply_args.extend(acknowledged_risk_args(&pinned_plan));
    assert_success(
        &run_ags_owned(&pinned_apply_args, &pinned_fixture),
        "pinned-policy install apply",
    );
    let pinned_check = parse_json(
        &run_ags(
            &[
                "skill",
                "check",
                "unlisted-contract-skill",
                "--format",
                "json",
            ],
            &pinned_fixture,
        ),
        "pinned-policy skill check",
    );
    assert_eq!(find_string(&pinned_check, "update_policy"), Some("pinned"));
    assert!(
        find_value(&pinned_check, "update_candidates")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty),
        "pinned policy must suppress update candidates: {pinned_check}"
    );
}
