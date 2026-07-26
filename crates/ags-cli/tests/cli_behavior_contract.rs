use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const INPUT_FIXTURE_SHA256: &str =
    "sha256:a2374a1d7c24ad97ac420c88ab6606e165454df28fe6466dff8c99f089e3b4d1";
const VALID_FULL_SHA256: &str =
    "sha256:e08f207b0ca39010f5a96c9ac4e0e4d52ec5b7b5e8f9a12ed8b379f59608b361";
const INVALID_ULTRACODE_SHA256: &str =
    "sha256:07360be698269fdd4e68bbc10a7275850e5af10699608539bd4876b9607ce7e9";
const VALID_RECEIPT_SHA256: &str =
    "sha256:201116091de1d502e646aa888ca39abb30073d2e18ba8e9ed6bc3dfeb53e7157";

#[derive(Debug, Deserialize)]
struct Contract {
    schema_version: String,
    baseline_product_version: String,
    baseline_release_tag: String,
    baseline_release_commit: String,
    baseline_executable_sha256: String,
    input_fixture: String,
    input_fixture_sha256: String,
    filesystem_delta_policy: String,
    filesystem_content_change_allowlist: BTreeSet<String>,
    normalization: Vec<String>,
    cases: Vec<BehaviorCase>,
}

#[derive(Debug, Deserialize)]
struct BehaviorCase {
    id: String,
    surface: String,
    #[serde(default)]
    human_root: Option<String>,
    args: Vec<String>,
    stdin_fixture: Option<String>,
    stdin_sha256: Option<String>,
    #[serde(default)]
    argv_fixture: Option<String>,
    #[serde(default)]
    argv_fixture_sha256: Option<String>,
    #[serde(default)]
    machine_capability: Option<String>,
    exit_code: i32,
    output_policy: String,
    stdout: String,
    stderr: String,
    stdout_json_contract: Option<Value>,
    filesystem_delta: Value,
}

#[derive(Debug, Deserialize)]
struct InputFixture {
    schema_version: String,
    files: BTreeMap<String, String>,
    stdin_fixtures: BTreeMap<String, String>,
}

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(input: &InputFixture) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ags-cli-behavior-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create CLI behavior sandbox");

        for (relative, content) in &input.files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture file parent"))
                .expect("create immutable input parent");
            std::fs::write(&path, content).expect("seed immutable Human input");
            #[cfg(unix)]
            if relative.ends_with(".sh") {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .expect("mark fixture script executable");
            }
        }
        for (relative, content) in &input.stdin_fixtures {
            let path = root.join("suite").join(relative);
            std::fs::create_dir_all(path.parent().expect("baseline input parent"))
                .expect("create baseline input parent");
            std::fs::write(path, content).expect("seed immutable v0.3.0 input");
        }
        for directory in ["home", "runtime", "xdg"] {
            std::fs::create_dir_all(root.join(directory)).expect("create sandbox root");
        }
        Self { root }
    }

    fn suite(&self) -> PathBuf {
        self.root.join("suite")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn contract_path() -> OsString {
    if cfg!(windows) {
        std::env::var_os("PATH").unwrap_or_default()
    } else {
        OsString::from("/usr/bin:/bin")
    }
}

fn load_contract() -> Contract {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cli-behavior-v0.3.0.json"
    ))
    .expect("read CLI behavior contract");
    serde_json::from_str(&fixture).expect("parse CLI behavior contract")
}

fn load_input_fixture(contract: &Contract) -> InputFixture {
    let path = repo_root().join(&contract.input_fixture);
    let bytes = std::fs::read(&path).expect("read immutable v0.3.0 input fixture");
    assert_eq!(
        ags_platform::sha256(&bytes),
        INPUT_FIXTURE_SHA256,
        "immutable v0.3.0 Human input fixture drifted"
    );
    assert_eq!(contract.input_fixture_sha256, INPUT_FIXTURE_SHA256);
    let input: InputFixture =
        serde_json::from_slice(&bytes).expect("parse immutable v0.3.0 input fixture");
    assert_eq!(input.schema_version, "ags-cli-behavior-input/1");
    input
}

