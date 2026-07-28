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
        file: "crates/ags-mcp/src/server.rs",
        from: "        && !preflight.governance.is_preflight_completed()\n        && !allowed_in_bootstrap",
        to: "        && false\n        && !allowed_in_bootstrap",
        package: "ags-mcp",
        test: "tools_call_requires_preflight",
    },
    Mutation {
        id: "X2",
        file: "crates/ags-mcp/src/tools/apply.rs",
        from: "            held.consumed = true;",
        to: "            held.consumed = false;",
        package: "ags-mcp",
        test: "decision_lease_is_consumed_exactly_once",
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
        let path = workspace.root.join(mutation.file);
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
        fs::write(&path, mutated)
            .map_err(|error| format!("{} cannot write temporary mutation: {error}", mutation.id))?;

        let compile = run_cargo_test(
            &workspace.root,
            &target_dir,
            mutation.package,
            mutation.test,
            true,
        )?;
        if !compile.status.success() {
            return Err(format!(
                "{} COMPILE_FAILED (not killed by semantic test): {}",
                mutation.id,
                output_summary(&compile)
            ));
        }

        let semantic = run_cargo_test(
            &workspace.root,
            &target_dir,
            mutation.package,
            mutation.test,
            false,
        )?;
        fs::write(&path, original)
            .map_err(|error| format!("{} cannot restore temporary source: {error}", mutation.id))?;
        if semantic.status.success() {
            return Err(format!(
                "{} SURVIVED: {}::{} accepted the mutated implementation",
                mutation.id, mutation.package, mutation.test
            ));
        }
        killed.push(mutation.id);
    }

    Ok(format!(
        "{} fixed semantic mutations KILLED: {}",
        killed.len(),
        killed.join(", ")
    ))
}

fn run_cargo_test(
    workspace: &Path,
    target_dir: &Path,
    package: &str,
    test: &str,
    no_run: bool,
) -> Result<Output, String> {
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(["test", "-q", "-p", package, test]);
    if no_run {
        command.arg("--no-run");
    }
    command
        .output()
        .map_err(|error| format!("cannot execute cargo for mutation {package}::{test}: {error}"))
}

fn output_summary(output: &Output) -> String {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined.chars().take(1200).collect()
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
    fn nine_security_mutations_are_killed_by_semantic_tests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let evidence = verify(root).unwrap();
        assert!(evidence.starts_with("9 fixed semantic mutations KILLED:"));
    }
}
