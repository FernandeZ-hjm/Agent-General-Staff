use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::Deserialize;

#[derive(Deserialize)]
struct Contract {
    schema_version: String,
    baseline_product_version: String,
    baseline_release_commit: String,
    baseline_executable_sha256: String,
    help: BTreeMap<String, String>,
    machine_help: BTreeMap<String, String>,
}

#[test]
fn v033_preserves_the_v030_machine_cli_contract() {
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
            normalized_help(output.stdout),
            *expected,
            "Machine CLI drift at {path}"
        );
        assert!(output.stderr.is_empty());
    }
}

fn ags() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ags"))
}

fn normalized_help(stdout: Vec<u8>) -> String {
    String::from_utf8(stdout).unwrap().replace("ags.exe", "ags")
}

#[test]
fn platform_executable_suffix_is_not_command_surface_drift() {
    assert_eq!(
        normalized_help(b"Usage: ags.exe skill --help\n".to_vec()),
        "Usage: ags skill --help\n"
    );
}

#[test]
fn v033_preserves_the_complete_v030_human_command_surface() {
    let fixture = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/human-cli-v0.3.0.json"
    ))
    .unwrap();
    let contract: Contract = serde_json::from_str(&fixture).unwrap();
    assert_eq!(contract.schema_version, "ags-human-cli-contract/1");
    assert_eq!(contract.baseline_product_version, "0.3.0");
    assert_eq!(
        contract.baseline_release_commit,
        "7d7e0477829a9288e97f3f2536a5ba6a8763cd58"
    );
    assert_eq!(
        contract.baseline_executable_sha256,
        "sha256:af4aaf3f396bbb83c9f2bee3cac2c6352df412e4c6a2c9aade6a8417aeb2a7be"
    );
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.3.3");

    for (path, expected) in &contract.help {
        let mut command = ags();
        command.args(path.split_whitespace()).arg("--help");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "help failed for {path:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            normalized_help(output.stdout),
            *expected,
            "human CLI drift at {path:?}"
        );
        assert!(
            output.stderr.is_empty(),
            "help wrote stderr for {path:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        "ags 0.3.3"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn v033_rejects_commands_and_flags_absent_from_v030() {
    let cases = [
        (
            &["skill", "adopt-source", "x"][..],
            "error: unrecognized subcommand 'adopt-source'\n\n  tip: a similar subcommand exists: 'adopt'\n\nUsage: ags skill [OPTIONS] [COMMAND]\n\nFor more information, try '--help'.\n",
        ),
        (
            &["skill", "route-test", "x"][..],
            "error: unrecognized subcommand 'route-test'\n\nUsage: ags skill [OPTIONS] [COMMAND]\n\nFor more information, try '--help'.\n",
        ),
        (
            &["setup", "--with-evomap"][..],
            "error: unexpected argument '--with-evomap' found\n\nUsage: ags setup [OPTIONS]\n\nFor more information, try '--help'.\n",
        ),
        (
            &["plan", "--profile", "private", "--with-evomap"][..],
            "error: unexpected argument '--with-evomap' found\n\nUsage: ags plan --profile <PROFILE>\n\nFor more information, try '--help'.\n",
        ),
        (
            &["apply", "--profile", "private", "--with-evomap"][..],
            "error: unexpected argument '--with-evomap' found\n\nUsage: ags apply --profile <PROFILE>\n\nFor more information, try '--help'.\n",
        ),
        (
            &["verify", "--with-evomap"][..],
            "error: unexpected argument '--with-evomap' found\n\nUsage: ags verify [OPTIONS] [COMMAND]\n\nFor more information, try '--help'.\n",
        ),
        (
            &["update", "apply", "--with-evomap"][..],
            "error: unexpected argument '--with-evomap' found\n\nUsage: ags update apply [OPTIONS]\n\nFor more information, try '--help'.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = ags().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(
            normalized_help(output.stderr),
            expected_stderr,
            "parser contract drift at {args:?}"
        );
    }
}