fn expand_args(args: &[String], root: &Path) -> Vec<String> {
    args.iter()
        .map(|arg| match arg.as_str() {
            "{{root}}" => root.display().to_string(),
            "{{suite}}" => root.join("suite").display().to_string(),
            "{{project}}" => root.join("project").display().to_string(),
            "{{runtime}}" => root.join("runtime").display().to_string(),
            "{{home}}" => root.join("home").display().to_string(),
            _ => arg.clone(),
        })
        .collect()
}

fn receipt_name(candidate: &str) -> bool {
    let Some(stem) = candidate.strip_suffix(".json") else {
        return false;
    };
    let mut suffixes = stem.rsplitn(3, '-');
    let Some(hash) = suffixes.next() else {
        return false;
    };
    let Some(timestamp) = suffixes.next() else {
        return false;
    };
    let Some(prefix) = suffixes.next() else {
        return false;
    };
    prefix.starts_with("ar-")
        && timestamp.len() == 10
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && hash.len() == 16
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_receipt_names(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut offset = 0;
    while let Some(relative_start) = text[offset..].find("ar-") {
        let start = offset + relative_start;
        output.push_str(&text[offset..start]);
        let Some(relative_end) = text[start..].find(".json") else {
            output.push_str(&text[start..]);
            return output;
        };
        let end = start + relative_end + ".json".len();
        let candidate = &text[start..end];
        if receipt_name(candidate) {
            output.push_str("<RECEIPT>.json");
        } else {
            output.push_str(candidate);
        }
        offset = end;
    }
    output.push_str(&text[offset..]);
    normalize_receipt_ids_and_timestamps(&output)
}

fn normalize_receipt_ids_and_timestamps(text: &str) -> String {
    let mut words = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if text[index..].starts_with("unix-")
            && text[index + 5..]
                .get(..10)
                .is_some_and(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
        {
            words.push_str("unix-<TIMESTAMP>");
            index += 15;
            continue;
        }
        if text[index..].starts_with("ar-") {
            let tail = &text[index..];
            let end = tail
                .find(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
                .unwrap_or(tail.len());
            let candidate = &tail[..end];
            let parts = candidate.rsplitn(3, '-').collect::<Vec<_>>();
            if parts.len() == 3
                && parts[0].len() == 16
                && parts[0].bytes().all(|byte| byte.is_ascii_hexdigit())
                && parts[1].len() == 10
                && parts[1].bytes().all(|byte| byte.is_ascii_digit())
            {
                words.push_str("<RECEIPT>");
                index += candidate.len();
                continue;
            }
        }
        let character = text[index..].chars().next().expect("valid UTF-8 boundary");
        words.push(character);
        index += character.len_utf8();
    }
    words
}

fn normalize(bytes: Vec<u8>, root: &Path) -> String {
    let mut text = String::from_utf8(bytes)
        .expect("CLI output must be UTF-8")
        .replace("\r\n", "\n")
        .replace("ags.exe", "ags");
    let home = root.join("home");
    let mut candidates = vec![
        home.display().to_string(),
        home.canonicalize()
            .unwrap_or_else(|_| home.clone())
            .display()
            .to_string(),
        root.display().to_string(),
        root.canonicalize()
            .unwrap_or_else(|_| root.to_path_buf())
            .display()
            .to_string(),
    ];
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.len()));
    candidates.dedup();
    for candidate in candidates {
        let sanitized = candidate
            .trim_matches('/')
            .replace(['/', '\\', '.'], "-")
            .trim_matches('-')
            .to_string();
        text = text.replace(&candidate, "<CONTRACT_ROOT>");
        text = text.replace(&sanitized, "<CONTRACT_ROOT_SANITIZED>");
    }
    normalize_receipt_names(&text)
}

#[cfg(unix)]
fn mode(path: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!(
        "{:o}",
        path.symlink_metadata()
            .expect("entry metadata")
            .permissions()
            .mode()
            & 0o7777
    )
}

#[cfg(not(unix))]
fn mode(_path: &Path) -> String {
    "portable".to_string()
}

