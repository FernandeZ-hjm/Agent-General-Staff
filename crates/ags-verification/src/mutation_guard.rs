use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Mutation {
    id: &'static str,
    file: &'static str,
    from: &'static str,
    to: &'static str,
    package: &'static str,
    test: &'static str,
}

const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: "A1",
        file: "crates/ags-task-contract/src/validator/constants.rs",
        from: "    \"fanout-cross-card\",\n];",
        to: "    \"limited\",\n];",
        package: "ags-task-contract",
        test: "legacy_authority_fields_and_values_are_rejected",
    },
    Mutation {
        id: "A2",
        file: "crates/ags-task-contract/src/validator/authority.rs",
        from: "    if execution_mode == \"plan-only\" && execution_topology != \"single\" {",
        to: "    if false && execution_mode == \"plan-only\" && execution_topology != \"single\" {",
        package: "ags-task-contract",
        test: "plan_only_parallel_topology_is_rejected",
    },
    Mutation {
        id: "A3",
        file: "crates/ags-task-contract/src/validator/checks.rs",
        from: "    if delegation_planning == \"yes\" && permission == \"plan-only\" {",
        to: "    if false && delegation_planning == \"yes\" && permission == \"plan-only\" {",
        package: "ags-task-contract",
        test: "plan_only_delegation_planning_is_rejected",
    },
    Mutation {
        id: "P1",
        file: "crates/ags-governance-decision/src/policy/rules.rs",
        from: "        if !forbids_writes {",
        to: "        let _ = forbids_writes;\n        if true {",
        package: "ags-governance-decision",
        test: "launch_arg_generator_blocks_plan_only_write_flags",
    },
    Mutation {
        id: "X1",
        file: "crates/ags-control-plane/src/control_plane.rs",
        from: "        if action.plan.binding_hash != binding.binding_hash || caller_hash != binding.binding_hash {",
        to: "        if false && (action.plan.binding_hash != binding.binding_hash || caller_hash != binding.binding_hash) {",
        package: "ags-control-plane",
        test: "action_ref_rejects_tamper_cross_connection_host_and_workspace",
    },
    Mutation {
        id: "X2",
        file: "crates/ags-control-plane/src/control_plane.rs",
        from: "        if action.consumed {",
        to: "        if false && action.consumed {",
        package: "ags-control-plane",
        test: "consumed_action_ref_is_rejected_before_effects",
    },
    Mutation {
        id: "R1",
        file: "crates/ags-evidence/src/delivery_report.rs",
        from: "        &report_task_hash,\n        &task_card_hash,",
        to: "        &report_task_hash,\n        &report_task_hash,",
        package: "ags-evidence",
        test: "rejects_report_task_card_hash_tampering",
    },
    Mutation {
        id: "R2",
        file: "crates/ags-evidence/src/delivery_report.rs",
        from: "        &plan_task_hash,\n        &task_card_hash,",
        to: "        &plan_task_hash,\n        &plan_task_hash,",
        package: "ags-evidence",
        test: "rejects_launch_plan_task_card_hash_tampering",
    },
    Mutation {
        id: "R3",
        file: "crates/ags-evidence/src/delivery_report.rs",
        from: "        &report_launch_plan_hash,\n        &launch_plan_hash,",
        to: "        &report_launch_plan_hash,\n        &report_launch_plan_hash,",
        package: "ags-evidence",
        test: "rejects_report_launch_plan_hash_tampering",
    },
];

pub(super) fn verify(repo_root: &Path) -> Result<String, String> {
    let workspace = TemporaryWorkspace::copy_from(repo_root)?;
    let target_dir = repo_root.join("target").join("mutation-guards");
    fs::create_dir_all(&target_dir)
        .map_err(|error| format!("cannot create mutation target directory: {error}"))?;
    let mut killed = Vec::new();

    for mutation in MUTATIONS {
        run_mutation(&workspace.root, &target_dir, mutation)?;
        killed.push(mutation.id);
    }

    Ok(format!(
        "{} fixed semantic mutations KILLED: {}",
        killed.len(),
        killed.join(", ")
    ))
}

