use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const EXPECTED_PACKAGES: &[&str] = &[
    "ags-capability-governance",
    "ags-cli",
    "ags-evidence",
    "ags-governance-decision",
    "ags-host-integration",
    "ags-lifecycle",
    "ags-mcp",
    "ags-platform",
    "ags-session",
    "ags-task-contract",
    "ags-verification",
    "ags-workspace-facts",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn cargo_metadata(root: &Path) -> Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata JSON")
}

#[test]
fn runtime_workspace_has_exactly_the_twelve_authoritative_packages() {
    let metadata = cargo_metadata(&workspace_root());
    let actual = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|package| package["name"].as_str().expect("package name").to_string())
        .collect::<BTreeSet<_>>();
    let expected = EXPECTED_PACKAGES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn authoritative_dependency_direction_has_no_reverse_edges() {
    let metadata = cargo_metadata(&workspace_root());
    let dependencies = metadata["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .map(|package| {
            let name = package["name"].as_str().expect("package name").to_string();
            let names = package["dependencies"]
                .as_array()
                .expect("dependencies")
                .iter()
                .map(|dependency| {
                    dependency["name"]
                        .as_str()
                        .expect("dependency name")
                        .to_string()
                })
                .collect::<BTreeSet<_>>();
            (name, names)
        })
        .collect::<BTreeMap<_, _>>();

    let lifecycle = dependencies
        .get("ags-lifecycle")
        .expect("lifecycle package");
    for forbidden in ["ags-cli", "ags-mcp", "ags-session", "ags-verification"] {
        assert!(
            !lifecycle.contains(forbidden),
            "ags-lifecycle has reverse dependency on {forbidden}"
        );
    }

    let session = dependencies.get("ags-session").expect("session package");
    for forbidden in ["ags-cli", "ags-mcp", "ags-lifecycle", "ags-verification"] {
        assert!(
            !session.contains(forbidden),
            "ags-session has reverse dependency on {forbidden}"
        );
    }

    let capability = dependencies
        .get("ags-capability-governance")
        .expect("capability package");
    for forbidden in ["ags-cli", "ags-mcp", "ags-lifecycle", "ags-verification"] {
        assert!(
            !capability.contains(forbidden),
            "ags-capability-governance has reverse dependency on {forbidden}"
        );
    }
}