fn file_state(path: &Path) -> Value {
    let metadata = path.symlink_metadata().expect("entry metadata");
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(path).expect("read symlink");
        return json!({
            "kind": "symlink",
            "mode": mode(path),
            "target": normalize(
                target.display().to_string().into_bytes(),
                sandbox_root(path),
            ),
        });
    }
    if metadata.is_dir() {
        return json!({"kind": "directory", "mode": mode(path)});
    }
    json!({
        "kind": "file",
        "mode": mode(path),
        "sha256": ags_platform::sha256(normalize_file_bytes(path)),
    })
}

fn normalize_file_bytes(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("read file for content contract");
    match String::from_utf8(bytes.clone()) {
        Ok(text) => normalize(text.into_bytes(), sandbox_root(path)).into_bytes(),
        Err(_) => bytes,
    }
}

fn sandbox_root(path: &Path) -> &Path {
    path.ancestors()
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ags-cli-behavior-"))
        })
        .expect("file belongs to CLI behavior sandbox")
}

fn walk(root: &Path, current: &Path, result: &mut BTreeMap<String, Value>) {
    let mut entries = std::fs::read_dir(current)
        .expect("read sandbox directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect sandbox entries");
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .expect("sandbox-relative entry")
            .to_string_lossy()
            .replace('\\', "/");
        let normalized = normalize_receipt_names(&relative);
        let state = file_state(&path);
        let is_directory = state["kind"] == "directory";
        result.insert(normalized, state);
        if is_directory {
            walk(root, &path, result);
        }
    }
}

fn snapshot(root: &Path) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

fn filesystem_delta(before: &BTreeMap<String, Value>, after: &BTreeMap<String, Value>) -> Value {
    let created = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .map(|path| {
            let mut entry = after[path].clone();
            entry
                .as_object_mut()
                .expect("file state object")
                .insert("path".into(), json!(path));
            entry
        })
        .collect::<Vec<_>>();
    let deleted = before
        .keys()
        .filter(|path| !after.contains_key(*path))
        .map(|path| {
            let mut entry = before[path].clone();
            entry
                .as_object_mut()
                .expect("file state object")
                .insert("path".into(), json!(path));
            entry
        })
        .collect::<Vec<_>>();
    let modified = before
        .keys()
        .filter(|path| {
            after
                .get(*path)
                .is_some_and(|state| state != &before[*path])
        })
        .map(|path| {
            json!({
                "path": path,
                "before": before[path],
                "after": after[path],
            })
        })
        .collect::<Vec<_>>();
    json!({"created": created, "modified": modified, "deleted": deleted})
}

fn delta_shape(delta: &Value) -> BTreeSet<(String, String, String)> {
    let mut shape = BTreeSet::new();
    for operation in ["created", "modified", "deleted"] {
        for entry in delta[operation]
            .as_array()
            .unwrap_or_else(|| panic!("{operation} delta must be an array"))
        {
            let path = entry["path"].as_str().expect("delta path").to_string();
            let kind = if operation == "modified" {
                format!(
                    "{}->{}",
                    entry["before"]["kind"].as_str().expect("before kind"),
                    entry["after"]["kind"].as_str().expect("after kind")
                )
            } else {
                entry["kind"].as_str().expect("entry kind").to_string()
            };
            shape.insert((operation.to_string(), path, kind));
        }
    }
    shape
}

fn fixture_hash(input: &InputFixture, relative: &str) -> String {
    ags_platform::sha256(
        input
            .stdin_fixtures
            .get(relative)
            .unwrap_or_else(|| panic!("missing immutable fixture {relative}"))
            .as_bytes(),
    )
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn values_at_path<'a>(document: &'a Value, path: &str) -> Vec<&'a Value> {
    if path == "$" {
        return vec![document];
    }
    let Some(segments) = path.strip_prefix("$.") else {
        panic!("unsupported JSON contract path: {path}");
    };
    let mut current = vec![document];
    for segment in segments.split('.') {
        let array_items = segment.ends_with("[]");
        let key = segment.strip_suffix("[]").unwrap_or(segment);
        let mut next = Vec::new();
        for value in current {
            let Some(child) = value.as_object().and_then(|object| object.get(key)) else {
                continue;
            };
            if array_items {
                if let Some(items) = child.as_array() {
                    next.extend(items);
                }
            } else {
                next.push(child);
            }
        }
        current = next;
    }
    current
}