fn run_mutation(workspace: &Path, target_dir: &Path, mutation: &Mutation) -> Result<(), String> {
    let path = workspace.join(mutation.file);
    let original = fs::read_to_string(&path)
        .map_err(|error| format!("{} cannot read {}: {error}", mutation.id, mutation.file))?;
    let count = original.matches(mutation.from).count();
    if count != 1 {
        return Err(format!(
            "{} anchor count for {} is {count}, expected exactly 1",
            mutation.id, mutation.file
        ));
    }
    let mutated = original.replacen(mutation.from, mutation.to, 1);
    let mut source = MutationSource::apply(&path, original, mutated, mutation.id)?;

    let result = (|| {
        let compile = compile_test(workspace, target_dir, mutation.package, mutation.test)?;
        if !compile.status.success() {
            return Err(format!(
                "{} COMPILE_FAILED (not killed by semantic test): {}",
                mutation.id,
                output_summary(&compile)
            ));
        }

        let (executable, resolved_test) =
            locate_test_executable(&compile, workspace, mutation.test)
                .map_err(|error| format!("{} TEST_EXECUTABLE_NOT_FOUND: {error}", mutation.id))?;
        let semantic =
            run_test_binary(&executable, workspace, &resolved_test).map_err(|error| {
                format!(
                    "{} TEST_EXECUTION_FAILED for {}::{}: {error}",
                    mutation.id, mutation.package, mutation.test
                )
            })?;
        if semantic.status.success() {
            return Err(format!(
                "{} SURVIVED: {}::{} accepted the mutated implementation",
                mutation.id, mutation.package, mutation.test
            ));
        }
        Ok(())
    })();

    let restore = source.restore();
    match (result, restore) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!(
            "{} source restoration failed: {error}",
            mutation.id
        )),
        (Err(run_error), Err(restore_error)) => Err(format!(
            "{run_error}; {} source restoration failed: {restore_error}",
            mutation.id
        )),
    }
}

fn compile_test(
    workspace: &Path,
    target_dir: &Path,
    package: &str,
    test: &str,
) -> Result<Output, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(cargo_compile_args(package, test));
    command
        .output()
        .map_err(|error| format!("cannot execute cargo for mutation {package}::{test}: {error}"))
}

/// Arguments for the one cargo invocation allowed per mutation.
///
/// JSON messages are parsed to locate the executable; no target path is
/// reconstructed from package or test names.
fn cargo_compile_args(package: &str, test: &str) -> Vec<String> {
    vec![
        "test".to_string(),
        "--message-format=json".to_string(),
        "--no-run".to_string(),
        "-p".to_string(),
        package.to_string(),
        test.to_string(),
    ]
}

fn test_binary_args(test: &str) -> Vec<String> {
    vec![test.to_string(), "--exact".to_string()]
}

fn locate_test_executable(
    compile: &Output,
    workspace: &Path,
    test: &str,
) -> Result<(PathBuf, String), String> {
    let candidates = test_executable_candidates(&compile.stdout)?;
    let mut list_errors = Vec::new();
    for executable in candidates {
        let listed = Command::new(&executable)
            .current_dir(workspace)
            .arg("--list")
            .output()
            .map_err(|error| format!("cannot inspect {}: {error}", executable.display()))?;
        if !listed.status.success() {
            list_errors.push(format!(
                "{} exited {}: {}",
                executable.display(),
                listed.status,
                output_summary(&listed)
            ));
            continue;
        }
        if let Some(resolved_test) = test_list_match(&listed.stdout, test) {
            return Ok((executable, resolved_test));
        }
    }

    let detail = if list_errors.is_empty() {
        "no compiled test binary listed the requested exact test".to_string()
    } else {
        format!(
            "no compiled test binary listed the requested exact test; {}",
            list_errors.join("; ")
        )
    };
    Err(detail)
}

