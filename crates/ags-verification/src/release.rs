use super::*;
use std::path::Path;

pub(super) fn check_release_boundary(repo_root: &Path) -> Vec<CheckItem> {
    let mut items = Vec::new();

    let manifest = crate::release_manifest::verify_release_manifest(repo_root);
    if manifest.passed {
        items.push(CheckItem::pass(
            "release-public-manifest",
            "release",
            "Tracked public payload exactly matches the canonical authority.",
        ));
    } else {
        items.push(CheckItem::fail(
            "release-public-manifest",
            "release",
            &format!(
                "Public release payload failed: missing=[{}], forbidden=[{}], extra=[{}], content=[{}], authority=[{}]",
                manifest.required_missing.join(", "),
                manifest.forbidden_found.join(", "),
                manifest.extra_files.join(", "),
                manifest.content_mismatches.join(", "),
                manifest.authority_errors.join(", "),
            ),
            "Restore manifests/public-release-payload.yaml, add every required file, and remove every non-authority or forbidden tracked file.",
        ));
    }
    items.push(check_release_version_surfaces(repo_root));
    items.push(check_first_party_language_boundary(repo_root));
    items.push(check_validator_mutation_guards(repo_root));

    items
}

fn check_first_party_language_boundary(repo_root: &Path) -> CheckItem {
    let mut violations = Vec::new();
    let scripts = repo_root.join("scripts");
    match std::fs::read_dir(&scripts) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let relative = format!("scripts/{}", entry.file_name().to_string_lossy());
                match entry.file_type() {
                    Ok(file_type)
                        if file_type.is_file()
                            && matches!(
                                relative.as_str(),
                                "scripts/ags-memory-lifecycle-omp.js"
                                    | "scripts/sign-release-index.mjs"
                            ) => {}
                    Ok(_) => violations.push(format!(
                        "{relative}: scripts/ may contain only reviewed, bounded runtime adapters"
                    )),
                    Err(error) => {
                        violations.push(format!("{relative}: cannot inspect file type: {error}"))
                    }
                }
            }
        }
        Err(error) => violations.push(format!("scripts/: cannot inspect directory: {error}")),
    }

    for file in rust_source_files(&repo_root.join("crates")) {
        match std::fs::read_to_string(&file) {
            Ok(body) if directly_launches_python(&body) => {
                violations.push(format!(
                    "{}: Rust core must not delegate first-party logic to Python",
                    file.strip_prefix(repo_root).unwrap_or(&file).display()
                ));
            }
            Ok(_) => {}
            Err(error) => violations.push(format!(
                "{}: cannot inspect source: {error}",
                file.strip_prefix(repo_root).unwrap_or(&file).display()
            )),
        }
    }

    let omp = scripts.join("ags-memory-lifecycle-omp.js");
    match std::fs::read_to_string(&omp) {
        Ok(body) => {
            let lower = body.to_ascii_lowercase();
            for forbidden in [
                "permission mode",
                "execution mode",
                "execution topology",
                "task-card-hash",
                "launch-plan-hash",
                "sha256",
                "receipt_id",
                "receipt-id",
            ] {
                if lower.contains(forbidden) {
                    violations.push(format!(
                        "scripts/ags-memory-lifecycle-omp.js: thin adapter contains forbidden governance token `{forbidden}`"
                    ));
                }
            }
            for required in ["spawnSync", "\"host\"", "\"lifecycle\""] {
                if !body.contains(required) {
                    violations.push(format!(
                        "scripts/ags-memory-lifecycle-omp.js: missing thin-adapter marker `{required}`"
                    ));
                }
            }
        }
        Err(error) => violations.push(format!(
            "scripts/ags-memory-lifecycle-omp.js: cannot read required adapter: {error}"
        )),
    }

    let signer = scripts.join("sign-release-index.mjs");
    match std::fs::read_to_string(&signer) {
        Ok(body) => {
            for required in [
                "node:crypto",
                "crypto.sign(null",
                "crypto.verify(null",
                "release-index.json",
                "release-index.sig",
                "AGS_RELEASE_SIGNING_PRIVATE_KEY",
            ] {
                if !body.contains(required) {
                    violations.push(format!(
                        "scripts/sign-release-index.mjs: missing release-signing adapter marker `{required}`"
                    ));
                }
            }
            for forbidden in [
                "node:child_process",
                "node:http",
                "node:https",
                "fetch(",
                "eval(",
            ] {
                if body.contains(forbidden) {
                    violations.push(format!(
                        "scripts/sign-release-index.mjs: bounded signer contains forbidden capability `{forbidden}`"
                    ));
                }
            }
        }
        Err(error) => violations.push(format!(
            "scripts/sign-release-index.mjs: cannot read required adapter: {error}"
        )),
    }

    if violations.is_empty() {
        CheckItem::pass(
            "first-party-language-boundary",
            "release",
            "Rust owns first-party governance, lifecycle, verification, release planning, and safety logic; scripts/ contains only reviewed, bounded host and cryptographic adapters.",
        )
    } else {
        CheckItem::fail(
            "first-party-language-boundary",
            "release",
            &truncate(&violations.join("\n"), 1600),
            "Move first-party logic into Rust and keep non-Rust host adapters transport-only.",
        )
    }
}

fn directly_launches_python(body: &str) -> bool {
    let compact = body.split_whitespace().collect::<String>();
    compact.contains(concat!("Command::new(\"", "python", "\")"))
        || compact.contains(concat!("Command::new(\"", "python", "3\")"))
}

fn rust_source_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            files.extend(rust_source_files(&path));
        } else if file_type.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        {
            files.push(path);
        }
    }
    files
}

fn check_validator_mutation_guards(repo_root: &Path) -> CheckItem {
    match crate::mutation_guard::verify(repo_root) {
        Ok(evidence) => CheckItem::pass("semantic-mutation-guards", "release", &evidence),
        Err(error) => CheckItem::fail(
            "semantic-mutation-guards",
            "release",
            &truncate(&error, 1200),
            "Repair the Rust semantic contract test or the production invariant that survived mutation.",
        )
    }
}

#[cfg(test)]
mod language_boundary_tests {
    use super::directly_launches_python;

    #[test]
    fn legacy_python_hook_text_is_not_a_process_launch() {
        assert!(!directly_launches_python(
            r#"const LEGACY: &str = "python3 context-memory-start.py";"#
        ));
    }

    #[test]
    fn direct_python_process_launch_is_rejected() {
        let source = [
            "std::process::Command::new(\"",
            "python3",
            "\").arg(\"legacy.py\");",
        ]
        .concat();
        assert!(directly_launches_python(&source));
    }
}