fn assert_json_contract(actual: &str, contract: &Value, case_id: &str) {
    let document: Value = serde_json::from_str(actual)
        .unwrap_or_else(|error| panic!("{case_id} stdout is not JSON: {error}"));
    let baseline: Value = serde_json::from_str(
        contract["baseline"]
            .as_str()
            .expect("captured baseline JSON"),
    )
    .expect("captured baseline remains valid JSON");
    assert_eq!(
        json_shape(&document),
        json_shape(&baseline),
        "{case_id} JSON keys or shapes drifted"
    );
    for path in contract["allowed_product_version_paths"]
        .as_array()
        .expect("explicit product-version exception paths")
    {
        let path = path.as_str().expect("product-version exception path");
        assert_eq!(
            values_at_path(&baseline, path)
                .first()
                .and_then(|value| value.as_str()),
            Some("0.3.0"),
            "only an explicit v0.3.0 product-version scalar may vary at {path}"
        );
        assert_eq!(
            values_at_path(&document, path)
                .first()
                .and_then(|value| value.as_str()),
            Some(env!("CARGO_PKG_VERSION")),
            "{case_id} product version drift at {path}"
        );
    }
    for required in contract["required_types"]
        .as_array()
        .expect("required JSON types")
    {
        let path = required["path"].as_str().expect("required JSON path");
        let expected_type = required["type"].as_str().expect("required JSON type");
        let values = values_at_path(&document, path);
        assert!(
            values
                .iter()
                .any(|candidate| value_type(candidate) == expected_type),
            "{case_id} JSON contract drift: {path} must retain type {expected_type}"
        );
    }
    for assertion in contract["assertions"]
        .as_array()
        .expect("JSON scalar assertions")
    {
        let path = assertion["path"].as_str().expect("JSON assertion path");
        let values = values_at_path(&document, path);
        assert_eq!(
            values,
            vec![&assertion["value"]],
            "{case_id} JSON semantic drift at {path}"
        );
    }
}

fn json_shape(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, child)| (key.clone(), json_shape(child)))
                .collect(),
        ),
        Value::Array(items) => {
            let mut shapes = items.iter().map(json_shape).collect::<Vec<_>>();
            shapes.sort_by_key(|shape| serde_json::to_string(shape).unwrap_or_default());
            shapes.dedup();
            Value::Array(shapes)
        }
        _ => Value::String(value_type(value).to_string()),
    }
}

