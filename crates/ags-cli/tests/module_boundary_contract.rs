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

const RETIRED_PACKAGES: &[&str] = &[
    "bootstrap-dry-run",
    "capability-registry",
    "delivery-report-validator",
    "execution-policy",
    "runner",
    "skill-governance",
    "suite-doctor",
    "task-card-validator",
    "workflow-sync-check",
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

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, files: &mut Vec<PathBuf>) {
        if !directory.exists() {
            return;
        }
        let mut entries = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
            .map(|entry| entry.expect("directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(root, &mut files);
    files
}

#[test]
fn every_workspace_rust_source_is_tracked() {
    let root = workspace_root();
    let output = Command::new("git")
        .args(["ls-files", "-z", "--", "crates"])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tracked = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect::<BTreeSet<_>>();
    let missing = EXPECTED_PACKAGES
        .iter()
        .flat_map(|package| rust_sources(&root.join("crates").join(package).join("src")))
        .filter_map(|path| {
            let relative = path
                .strip_prefix(&root)
                .expect("workspace source")
                .to_string_lossy()
                .replace('\\', "/");
            (!tracked.contains(&relative)).then_some(relative)
        })
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "workspace Rust sources must be tracked: {}",
        missing.join(", ")
    );
}

fn read_rust_tree(root: &Path) -> String {
    rust_sources(root)
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_contains_all(label: &str, source: &str, markers: &[&str]) {
    for marker in markers {
        assert!(
            source.contains(marker),
            "{label} is missing authority marker `{marker}`"
        );
    }
}

fn assert_contains_none(label: &str, source: &str, markers: &[&str]) {
    for marker in markers {
        assert!(
            !source.contains(marker),
            "{label} still contains retired authority marker `{marker}`"
        );
    }
}

#[test]
fn runtime_workspace_has_exactly_the_twelve_authoritative_packages() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);
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
    assert_eq!(
        actual, expected,
        "the runtime workspace must expose one package per authoritative module"
    );
}

#[test]
fn authoritative_manifests_do_not_depend_on_retired_packages() {
    let root = workspace_root();
    for package in EXPECTED_PACKAGES {
        let manifest = root.join("crates").join(package).join("Cargo.toml");
        let content = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("read {}: {error}", manifest.display()));
        for retired in RETIRED_PACKAGES {
            assert!(
                !content.contains(&format!("{retired} ="))
                    && !content.contains(&format!("package = \"{retired}\"")),
                "{} still depends on retired package {retired}",
                manifest.display()
            );
        }
    }
}

#[test]
fn retired_package_manifests_are_removed() {
    let root = workspace_root();
    for retired in RETIRED_PACKAGES {
        assert!(
            !root
                .join("crates")
                .join(retired)
                .join("Cargo.toml")
                .exists(),
            "retired package manifest still exists: {retired}"
        );
    }
}

#[test]
fn lifecycle_and_capability_crates_are_the_only_domain_authorities() {
    let root = workspace_root();
    let cli = root.join("crates/ags-cli/src");
    let lifecycle = root.join("crates/ags-lifecycle/src");
    let capability = root.join("crates/ags-capability-governance/src");

    let cli_setup = read_rust_tree(&cli.join("setup"));
    let lifecycle_setup = read_rust_tree(&lifecycle.join("setup"));
    assert_contains_all(
        "ags-lifecycle setup",
        &lifecycle_setup,
        &[
            "pub struct PrivateApplyRequest",
            "pub fn apply_private",
            "pub fn private_plan_presentation",
        ],
    );
    assert_contains_none(
        "ags-cli setup adapter",
        &cli_setup,
        &[
            "struct PrivateInstallPlan",
            "fn build_private_install_plan",
            "fn write_install_file",
            "fn merge_ags_entry_block",
        ],
    );

    let cli_init = read_rust_tree(&cli.join("init"));
    let lifecycle_init = read_rust_tree(&lifecycle.join("init"));
    assert_contains_all(
        "ags-lifecycle init",
        &lifecycle_init,
        &[
            "pub struct ProjectInitPlan",
            "fn desired_project_file_content",
            "pub fn compute_overlay_plan",
            "pub fn apply_overlay",
        ],
    );
    assert_contains_none(
        "ags-cli init adapter",
        &cli_init,
        &[
            "struct ProjectInitPlan",
            "fn desired_project_file_content",
            "fn compute_overlay_plan",
            "fn apply_overlay",
        ],
    );

    let cli_update = read_rust_tree(&cli.join("update"));
    let lifecycle_update = read_rust_tree(&lifecycle.join("update"));
    assert_contains_all(
        "ags-lifecycle update",
        &lifecycle_update,
        &[
            "pub struct ApplyRequest",
            "pub trait UpdateEffects",
            "pub fn build_all_update_lanes",
        ],
    );
    assert_contains_none(
        "ags-cli update adapter",
        &cli_update,
        &[
            "fn orchestrate_local_kernel_build",
            "fn build_update_lane_plan",
            "struct NotifierState",
            "fn fetch_latest_version",
        ],
    );

    let cli_skill = read_rust_tree(&cli.join("skill"));
    let adoption_module = capability.join("adoption");
    let capability_adoption = if adoption_module.is_dir() {
        read_rust_tree(&adoption_module)
    } else {
        std::fs::read_to_string(capability.join("adoption.rs"))
            .expect("capability adoption authority")
    };
    assert_contains_all(
        "ags-capability-governance adoption",
        &capability_adoption,
        &[
            "pub struct AdoptionPlan",
            "fn install_canonical_body",
            "fn acquire_github_source",
            "fn audit_skill_directory",
            "fn persist_plan",
            "fn validate_saved_plan_integrity",
        ],
    );
    assert_contains_none(
        "ags-cli skill adapter",
        &cli_skill,
        &[
            "struct AdoptionPlan",
            "fn install_canonical_body",
            "fn acquire_github_source",
            "fn audit_skill_directory",
            "fn persist_plan",
            "fn validate_saved_plan_integrity",
        ],
    );

    let workspace_facts = read_rust_tree(&root.join("crates/ags-workspace-facts/src"));
    let cli_managed_projects = std::fs::read_to_string(cli.join("managed_projects.rs"))
        .expect("CLI managed-project facade");
    assert_contains_all(
        "ags-workspace-facts managed-project authority",
        &workspace_facts,
        &[
            "pub struct ManagedProjectsRegistry",
            "pub fn upsert",
            "pub fn partition_existing",
            "pub fn render_yaml",
        ],
    );
    assert_contains_none(
        "ags-cli managed-project adapter",
        &cli_managed_projects,
        &[
            "struct ManagedProjectsRegistry",
            "fn parse_registry",
            "fn apply_field",
            "pub fn upsert",
            "pub fn partition_existing",
        ],
    );

    let verification_release =
        std::fs::read_to_string(root.join("crates/ags-verification/src/release_package.rs"))
            .expect("verification release-package authority");
    let cli_release =
        std::fs::read_to_string(cli.join("kernel/release.rs")).expect("CLI release adapter");
    assert_contains_all(
        "ags-verification release-package authority",
        &verification_release,
        &[
            "pub fn release_package_plan",
            "fn git_tracked_release_files",
            "fn release_file_list",
        ],
    );
    assert_contains_none(
        "ags-cli release adapter",
        &cli_release,
        &[
            "fn release_package_plan",
            "fn git_tracked_release_files",
            "fn release_file_list",
            "fn public_release_forbidden_patterns",
        ],
    );
    assert!(
        cli_release.contains("ags_verification::release_package::release_package_plan"),
        "CLI release adapter must delegate package planning to ags-verification"
    );
}

