use std::process::{Command, Output};

const PUBLIC_COMMANDS: &[&str] = &[
    "setup", "init", "agent", "govern", "update", "doctor", "check", "test", "apply", "schema",
];

const RETIRED_COMMANDS: &[&str] = &[
    "onboarding",
    "task",
    "policy",
    "gate",
    "project",
    "protocol",
    "agents",
    "capability",
    "receipt",
    "memory",
    "host",
    "compliance",
    "session",
    "skill",
    "release",
    "mcp",
    "hooks",
    "run",
    "verify",
];

fn ags(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ags"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn root_help_lists_exact_contract_v2_commands() {
    let output = ags(&["--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let commands = stdout
        .lines()
        .skip_while(|line| line.trim() != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    assert_eq!(commands, PUBLIC_COMMANDS);
}

#[test]
fn retired_commands_are_standard_clap_unknowns() {
    for command in RETIRED_COMMANDS {
        let output = ags(&[command]);
        assert_eq!(output.status.code(), Some(2), "{command}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
        assert!(!stderr.contains("removed") && !stderr.contains("deprecated"));
    }
}

#[test]
fn retired_options_are_standard_clap_unknowns() {
    for (command, option) in [
        ("setup", "--target"),
        ("setup", "--yes"),
        ("init", "--mode"),
        ("doctor", "--fix"),
        ("update", "--apply"),
    ] {
        let output = ags(&[command, option]);
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("unexpected argument"), "{stderr}");
        assert!(!stderr.contains("removed"));
    }
}

#[test]
fn retired_nested_skill_adopt_is_unknown_and_install_is_canonical() {
    let retired = ags(&["govern", "skill", "adopt"]);
    assert_eq!(retired.status.code(), Some(2));
    let stderr = String::from_utf8(retired.stderr).unwrap();
    assert!(stderr.contains("unrecognized subcommand"), "{stderr}");
    assert!(!stderr.contains("removed") && !stderr.contains("deprecated"));

    let canonical = ags(&["govern", "skill", "install", "--help"]);
    assert!(
        canonical.status.success(),
        "{}",
        String::from_utf8_lossy(&canonical.stderr)
    );
}

#[test]
fn source_has_no_compatibility_or_duplicate_operation_registry() {
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![source_root];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for forbidden in [
                "RemovedCommand",
                "removed_command",
                "visible_alias",
                "alias =",
                "enum OperationInput",
                "struct AdapterRequest",
                "Commands::Mcp",
                "Commands::Host",
                "GovernSkillAdopt",
                "SkillAdoptRequest",
                "skill_adopt",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{} retained {forbidden}",
                    path.display()
                );
            }
        }
    }

    let cli = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/mod.rs"),
    )
    .unwrap();
    assert!(
        cli.contains("trait CliOperationAdapter"),
        "typed CLI adapters must be local to the CLI surface"
    );
    assert!(
        cli.contains("ags_control_plane::for_each_operation!(define_cli_registry)"),
        "CLI route metadata and dispatch must be generated from the canonical registry"
    );
}