#[test]
fn v032_preserves_v030_human_and_machine_cli_behavior() {
    let contract = load_contract();
    assert_eq!(contract.schema_version, "ags-cli-behavior-contract/3");
    assert_eq!(contract.baseline_product_version, "0.3.0");
    assert_eq!(contract.baseline_release_tag, "v0.3.0");
    assert_eq!(
        contract.baseline_release_commit,
        "7d7e0477829a9288e97f3f2536a5ba6a8763cd58"
    );
    assert_eq!(
        contract.baseline_executable_sha256,
        "sha256:af4aaf3f396bbb83c9f2bee3cac2c6352df412e4c6a2c9aade6a8417aeb2a7be"
    );
    assert_eq!(
        contract.filesystem_delta_policy,
        "exact-content-hash-unix-mode-symlink-target"
    );
    assert_eq!(
        contract.filesystem_content_change_allowlist,
        BTreeSet::from([
            "project/.gitignore".to_string(),
            "project/AGENTS.md".to_string(),
            "project/AGENT_SUITE_PROTOCOL.md".to_string(),
            "project/CLAUDE.md".to_string(),
            "runtime/managed-projects.yaml".to_string(),
            "runtime/receipts/<RECEIPT>.json".to_string(),
        ])
    );
    assert_eq!(
        contract.normalization,
        [
            "crlf-to-lf",
            "ags.exe-to-ags",
            "sandbox-root-to-contract-root",
            "sandbox-sanitized-root-to-placeholder",
            "receipt-name-to-placeholder",
        ]
    );

    let input = load_input_fixture(&contract);
    assert_eq!(
        fixture_hash(&input, "tests/fixtures/valid-full.md"),
        VALID_FULL_SHA256
    );
    assert_eq!(
        fixture_hash(
            &input,
            "tests/fixtures/invalid-ultracode-authority-abuse.md"
        ),
        INVALID_ULTRACODE_SHA256
    );
    assert_eq!(
        fixture_hash(&input, "tests/fixtures/receipt-valid.json"),
        VALID_RECEIPT_SHA256
    );

    assert_eq!(
        contract
            .cases
            .iter()
            .filter_map(|case| case.human_root.as_deref())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "agents",
            "capability",
            "doctor",
            "init",
            "onboarding",
            "setup",
            "skill",
            "update",
        ])
    );
    assert_eq!(
        contract
            .cases
            .iter()
            .filter_map(|case| case.machine_capability.as_deref())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "PolicyResolve",
            "ProjectVerify",
            "ReceiptVerify",
            "SkillAdopt",
            "SkillTagsVerify",
            "TaskCompile",
            "TaskPrepareExecution",
            "TaskValidate",
        ])
    );
    assert!(contract.cases.iter().all(|case| {
        matches!(
            (
                case.surface.as_str(),
                case.human_root.is_some(),
                case.machine_capability.is_some()
            ),
            ("human", true, false) | ("machine", false, true)
        )
    }));
    assert_eq!(
        contract
            .cases
            .iter()
            .map(|case| case.exit_code)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 1, 2]),
        "the baseline must retain success, refusal, and clap rejection exits"
    );
    for flag in ["--dry-run", "--apply", "--yes", "--force"] {
        assert!(
            contract
                .cases
                .iter()
                .any(|case| case.surface == "human" && case.args.iter().any(|arg| arg == flag)),
            "Human behavior contract does not exercise {flag}"
        );
    }
    assert!(contract.cases.iter().any(|case| {
        case.surface == "human"
            && case.exit_code == 0
            && serde_json::from_str::<Value>(&case.stdout).is_ok()
    }));
    assert!(
        contract
            .cases
            .iter()
            .find(|case| case.id == "human_init_apply_json")
            .is_some_and(|case| !delta_shape(&case.filesystem_delta).is_empty()),
        "the confirmed Human apply baseline must contain a filesystem delta"
    );
    for id in [
        "human_setup_yes_force_dry_run_json",
        "human_init_plan_json",
        "human_skill_adopt_apply_refusal_json",
    ] {
        let case = contract
            .cases
            .iter()
            .find(|case| case.id == id)
            .expect("required no-mutation Human case");
        assert!(
            delta_shape(&case.filesystem_delta).is_empty(),
            "{id} must remain no-mutation"
        );
    }

    for case in contract.cases {
        if let Some(relative) = &case.stdin_fixture {
            assert_eq!(
                case.stdin_sha256.as_deref(),
                Some(fixture_hash(&input, relative).as_str()),
                "captured stdin hash drift for {}",
                case.id
            );
        } else {
            assert!(case.stdin_sha256.is_none());
        }
        if let Some(relative) = &case.argv_fixture {
            assert_eq!(
                case.argv_fixture_sha256.as_deref(),
                Some(fixture_hash(&input, relative).as_str()),
                "captured argv fixture hash drift for {}",
                case.id
            );
        } else {
            assert!(case.argv_fixture_sha256.is_none());
        }

        let sandbox = Sandbox::new(&input);
        let before = snapshot(&sandbox.root);
        let mut child = Command::new(env!("CARGO_BIN_EXE_ags"))
            .args(expand_args(&case.args, &sandbox.root))
            .current_dir(sandbox.suite())
            .env("HOME", sandbox.root.join("home"))
            .env("AGS_HOME", sandbox.root.join("runtime"))
            .env("AGS_RUNTIME_HOME", sandbox.root.join("runtime"))
            .env("AGS_THIRD_PARTY_MANIFEST_OFFLINE", "1")
            .env("XDG_CONFIG_HOME", sandbox.root.join("xdg"))
            .env("PATH", contract_path())
            .env("NO_COLOR", "1")
            .stdin(if case.stdin_fixture.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("spawn behavior case {}: {error}", case.id));
        if let Some(relative) = &case.stdin_fixture {
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(
                    input
                        .stdin_fixtures
                        .get(relative)
                        .expect("immutable stdin fixture")
                        .as_bytes(),
                )
                .unwrap_or_else(|error| panic!("write stdin for {}: {error}", case.id));
        }
        let output = child
            .wait_with_output()
            .unwrap_or_else(|error| panic!("wait for behavior case {}: {error}", case.id));
        let after = snapshot(&sandbox.root);
        assert_eq!(
            output.status.code(),
            Some(case.exit_code),
            "exit-code drift for {}",
            case.id
        );
        assert!(
            matches!(case.output_policy.as_str(), "exact" | "json-contract"),
            "unknown output policy for {}",
            case.id
        );
        let actual_stdout = normalize(output.stdout, &sandbox.root);
        if case.output_policy == "exact" {
            assert_eq!(actual_stdout, case.stdout, "stdout drift for {}", case.id);
            assert!(case.stdout_json_contract.is_none());
        } else {
            assert_json_contract(
                &actual_stdout,
                case.stdout_json_contract
                    .as_ref()
                    .expect("captured JSON contract"),
                &case.id,
            );
        }
        assert_eq!(
            normalize(output.stderr, &sandbox.root),
            case.stderr,
            "stderr drift for {}",
            case.id
        );
        let actual_delta = normalize_allowed_content_changes(
            portable_delta(filesystem_delta(&before, &after)),
            &contract.filesystem_content_change_allowlist,
        );
        let expected_delta = normalize_allowed_content_changes(
            portable_delta(case.filesystem_delta.clone()),
            &contract.filesystem_content_change_allowlist,
        );
        assert_eq!(
            actual_delta, expected_delta,
            "filesystem content/mode/target delta drift for {}",
            case.id
        );
    }
}