fn test_executable_candidates(stdout: &[u8]) -> Result<Vec<PathBuf>, String> {
    let text = String::from_utf8(stdout.to_vec())
        .map_err(|error| format!("cargo JSON output is not UTF-8: {error}"))?;
    let mut candidates = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        if value
            .get("profile")
            .and_then(|profile| profile.get("test"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            continue;
        }
        let Some(executable) = value.get("executable").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let path = PathBuf::from(executable);
        if !path.is_absolute() {
            return Err(format!(
                "cargo reported a non-absolute test executable path `{executable}`"
            ));
        }
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    if candidates.is_empty() {
        return Err("cargo JSON output contained no test executable".to_string());
    }
    Ok(candidates)
}

fn test_list_match(stdout: &[u8], test: &str) -> Option<String> {
    let qualified_suffix = format!("::{test}");
    String::from_utf8_lossy(stdout)
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .find(|listed| *listed == test || listed.ends_with(&qualified_suffix))
        .map(ToString::to_string)
}

fn run_test_binary(executable: &Path, workspace: &Path, test: &str) -> Result<Output, String> {
    Command::new(executable)
        .current_dir(workspace)
        .args(test_binary_args(test))
        .output()
        .map_err(|error| format!("cannot execute {}: {error}", executable.display()))
}

fn output_summary(output: &Output) -> String {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined.chars().take(1200).collect()
}

struct MutationSource {
    path: PathBuf,
    original: String,
    restored: bool,
}

impl MutationSource {
    fn apply(path: &Path, original: String, mutated: String, id: &str) -> Result<Self, String> {
        fs::write(path, mutated)
            .map_err(|error| format!("{id} cannot write temporary mutation: {error}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            original,
            restored: false,
        })
    }

    fn restore(&mut self) -> Result<(), String> {
        if self.restored {
            return Ok(());
        }
        fs::write(&self.path, &self.original)
            .map_err(|error| format!("cannot restore {}: {error}", self.path.display()))?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for MutationSource {
    fn drop(&mut self) {
        if !self.restored {
            let _ = fs::write(&self.path, &self.original);
        }
    }
}

struct TemporaryWorkspace {
    root: PathBuf,
}

impl TemporaryWorkspace {
    fn copy_from(source: &Path) -> Result<Self, String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("ags-rust-mutation-{}-{nonce}", std::process::id()));
        fs::create_dir(&root)
            .map_err(|error| format!("cannot create temporary mutation workspace: {error}"))?;
        copy_tree(source, &root)?;
        Ok(Self { root })
    }
}

impl Drop for TemporaryWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("cannot read source directory {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot read source entry: {error}"))?;
        let name = entry.file_name();
        if matches!(
            name.to_str(),
            Some(".git" | ".codegraph" | ".ags-local" | "target" | "node_modules" | "__pycache__")
        ) {
            continue;
        }
        let from = entry.path();
        let to = target.join(&name);
        let metadata = fs::symlink_metadata(&from)
            .map_err(|error| format!("cannot inspect {}: {error}", from.display()))?;
        if metadata.is_dir() {
            fs::create_dir(&to)
                .map_err(|error| format!("cannot create {}: {error}", to.display()))?;
            copy_tree(&from, &to)?;
        } else if metadata.file_type().is_symlink() {
            copy_symlink(&from, &to)?;
        } else if metadata.is_file() {
            fs::copy(&from, &to)
                .map_err(|error| format!("cannot copy {}: {error}", from.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    let link = fs::read_link(source)
        .map_err(|error| format!("cannot read symlink {}: {error}", source.display()))?;
    std::os::unix::fs::symlink(link, target)
        .map_err(|error| format!("cannot copy symlink {}: {error}", source.display()))
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, target: &Path) -> Result<(), String> {
    let resolved = fs::canonicalize(source)
        .map_err(|error| format!("cannot resolve symlink {}: {error}", source.display()))?;
    if resolved.is_file() {
        fs::copy(&resolved, target)
            .map(|_| ())
            .map_err(|error| format!("cannot copy symlink target {}: {error}", source.display()))
    } else {
        fs::create_dir(target)
            .map_err(|error| format!("cannot create symlink target directory: {error}"))?;
        copy_tree(&resolved, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_compile_command_is_one_json_no_run_invocation() {
        let args = cargo_compile_args("ags-verification", "a_test");
        assert_eq!(args[0], "test");
        assert_eq!(args.iter().filter(|arg| *arg == "--no-run").count(), 1);
        assert_eq!(
            args.iter()
                .filter(|arg| *arg == "--message-format=json")
                .count(),
            1
        );
        assert_eq!(args.iter().filter(|arg| *arg == "a_test").count(), 1);
        assert!(!args.contains(&"--exact".to_string()));
    }

    #[test]
    fn test_binary_command_uses_exact_filter() {
        assert_eq!(
            test_binary_args("a_test"),
            vec!["a_test".to_string(), "--exact".to_string()]
        );
    }

    #[test]
    fn cargo_json_parser_uses_reported_test_executable() {
        let stdout = br#"{"reason":"compiler-artifact","profile":{"test":true},"executable":"/tmp/target/deps/ags-test"}
{"reason":"compiler-artifact","profile":{"test":false},"executable":"/tmp/target/debug/build-script"}
"#;
        assert_eq!(
            test_executable_candidates(stdout).unwrap(),
            vec![PathBuf::from("/tmp/target/deps/ags-test")]
        );
    }

    #[test]
    fn exact_test_listing_does_not_accept_a_prefix() {
        let stdout = b"a_test_extra: test\na_test: test\n";
        assert_eq!(test_list_match(stdout, "a_test").as_deref(), Some("a_test"));
        assert_eq!(
            test_list_match(b"validator::tests::a_test: test\n", "a_test").as_deref(),
            Some("validator::tests::a_test")
        );
        assert_eq!(test_list_match(b"a_test_extra: test\n", "a_test"), None);
    }

    #[test]
    #[ignore = "the release gate owns the full nine-mutation execution"]
    fn nine_security_mutations_are_killed_by_semantic_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let evidence = verify(root).unwrap();
        assert!(evidence.starts_with("9 fixed semantic mutations KILLED:"));
    }
}
