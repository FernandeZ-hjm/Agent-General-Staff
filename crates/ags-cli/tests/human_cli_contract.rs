use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::Deserialize;

#[derive(Deserialize)]
struct Contract {
    schema_version: String,
    baseline_product_version: String,
    allowed_version_change: String,
    help: BTreeMap<String, String>,
    machine_help: BTreeMap<String, String>,
}

fn ags() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ags"))
}

#[test]
fn v031_preserves_the_v030_machine_cli_contract() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/human-cli-v0.3.0.json"
    ))
    .unwrap();
    let contract: Contract = serde_json::from_str(&fixture).unwrap();
    assert_eq!(
        contract
            .machine_help
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "gate skill-tags".to_string(),
            "policy resolve".to_string(),
            "project detect".to_string(),
            "receipt verify".to_string(),
            "run".to_string(),
            "skill adopt".to_string(),
            "task compile".to_string(),
            "task validate".to_string(),
        ])
    );
    for (path, expected) in &contract.machine_help {
        let output = ags()
            .args(path.split_whitespace())
            .arg("--help")
            .output()
            .unwrap();
        assert!(output.status.success(), "Machine CLI help failed at {path}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            *expected,
            "Machine CLI drift at {path}"
        );
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn v031_preserves_the_complete_v030_human_command_surface() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/human-cli-v0.3.0.json"
    ))
    .unwrap();
    let contract: Contract = serde_json::from_str(&fixture).unwrap();
    assert_eq!(contract.schema_version, "ags-human-cli-contract/1");
    assert_eq!(contract.baseline_product_version, "0.3.0");
    assert_eq!(contract.allowed_version_change, env!("CARGO_PKG_VERSION"));

    for (path, expected) in &contract.help {
        let output = ags()
            .args(path.split_whitespace())
            .arg("--help")
            .output()
            .unwrap();
        assert!(output.status.success(), "help failed for {path:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            *expected,
            "human CLI drift at {path:?}"
        );
        assert!(output.stderr.is_empty());
    }

    let roots: BTreeSet<_> = contract
        .help
        .keys()
        .filter_map(|path| path.split_whitespace().next())
        .filter(|root| !root.is_empty())
        .collect();
    assert_eq!(
        roots,
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
}

#[test]
fn only_the_product_version_output_changes() {
    let output = ags().arg("--version").output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "ags 0.3.1"
    );
    assert!(output.stderr.is_empty());
}