fn normalize_allowed_content_changes(mut delta: Value, allowlist: &BTreeSet<String>) -> Value {
    for operation in ["created", "modified", "deleted"] {
        for entry in delta[operation]
            .as_array_mut()
            .unwrap_or_else(|| panic!("{operation} delta must be an array"))
        {
            let path = entry["path"].as_str().expect("delta path");
            if !allowlist.contains(path) {
                continue;
            }
            match operation {
                "modified" => {
                    if let Some(object) = entry["after"].as_object_mut() {
                        if object.contains_key("sha256") {
                            object.insert(
                                "sha256".to_string(),
                                Value::String("<ALLOWED-CONTENT-CHANGE>".to_string()),
                            );
                        }
                    }
                }
                _ => {
                    if let Some(object) = entry.as_object_mut() {
                        if object.contains_key("sha256") {
                            object.insert(
                                "sha256".to_string(),
                                Value::String("<ALLOWED-CONTENT-CHANGE>".to_string()),
                            );
                        }
                    }
                }
            }
        }
    }
    delta
}

fn portable_delta(delta: Value) -> Value {
    #[cfg(not(unix))]
    {
        let mut delta = delta;
        fn normalize_modes(value: &mut Value) {
            match value {
                Value::Object(object) => {
                    if object.contains_key("mode") {
                        object.insert("mode".to_string(), Value::String("portable".to_string()));
                    }
                    for child in object.values_mut() {
                        normalize_modes(child);
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        normalize_modes(child);
                    }
                }
                _ => {}
            }
        }
        normalize_modes(&mut delta);
        delta
    }
    #[cfg(unix)]
    {
        delta
    }
}