#[test]
fn retired_crates_and_previous_monolith_sources_are_absent() {
    let root = workspace_root();
    for retired in RETIRED_PACKAGES {
        let retired_root = root.join("crates").join(retired);
        assert!(
            rust_sources(&retired_root).is_empty(),
            "retired crate still contains Rust authority sources: {retired}"
        );
    }

    for retired_source in [
        "crates/skill-governance/src/console.rs",
        "crates/skill-governance/src/lib.rs",
        "crates/task-card-validator/src/lib.rs",
        "crates/execution-policy/src/lib.rs",
        "crates/runner/src/lib.rs",
        "crates/ags-cli/src/init/overlay.rs",
    ] {
        assert!(
            !root.join(retired_source).exists(),
            "retired monolith/source still exists: {retired_source}"
        );
    }
}

#[test]
fn mcp_and_cli_delegate_workspace_state_to_ags_session() {
    let root = workspace_root();
    let cli_source = read_rust_tree(&root.join("crates/ags-cli/src"));
    let mcp_source = read_rust_tree(&root.join("crates/ags-mcp/src"));
    let session_source = read_rust_tree(&root.join("crates/ags-session/src"));

    assert_contains_all(
        "ags-session authority",
        &session_source,
        &[
            "pub struct WorkspaceClientSession",
            "pub struct SessionActionStore",
            "struct WorkspaceCapabilityBundle",
            "fn connect_or_start",
        ],
    );
    assert_contains_none(
        "ags-cli adapter",
        &cli_source,
        &[
            "struct WorkspaceCapabilityBundle",
            "struct WorkspaceRegistry",
            "struct WorkspaceClientSession",
            "struct SessionActionStore",
            "fn connect_or_start",
            "fn run_workspace_daemon",
        ],
    );
    assert_contains_none(
        "ags-mcp adapter",
        &mcp_source,
        &[
            "struct WorkspaceCapabilityBundle",
            "struct WorkspaceRegistry",
            "struct WorkspaceClientSession",
            "struct SessionActionStore",
            "fn connect_or_start",
        ],
    );

    let mcp_server =
        std::fs::read_to_string(root.join("crates/ags-mcp/src/server.rs")).expect("MCP server");
    let mcp_wire =
        std::fs::read_to_string(root.join("crates/ags-mcp/src/tools/wire.rs")).expect("MCP wire");
    assert!(
        mcp_server.contains("governance: ags_session::WorkspaceClientSession<tools::HeldAction>"),
        "MCP preflight state must delegate governance state to ags-session"
    );
    assert!(
        mcp_wire.contains("type RoutingSession = ags_session::SessionActionStore<HeldAction>"),
        "MCP routing session must remain an ags-session type alias"
    );
    let mcp_lib =
        std::fs::read_to_string(root.join("crates/ags-mcp/src/lib.rs")).expect("MCP library");
    assert!(
        mcp_lib.contains("ags_session::run_workspace_daemon(")
            && mcp_lib.contains("Arc::new(McpSessionHandler"),
        "the MCP daemon entrypoint must remain a thin ags-session delegation"
    );
}

#[test]
fn authoritative_dependency_direction_has_no_reverse_or_lifecycle_verification_edge() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);
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
    assert!(
        !lifecycle.contains("ags-verification"),
        "ags-lifecycle must not depend on ags-verification; that recreates lifecycle -> verification -> capability -> lifecycle"
    );
    for forbidden in ["ags-cli", "ags-mcp", "ags-session"] {
        assert!(
            !lifecycle.contains(forbidden),
            "ags-lifecycle has reverse adapter/runtime dependency on {forbidden}"
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