#[test]
fn task_prepare_execution_remains_the_canonical_machine_capability() {
    let contract = load_contract();
    let mapped = contract
        .cases
        .iter()
        .filter_map(|case| {
            (case.machine_capability.as_deref() == Some("TaskPrepareExecution"))
                .then_some((case.args.as_slice(), "TaskPrepareExecution"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        mapped,
        vec![(
            &[
                "run".to_string(),
                "-".to_string(),
                "--check-only".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ][..],
            "TaskPrepareExecution",
        )]
    );

    let canonical = ags_governance_decision::CliCapabilityId::TaskPrepareExecution;
    assert_eq!(
        serde_json::to_string(&canonical).unwrap(),
        "\"task_prepare_execution\""
    );
    let legacy: ags_governance_decision::CliCapabilityId =
        serde_json::from_str("\"task_execute\"").unwrap();
    assert_eq!(legacy, canonical);
    assert_eq!(
        serde_json::to_string(&legacy).unwrap(),
        "\"task_prepare_execution\"",
        "legacy compatibility must never restore task_execute as public output"
    );
}

#[test]
fn json_contract_rejects_added_missing_and_shape_changes() {
    let contract = json!({
        "baseline": r#"{"status":"ok","items":[{"id":"one"}]}"#,
        "required_types": [
            {"path":"$.status","type":"string"},
            {"path":"$.items","type":"array"},
            {"path":"$.items[].id","type":"string"}
        ],
        "assertions": [{"path":"$.status","value":"ok"}],
        "allowed_product_version_paths": []
    });
    for drift in [
        r#"{"status":"ok","items":[{"id":"one"}],"new":true}"#,
        r#"{"status":"ok","items":[{}]}"#,
        r#"{"status":"ok","items":{"id":"one"}}"#,
    ] {
        assert!(
            std::panic::catch_unwind(|| assert_json_contract(drift, &contract, "shape-probe"))
                .is_err(),
            "strict JSON contract accepted drift: {drift}"
        );
    }
}

#[test]
fn json_contract_allows_only_an_explicit_product_version_path() {
    let contract = json!({
        "baseline": r#"{"product_version":"0.3.0","schema_version":"0.3.0-wire"}"#,
        "required_types": [
            {"path":"$.product_version","type":"string"},
            {"path":"$.schema_version","type":"string"}
        ],
        "assertions": [{"path":"$.schema_version","value":"0.3.0-wire"}],
        "allowed_product_version_paths": ["$.product_version"]
    });
    let candidate = format!(
        r#"{{"product_version":"{}","schema_version":"0.3.0-wire"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_json_contract(&candidate, &contract, "product-version-probe");
    let changed_schema = format!(
        r#"{{"product_version":"{}","schema_version":"0.3.2-wire"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert!(std::panic::catch_unwind(|| {
        assert_json_contract(&changed_schema, &contract, "schema-version-probe")
    })
    .is_err());
}

#[test]
fn filesystem_contract_tracks_file_content_and_mode() {
    let input = InputFixture {
        schema_version: "ags-cli-behavior-input/1".to_string(),
        files: BTreeMap::from([("suite/value.txt".to_string(), "before".to_string())]),
        stdin_fixtures: BTreeMap::new(),
    };
    let sandbox = Sandbox::new(&input);
    let before = snapshot(&sandbox.root);
    std::fs::write(sandbox.suite().join("value.txt"), "after").unwrap();
    let after = snapshot(&sandbox.root);
    let delta = filesystem_delta(&before, &after);
    assert_ne!(
        delta["modified"][0]["before"]["sha256"],
        delta["modified"][0]["after"]["sha256"]
    );
    assert_eq!(
        delta["modified"][0]["before"]["mode"],
        delta["modified"][0]["after"]["mode"]
    );
}

#[cfg(unix)]
#[test]
fn filesystem_contract_tracks_normalized_symlink_targets() {
    use std::os::unix::fs::symlink;

    let input = InputFixture {
        schema_version: "ags-cli-behavior-input/1".to_string(),
        files: BTreeMap::from([("suite/a.txt".to_string(), "a".to_string())]),
        stdin_fixtures: BTreeMap::new(),
    };
    let sandbox = Sandbox::new(&input);
    symlink(
        sandbox.suite().join("a.txt"),
        sandbox.suite().join("current"),
    )
    .unwrap();
    let state = file_state(&sandbox.suite().join("current"));
    assert_eq!(state["kind"], "symlink");
    assert_eq!(state["target"], "<CONTRACT_ROOT>/suite/a.txt");
    assert!(state["mode"].as_str().is_some());
}
