#[allow(unused_imports)]
use super::{
    actions::*, apply_transaction::*, dedupe::*, host_probe::*, host_verify::*, inventory::*,
    model::*, rendering::*, sync::*,
};
use std::path::{Path, PathBuf};

#[test]
fn analyze_duplicates_detects_name_collision_dry_run() {
    let root = std::env::temp_dir().join(format!("ags-dedupe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for store in ["global-skills", "skill-packs/optional"] {
        let d = root.join(store).join("dup");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "---\nname: dup\ndescription: x\n---\n").unwrap();
    }
    let r = analyze_duplicates(&root, false);
    assert_eq!(r.apply_status, "dry-run");
    assert!(r.applied_writes.is_empty(), "dry-run writes nothing");
    let group = r
        .groups
        .iter()
        .find(|g| g.name == "dup" && g.reason == "name-collision")
        .expect("name-collision group");
    assert_eq!(group.entries.len(), 2);
    assert!(group.keeper.as_deref().unwrap().contains("global-skills"));
    assert_eq!(group.quarantine.len(), 1);
    // dry-run leaves both copies on disk.
    assert!(root.join("global-skills/dup/SKILL.md").is_file());
    assert!(root.join("skill-packs/optional/dup/SKILL.md").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

fn seed_dup_repo(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("ags-dedupe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for store in ["global-skills", "skill-packs/optional"] {
        let d = root.join(store).join("dup");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "---\nname: dup\ndescription: x\n---\n").unwrap();
    }
    root
}

#[test]
fn analyze_duplicates_apply_populates_reversible_moves() {
    let root = seed_dup_repo("apply");
    let r = analyze_duplicates(&root, true);
    assert_eq!(r.apply_status, "applied");
    assert_eq!(r.applied_moves.len(), 1, "one non-keeper quarantined");
    // keeper (global) stays; non-keeper moved out of the optional store.
    assert!(root.join("global-skills/dup/SKILL.md").is_file());
    assert!(!root.join("skill-packs/optional/dup/SKILL.md").is_file());
    // the quarantine target lives under governance/backups and is restorable.
    let mv = &r.applied_moves[0];
    assert!(mv.to.contains("governance/backups"));
    assert!(Path::new(&mv.to).join("SKILL.md").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn analyze_duplicates_apply_failure_leaves_no_partial_quarantine() {
    let root = seed_dup_repo("fail");
    // Make governance/backups a FILE so the quarantine mkdir fails mid-apply.
    std::fs::create_dir_all(root.join("governance")).unwrap();
    std::fs::write(root.join("governance/backups"), "").unwrap();
    let r = analyze_duplicates(&root, true);
    assert_eq!(r.apply_status, "failed");
    assert!(
        r.applied_moves.is_empty(),
        "no partial quarantine on failure"
    );
    assert!(!r.apply_errors.is_empty());
    // both source copies remain in place (nothing half-moved).
    assert!(root.join("global-skills/dup/SKILL.md").is_file());
    assert!(root.join("skill-packs/optional/dup/SKILL.md").is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// Mock runner: returns canned `claude mcp list` / `codex mcp list`.
/// CodeBuddy-Code has no supported CLI MCP probe, so it is not invoked here.
/// PANICS on anything else — so any attempt to run an external installer or
/// registrar during a test fails loudly. Proves apply never shells out.
struct StrictMcpRunner {
    claude: CommandOutcome,
    codex: CommandOutcome,
}
impl CommandRunner for StrictMcpRunner {
    fn run(&self, program: &str, args: &[&str]) -> CommandOutcome {
        match (program, args) {
            ("claude", ["mcp", "list"]) => self.claude.clone(),
            ("codex", ["mcp", "list"]) => self.codex.clone(),
            _ => panic!(
                "console must only ever run a read-only `<host> mcp list`, got: {program} {args:?}"
            ),
        }
    }
}

// Claude `mcp list` format: `name: cmd ... - ✔ Connected`. ags + context7.
fn canned_list() -> CommandOutcome {
    CommandOutcome::Ran {
        success: true,
        stdout: "Checking MCP server health…\n\n\
                 ags: /home/.cargo/bin/ags mcp serve --transport stdio - ✔ Connected\n\
                 context7: npx -y @upstash/context7-mcp - ✔ Connected\n\
                 plugin:claude-mem:mcp-search: node -e launcher - ✔ Connected\n"
            .to_string(),
    }
}

// Codex `mcp list` format: a padded table. ags + context7 enabled;
// codegraph deliberately ABSENT (it is codex-expected → drives incomplete).
fn canned_codex_list() -> CommandOutcome {
    CommandOutcome::Ran {
        success: true,
        stdout: "Name       Command                Args   Env   Cwd   Status   Auth\n\
                 ags        /home/.cargo/bin/ags   mcp    -     -     enabled  Unsupported\n\
                 context7   npx                    args   -     -     enabled  Unsupported\n"
            .to_string(),
    }
}

fn ctx_with(tag: &str, list: CommandOutcome) -> (ConsoleContext, PathBuf) {
    ctx_with_repo_dir(tag, list, "repo")
}

fn ctx_with_repo_dir(
    tag: &str,
    list: CommandOutcome,
    repo_dir_name: &str,
) -> (ConsoleContext, PathBuf) {
    let base = std::env::temp_dir().join(format!("ags-console-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join(repo_dir_name);
    let home = base.join("home");

    write_file(
            &repo.join("manifests/suite.yaml"),
            "schema_version: \"1.0\"\n\
             suite:\n  name: \"test-suite\"\n  version: \"9.9.9\"\n  required:\n\
             \x20   - name: \"demo-skill\"\n      version: \"1.0\"\n      source: \"global-skills/demo-skill\"\n      hash: \"h1\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"demo-skill-ref\"\n",
        );
    write_file(
        &repo.join("global-skills/demo-skill/SKILL.md"),
        "---\nname: demo-skill\ndescription: demo.\n---\nbody\n",
    );
    write_file(
            &repo.join("manifests/skills-registry.yaml"),
            "schema_version: \"1.0\"\nskills:\n  - name: lark-shared\n    profile: optional\n    source: { type: external_cli_skill, manager: lark-cli }\n",
        );
    write_file(
        &home.join(".agents/skills/lark-shared/SKILL.md"),
        "---\nname: lark-shared\ndescription: official external body.\n---\n",
    );
    // An on-disk skill NOT in the manifest → should surface as Discovered.
    write_file(
        &repo.join("global-skills/orphan-skill/SKILL.md"),
        "---\nname: orphan-skill\ndescription: not in the manifest.\n---\nbody\n",
    );
    // installed_clients drives expected host visibility:
    //   ags, context7 → claude-code + codex;  codegraph → codex only.
    write_file(
            &repo.join("manifests/mcp-registry.yaml"),
            "schema_version: \"1.0\"\n\
             suite_interfaces:\n  - name: \"ags\"\n    role: \"host_initialization_adapter\"\n    governed: false\n    install:\n      installed_clients:\n        - \"claude-code\"\n        - \"codex\"\n\
             mcps:\n\
             \x20 - name: \"context7\"\n    package:\n      manager: \"npm\"\n    install:\n      installed_clients:\n        - \"claude-code\"\n        - \"codex\"\n\
             \x20 - name: \"codegraph\"\n    package:\n      manager: \"external-cli\"\n    install:\n      installed_clients:\n        - \"codex\"\n\
             \x20 - name: \"plugin:claude-mem:mcp-search\"\n    package:\n      manager: \"claude-plugin\"\n    install:\n      installed_clients:\n        - \"claude-code\"\n",
        );

    let ctx = ConsoleContext::new(
        repo,
        home,
        Box::new(StrictMcpRunner {
            claude: list,
            codex: canned_codex_list(),
        }),
    );
    (ctx, base)
}

fn write_file(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn imported_skill_distribution_is_dry_run_then_transactional_apply() {
    let (ctx, base) = ctx_with("imported-distribution", canned_list());
    let body_root = ctx.home.join(".ags/runtime/skill-bodies");
    let canonical = body_root.join("apple-design/sha256-demo");
    write_file(
        &canonical.join("SKILL.md"),
        "---\nname: apple-design\ndescription: Apple design guidance.\n---\n",
    );
    let hosts = vec!["codex".to_string(), "claude-code".to_string()];

    let plan =
        distribute_external_skill(&ctx, "apple-design", &canonical, &body_root, &hosts, false);
    assert_eq!(plan.apply_status, "dry-run");
    assert_eq!(plan.planned_writes.len(), 2);
    assert!(!ctx.home.join(".codex/skills/apple-design").exists());

    let applied =
        distribute_external_skill(&ctx, "apple-design", &canonical, &body_root, &hosts, true);
    assert!(applied.applied, "{applied:?}");
    assert_eq!(
        std::fs::canonicalize(ctx.home.join(".codex/skills/apple-design")).unwrap(),
        std::fs::canonicalize(&canonical).unwrap()
    );
    assert_eq!(
        std::fs::canonicalize(ctx.home.join(".claude/skills/apple-design")).unwrap(),
        std::fs::canonicalize(&canonical).unwrap()
    );
    let removed =
        remove_external_skill_distribution(&ctx, "apple-design", &canonical, &hosts, true);
    assert!(removed.applied, "{removed:?}");
    assert!(!ctx.home.join(".codex/skills/apple-design").exists());
    assert!(!ctx.home.join(".claude/skills/apple-design").exists());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn imported_skill_distribution_refuses_an_unrelated_same_name_host_entry() {
    let (ctx, base) = ctx_with("imported-distribution-collision", canned_list());
    let body_root = ctx.home.join(".ags/runtime/skill-bodies");
    let canonical = body_root.join("apple-design");
    write_file(
        &canonical.join("SKILL.md"),
        "---\nname: apple-design\ndescription: imported.\n---\n",
    );
    let existing = ctx.home.join(".codex/skills/apple-design");
    write_file(
        &existing.join("SKILL.md"),
        "---\nname: apple-design\ndescription: user-owned.\n---\n",
    );

    let result = distribute_external_skill(
        &ctx,
        "apple-design",
        &canonical,
        &body_root,
        &["codex".to_string()],
        true,
    );
    assert_eq!(result.apply_status, "blocked");
    assert!(result
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("canonical name collision")));
    assert!(std::fs::read_to_string(existing.join("SKILL.md"))
        .unwrap()
        .contains("user-owned"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn external_registry_skill_body_is_governed_from_shared_store() {
    let (ctx, base) = ctx_with("external-registry-body", canned_list());
    let shared = ctx.home.join(".agents/skills/lark-shared");

    let inv = build_inventory(&ctx, &["codex"]);
    let cap = find(&inv, "lark-shared");
    assert_eq!(cap.managed_status, ManagedStatus::Governed);
    assert_eq!(cap.registry_status, RegistryStatus::Registered);
    assert!(cap.canonical_present);
    assert_eq!(
        std::fs::canonicalize(cap.source.as_deref().unwrap()).unwrap(),
        std::fs::canonicalize(&shared).unwrap()
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn required_registry_parent_missing_body_is_expected() {
    let (ctx, base) = ctx_with("required-registry-parent-missing", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             \x20     upstream: superpowers\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let cap = find(&inv, "superpowers");
    assert_eq!(cap.profile.as_deref(), Some("required"));
    assert_eq!(cap.registry_status, RegistryStatus::Registered);
    assert!(!cap.canonical_present);
    assert!(cap.expected_hosts.iter().any(|host| host == "codex"));
    assert_eq!(
        cap.host_visibility
            .iter()
            .find(|visibility| visibility.host == "codex")
            .map(|visibility| visibility.status.clone()),
        Some(HostVisibilityStatus::NotVisible)
    );

    let verify = verify_host(&ctx, "codex");
    assert_eq!(verify.status, "incomplete");
    assert!(verify.checks.iter().any(|check| {
        check.name == "superpowers"
            && check.expected
            && check.visibility == HostVisibilityStatus::NotVisible
    }));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn required_host_system_parent_accepts_direct_codex_host_body() {
    let (ctx, base) = ctx_with("required-host-system-direct", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n",
    );
    write_file(
        &ctx.home.join(".codex/skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: host body.\n---\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let cap = find(&inv, "superpowers");
    assert_eq!(cap.managed_status, ManagedStatus::HostSystem);
    assert!(cap.canonical_present);
    assert_eq!(
        cap.host_visibility
            .iter()
            .find(|visibility| visibility.host == "codex")
            .map(|visibility| visibility.status.clone()),
        Some(HostVisibilityStatus::Visible)
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn entrypoint_integrity_degrades_visible_parent_without_expectation_leak() {
    let (ctx, base) = ctx_with("entrypoint-integrity", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
    );
    write_file(
        &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: parent router.\n---\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let parent = find(&inv, "superpowers");
    assert_eq!(parent.health_status, HealthStatus::Degraded);
    assert_eq!(
        parent
            .host_visibility
            .iter()
            .find(|visibility| visibility.host == "codex")
            .map(|visibility| visibility.status.clone()),
        Some(HostVisibilityStatus::Degraded)
    );
    let entrypoint = find(&inv, "verification-before-completion");
    assert!(entrypoint.is_route_target());
    assert!(entrypoint.expected_hosts.is_empty());

    let verify = verify_host(&ctx, "codex");
    assert!(verify.checks.iter().any(|check| {
        check.name == "superpowers"
            && check.expected
            && check.visibility == HostVisibilityStatus::Degraded
    }));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn entrypoint_exposure_shape_degrades_parent_when_playbook_is_standalone() {
    let (ctx, base) = ctx_with("entrypoint-exposure-shape", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
    );
    write_file(
        &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: parent router.\n---\n",
    );
    write_file(
        &ctx.home
            .join(".agents/skills/superpowers/playbooks/verification-before-completion/SKILL.md"),
        "---\nname: verification-before-completion\ndescription: nested playbook.\n---\n",
    );
    write_file(
        &ctx.home
            .join(".codex/skills/verification-before-completion/SKILL.md"),
        "---\nname: verification-before-completion\ndescription: stale standalone entry.\n---\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let parent = find(&inv, "superpowers");
    assert_eq!(parent.health_status, HealthStatus::Degraded);
    assert!(parent.host_visibility.iter().any(|visibility| {
        visibility.host == "codex"
            && visibility.status == HostVisibilityStatus::Degraded
            && visibility
                .evidence
                .iter()
                .any(|item| item.contains("unexpected standalone entrypoint"))
    }));
    let standalone = find(&inv, "verification-before-completion");
    assert!(standalone.expected_hosts.is_empty());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn nested_skill_files_are_rejected_as_host_discoverable_playbooks() {
    let (ctx, base) = ctx_with("nested-playbook-exposure", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
    );
    write_file(
        &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: parent router.\n---\n",
    );
    write_file(
        &ctx.home
            .join(".agents/skills/superpowers/playbooks/verification-before-completion/SKILL.md"),
        "---\nname: verification-before-completion\ndescription: nested playbook.\n---\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let parent = find(&inv, "superpowers");
    assert_eq!(parent.health_status, HealthStatus::Degraded);
    assert!(parent.host_visibility.iter().any(|visibility| {
        visibility.host == "codex"
            && visibility.status == HostVisibilityStatus::Degraded
            && visibility
                .evidence
                .iter()
                .any(|item| item.contains("nested SKILL.md") && item.contains("host-discoverable"))
    }));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn playbook_resources_are_loadable_without_becoming_skills() {
    let (ctx, base) = ctx_with("playbook-resource-shape", canned_list());
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\n\
             skills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: host-system\n\
             route_targets:\n\
             \x20 - name: verification-before-completion\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: verification-before-completion }\n",
    );
    write_file(
        &ctx.home.join(".agents/skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: parent router.\n---\n",
    );
    write_file(
        &ctx.home.join(
            ".agents/skills/superpowers/playbooks/verification-before-completion/PLAYBOOK.md",
        ),
        "# Verification before completion\n",
    );

    let inv = build_inventory(&ctx, &["codex"]);
    let parent = find(&inv, "superpowers");
    assert_eq!(parent.health_status, HealthStatus::Healthy);
    assert!(parent.host_visibility.iter().any(|visibility| {
        visibility.host == "codex" && visibility.status == HostVisibilityStatus::Visible
    }));

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn external_registry_skill_sync_targets_shared_body() {
    let (ctx, base) = ctx_with("external-registry-sync", canned_list());
    let shared = ctx.home.join(".agents/skills/lark-shared");
    link_shared_skill_entry(&ctx, ".claude/skills", "lark-shared");

    let result = sync_plan(&ctx, &["claude-code", "codex", "codebuddy-code"], false);
    let lark = result
        .items
        .iter()
        .find(|item| item.capability == "lark-shared")
        .expect("external Lark skill is syncable");
    assert!(
        lark.blocked_reasons.is_empty(),
        "{:?}",
        lark.blocked_reasons
    );
    assert!(
        lark.planned_writes
            .iter()
            .all(|write| !write.path.contains(".claude/skills/lark-shared")),
        "an exact existing thin index must not be rewritten: {:?}",
        lark.planned_writes
    );
    assert!(lark.planned_writes.iter().any(|write| {
        write.path.contains(".codebuddy/skills/lark-shared")
            && write.from.as_deref() == Some(shared.to_str().unwrap())
    }));
    assert!(
        lark.planned_writes
            .iter()
            .all(|write| !write.path.contains(".codex/skills/lark-shared")),
        "Codex already loads the shared body: {:?}",
        lark.planned_writes
    );
    assert!(
        lark.planned_writes
            .iter()
            .all(|write| write.path != shared.to_string_lossy()),
        "AGS must never mutate the external body"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn external_registry_skill_missing_body_fails_closed() {
    let (ctx, base) = ctx_with("external-registry-missing", canned_list());
    std::fs::remove_dir_all(ctx.home.join(".agents/skills/lark-shared")).unwrap();

    let inv = build_inventory(&ctx, &["claude-code"]);
    let cap = find(&inv, "lark-shared");
    assert_eq!(cap.managed_status, ManagedStatus::Governed);
    assert!(!cap.canonical_present);

    let verify = verify_host(&ctx, "claude-code");
    let check = verify
        .checks
        .iter()
        .find(|check| check.name == "lark-shared")
        .unwrap();
    assert!(check.expected);
    assert!(!verify.summary.all_visible);
    assert!(verify.summary.failed >= 1);
    assert_eq!(verify.status, "incomplete");

    let result = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
    assert!(!result.applied);
    assert!(result.planned_writes.is_empty());
    assert!(
        result
            .blocked_reasons
            .iter()
            .any(|reason| reason.contains("Canonical SKILL.md not found")),
        "{:?}",
        result.blocked_reasons
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn external_registry_skill_body_symlink_outside_shared_root_fails_closed() {
    let (ctx, base) = ctx_with("external-registry-outside", canned_list());
    let shared = ctx.home.join(".agents/skills/lark-shared");
    std::fs::remove_dir_all(&shared).unwrap();
    let outside = base.join("external/lark-shared");
    write_file(
        &outside.join("SKILL.md"),
        "---\nname: lark-shared\ndescription: outside body.\n---\n",
    );
    make_symlink(&outside, &shared).unwrap();

    let inv = build_inventory(&ctx, &["codex"]);
    let cap = find(&inv, "lark-shared");
    assert!(!cap.canonical_present);
    assert_eq!(
        cap.host_visibility[0].status,
        HostVisibilityStatus::Degraded
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn external_registry_skill_apply_writes_only_needed_thin_index() {
    let (ctx, base) = ctx_with("external-registry-apply", canned_list());
    let shared = ctx.home.join(".agents/skills/lark-shared");
    let original = std::fs::read_to_string(shared.join("SKILL.md")).unwrap();
    link_shared_skill_entry(&ctx, ".claude/skills", "lark-shared");

    let result = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
    assert!(result.applied, "{:?}", result.apply_errors);
    assert_eq!(result.applied_writes.len(), 1);
    assert!(result.applied_writes[0].contains(".codebuddy/skills/lark-shared"));
    assert!(std::fs::symlink_metadata(ctx.home.join(".codex/skills/lark-shared")).is_err());
    let codebuddy = ctx.home.join(".codebuddy/skills/lark-shared");
    assert!(std::fs::symlink_metadata(&codebuddy)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        std::fs::canonicalize(codebuddy).unwrap(),
        std::fs::canonicalize(&shared).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(shared.join("SKILL.md")).unwrap(),
        original
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// Manifest is the single routing authority, end-to-end: a `routing:` block
/// in skills-registry.yaml / mcp-registry.yaml is parsed; an entry with no
/// block is absent (never synthesized); a malformed block fails closed to
/// absent rather than panicking.
#[test]
fn read_routing_metadata_parses_manifests_and_fails_closed() {
    let base = std::env::temp_dir().join(format!("ags-routing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");

    write_file(
            &repo.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: current-skill\n    routing:\n      route_state: routable\n      intent_tags: [debug, diagnosing-bugs]\n      mutation_surface: read-only\n      requires_auth: false\n      cost_class: free\n      invoke_hint: \"[skill: current-skill]\"\n      route_priority: 10\n\
             \x20 - name: no-routing-skill\n    description: has no routing block\n\
             \x20 - name: broken-skill\n    routing: \"not-a-mapping\"\n",
        );
    write_file(
            &repo.join("manifests/mcp-registry.yaml"),
            "mcps:\n\
             \x20 - name: context7\n    profile: required\n    routing:\n      route_state: routable\n      intent_tags: [docs-lookup]\n      cost_class: network\n      route_priority: 30\n",
        );

    let read = read_routing_metadata(&repo);
    let map = &read.map;

    // Well-formed skill block: stable routing facts parsed.
    let ad = map
        .get("current-skill")
        .expect("current-skill routing present");
    assert_eq!(
        ad.intent_tags,
        vec!["debug".to_string(), "diagnosing-bugs".to_string()]
    );
    assert_eq!(ad.route_priority, 10);
    assert_eq!(ad.mutation_surface, MutationSurface::ReadOnly);

    // MCP routing block parsed from the other manifest.
    let c7 = map.get("context7").expect("context7 routing present");
    assert_eq!(c7.intent_tags, vec!["docs-lookup".to_string()]);
    assert_eq!(c7.cost_class, CostClass::Network);
    assert!(
        read.required_skill_parents
            .iter()
            .all(|skill| skill.name != "context7"),
        "a required MCP registry entry must not synthesize a required skill body"
    );

    // No routing block → absent (single authority, no synthesis).
    assert!(map.get("no-routing-skill").is_none());
    // Malformed block → fail-closed absent from the map, never a panic...
    assert!(map.get("broken-skill").is_none());
    // ...but the failure is SURFACED (not silently swallowed) for doctor.
    assert!(read.parse_failures.contains(&"broken-skill".to_string()));

    let _ = std::fs::remove_dir_all(&base);
}

/// `route_state` parses all three explicit values, and absence defaults to
/// the most restrictive `not-routable` (fail-closed).
#[test]
fn route_state_parses_and_defaults_fail_closed() {
    let routable: RoutingMetadata =
        serde_yaml::from_str("route_state: routable\nintent_tags: [verify]\n").unwrap();
    assert_eq!(routable.route_state, RouteState::Routable);
    let retired: RoutingMetadata = serde_yaml::from_str("route_state: retired\n").unwrap();
    assert_eq!(retired.route_state, RouteState::Retired);
    let not_routable: RoutingMetadata =
        serde_yaml::from_str("route_state: not-routable\n").unwrap();
    assert_eq!(not_routable.route_state, RouteState::NotRoutable);
    // Absent → fail-closed not-routable.
    let absent: RoutingMetadata = serde_yaml::from_str("intent_tags: [verify]\n").unwrap();
    assert_eq!(absent.route_state, RouteState::NotRoutable);
}

/// capability_group (multi-membership), upstream_group, and examples parse as
/// plain labels / fixtures.
#[test]
fn capability_group_upstream_and_examples_parse_as_labels() {
    let yaml = "route_state: routable\ncapability_group: [code-review, verification]\nupstream_group: \"obra/superpowers:requesting-code-review\"\nexamples:\n\x20 positive: [\"帮我做一次代码审查\"]\n\x20 negative: [\"帮我查飞书日历\"]\n";
    let meta: RoutingMetadata = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(
        meta.capability_group,
        vec!["code-review".to_string(), "verification".to_string()]
    );
    assert_eq!(
        meta.upstream_group.as_deref(),
        Some("obra/superpowers:requesting-code-review")
    );
    assert_eq!(
        meta.examples.positive,
        vec!["帮我做一次代码审查".to_string()]
    );
    assert_eq!(meta.examples.negative, vec!["帮我查飞书日历".to_string()]);
}

#[cfg(unix)]
fn link_skill_entry(ctx: &ConsoleContext, host_subdir: &str, name: &str, source: &str) {
    let parent = ctx.home.join(host_subdir);
    std::fs::create_dir_all(&parent).unwrap();
    make_symlink(&ctx.repo_root.join(source), &parent.join(name)).unwrap();
}

#[cfg(unix)]
fn link_shared_skill_entry(ctx: &ConsoleContext, host_subdir: &str, name: &str) {
    let parent = ctx.home.join(host_subdir);
    std::fs::create_dir_all(&parent).unwrap();
    make_symlink(
        &ctx.home.join(".agents/skills").join(name),
        &parent.join(name),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn omp_uses_native_thin_index_and_shared_agents_source() {
    let (ctx, base) = ctx_with("omp-skill-roots", canned_list());

    let inventory = build_inventory(&ctx, &["omp"]);
    let shared = &find(&inventory, "lark-shared").host_visibility[0];
    assert_eq!(shared.host, "omp");
    assert!(shared.supported);
    assert_eq!(shared.status, HostVisibilityStatus::Visible);
    assert!(shared
        .evidence
        .iter()
        .any(|evidence| evidence.contains("shared skill source visible")));

    link_skill_entry(
        &ctx,
        ".omp/agent/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    let inventory = build_inventory(&ctx, &["omp"]);
    let native = &find(&inventory, "demo-skill").host_visibility[0];
    assert_eq!(native.status, HostVisibilityStatus::Visible);

    let ags = &find(&inventory, "ags").host_visibility[0];
    assert!(find(&inventory, "ags")
        .expected_hosts
        .iter()
        .any(|host| host == "omp"));
    assert!(ags
        .evidence
        .iter()
        .any(|evidence| evidence.contains("OMP runtime probe NOT_RUN")));
    assert_eq!(
        find(&inventory, "ags").health_status,
        HealthStatus::Unknown,
        "registration-source-only evidence must never imply live runtime health"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn imported_skill_distribution_supports_omp_without_duplicate_shared_entry() {
    let (ctx, base) = ctx_with("omp-imported-distribution", canned_list());
    let body_root = ctx.home.join(".ags/runtime/skill-bodies");
    let canonical = body_root.join("apple-design");
    write_file(
        &canonical.join("SKILL.md"),
        "---\nname: apple-design\ndescription: Apple design guidance.\n---\n",
    );
    let hosts = vec!["omp".to_string()];
    let shared = ctx.home.join(".agents/skills/apple-design");
    std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&canonical, &shared).unwrap();

    let applied =
        distribute_external_skill(&ctx, "apple-design", &canonical, &body_root, &hosts, true);
    assert_eq!(applied.apply_status, "nothing-to-do", "{applied:?}");
    let entry = ctx.home.join(".omp/agent/skills/apple-design");
    assert!(
        std::fs::symlink_metadata(&entry).is_err(),
        "OMP native entry must be skipped when the shared entry resolves to the same body"
    );

    let removed =
        remove_external_skill_distribution(&ctx, "apple-design", &canonical, &hosts, true);
    assert_eq!(removed.apply_status, "nothing-to-do", "{removed:?}");
    assert!(
        shared.exists(),
        "shared entry is not owned by this transaction"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn imported_skill_distribution_blocks_unrelated_omp_shared_collision() {
    let (ctx, base) = ctx_with("omp-imported-shared-collision", canned_list());
    let body_root = ctx.home.join(".ags/runtime/skill-bodies");
    let canonical = body_root.join("apple-design");
    write_file(
        &canonical.join("SKILL.md"),
        "---\nname: apple-design\ndescription: Apple design guidance.\n---\n",
    );
    let unrelated = base.join("unrelated/apple-design");
    write_file(
        &unrelated.join("SKILL.md"),
        "---\nname: apple-design\ndescription: Unrelated body.\n---\n",
    );
    let shared = ctx.home.join(".agents/skills/apple-design");
    std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&unrelated, &shared).unwrap();
    let native = ctx.home.join(".omp/agent/skills/apple-design");
    std::fs::create_dir_all(native.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&canonical, &native).unwrap();

    let result = distribute_external_skill(
        &ctx,
        "apple-design",
        &canonical,
        &body_root,
        &["omp".to_string()],
        true,
    );
    assert!(!result.applied, "{result:?}");
    assert!(result
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("shared skill entry")));
    assert_eq!(
        std::fs::canonicalize(&native).unwrap(),
        std::fs::canonicalize(&canonical).unwrap(),
        "blocked distribution must preserve the existing exact native entry"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn imported_skill_distribution_blocks_exact_omp_duplicate_entries() {
    let (ctx, base) = ctx_with("omp-imported-exact-duplicate", canned_list());
    let body_root = ctx.home.join(".ags/runtime/skill-bodies");
    let canonical = body_root.join("apple-design");
    write_file(
        &canonical.join("SKILL.md"),
        "---\nname: apple-design\ndescription: Apple design guidance.\n---\n",
    );
    for entry in [
        ctx.home.join(".agents/skills/apple-design"),
        ctx.home.join(".omp/agent/skills/apple-design"),
    ] {
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&canonical, &entry).unwrap();
    }

    let result = distribute_external_skill(
        &ctx,
        "apple-design",
        &canonical,
        &body_root,
        &["omp".to_string()],
        true,
    );
    assert!(!result.applied, "{result:?}");
    assert!(result
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("duplicate canonical skill entries")));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
fn write_codex_plugin_skill(ctx: &ConsoleContext, name: &str) {
    write_codex_plugin_skill_with_enabled(ctx, name, true);
}

#[cfg(unix)]
fn write_codex_plugin_skill_with_enabled(ctx: &ConsoleContext, name: &str, enabled: bool) {
    write_file(
        &ctx.home
            .join(".codex/plugins/cache/openai-curated/superpowers/test/skills")
            .join(name)
            .join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: plugin skill.\n---\nbody\n"),
    );
    write_file(
        &ctx.home.join(".codex/config.toml"),
        &format!("[plugins.\"superpowers@openai-curated\"]\nenabled = {enabled}\n"),
    );
}

fn find<'a>(inv: &'a ManagedInventoryResult, name: &str) -> &'a ManagedCapability {
    inv.capabilities
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("capability '{name}' not found"))
}

#[test]
fn inventory_distinguishes_all_four_kinds() {
    let (ctx, base) = ctx_with("kinds", canned_list());
    let inv = build_inventory(&ctx, &["claude-code"]);

    assert_eq!(find(&inv, "lark-shared").kind, ManagedKind::Skill);
    assert_eq!(find(&inv, "ags").kind, ManagedKind::SuiteInterface);
    assert_eq!(find(&inv, "context7").kind, ManagedKind::Mcp);
    // external-cli MCP → CLI-backed
    assert_eq!(find(&inv, "codegraph").kind, ManagedKind::CliBacked);
    // synthetic CLI binary for the lark family
    assert_eq!(find(&inv, "lark-cli").kind, ManagedKind::CliBacked);
    // on-disk skill not in the manifest
    assert_eq!(
        find(&inv, "orphan-skill").managed_status,
        ManagedStatus::Discovered
    );
    assert_eq!(
        find(&inv, "orphan-skill").registry_status,
        RegistryStatus::NotRegistered
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn suite_and_external_managed_statuses_are_set() {
    let (ctx, base) = ctx_with("status", canned_list());
    let inv = build_inventory(&ctx, &["claude-code"]);
    let lark = find(&inv, "lark-shared");
    assert_eq!(lark.managed_status, ManagedStatus::Governed);
    assert_eq!(lark.registry_status, RegistryStatus::Registered);
    assert_eq!(
        find(&inv, "demo-skill").managed_status,
        ManagedStatus::SuiteManaged
    );
    let ags = find(&inv, "ags");
    assert_eq!(ags.managed_status, ManagedStatus::SuiteInterface);
    // ags offers only verify — it can't be removed via the console
    assert_eq!(ags.actions, vec!["verify".to_string()]);
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn claude_skill_path_visible_only_when_entry_present() {
    let (ctx, base) = ctx_with("skillpath", canned_list());
    // Distribute only lark-shared's host entry.
    link_shared_skill_entry(&ctx, ".claude/skills", "lark-shared");
    let inv = build_inventory(&ctx, &["claude-code"]);

    let lark_vis = &find(&inv, "lark-shared").host_visibility[0];
    assert_eq!(lark_vis.host, "claude-code");
    assert_eq!(lark_vis.status, HostVisibilityStatus::Visible);

    let demo_vis = &find(&inv, "demo-skill").host_visibility[0];
    assert_eq!(demo_vis.status, HostVisibilityStatus::NotVisible);

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn dangling_symlink_skill_is_degraded() {
    let (ctx, base) = ctx_with("dangling", canned_list());
    let skills = ctx.home.join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    std::os::unix::fs::symlink(base.join("nonexistent-target"), skills.join("demo-skill")).unwrap();
    let inv = build_inventory(&ctx, &["claude-code"]);
    assert_eq!(
        find(&inv, "demo-skill").host_visibility[0].status,
        HostVisibilityStatus::Degraded
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mcp_visibility_from_claude_list() {
    let (ctx, base) = ctx_with("mcpvis", canned_list());
    let inv = build_inventory(&ctx, &["claude-code"]);
    // context7 + ags are in the canned list → visible
    assert_eq!(
        find(&inv, "context7").host_visibility[0].status,
        HostVisibilityStatus::Visible
    );
    assert_eq!(
        find(&inv, "ags").host_visibility[0].status,
        HostVisibilityStatus::Visible
    );
    assert_eq!(
        find(&inv, "plugin:claude-mem:mcp-search").host_visibility[0].status,
        HostVisibilityStatus::Visible
    );
    // codegraph is NOT in the canned list → not visible
    assert_eq!(
        find(&inv, "codegraph").host_visibility[0].status,
        HostVisibilityStatus::NotVisible
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mcp_visibility_degraded_when_claude_unavailable() {
    let (ctx, base) = ctx_with("mcpunavail", CommandOutcome::Unavailable);
    let inv = build_inventory(&ctx, &["claude-code"]);
    // No panic; MCP checks degrade gracefully.
    assert_eq!(
        find(&inv, "context7").host_visibility[0].status,
        HostVisibilityStatus::Degraded
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn codex_skill_path_and_mcp_visibility_are_real() {
    let (ctx, base) = ctx_with("codexreal", canned_list());
    // Distribute a skill entry into the CODEX skills dir (~/.codex/skills).
    link_shared_skill_entry(&ctx, ".codex/skills", "lark-shared");
    let inv = build_inventory(&ctx, &["codex"]);

    // Codex is now a real (supported) host — not deferred.
    let lark = &find(&inv, "lark-shared").host_visibility[0];
    assert_eq!(lark.host, "codex");
    assert!(lark.supported);
    assert_eq!(
        lark.status,
        HostVisibilityStatus::Degraded,
        "native plus shared Codex entries are an ambiguous duplicate"
    );
    assert!(lark
        .evidence
        .iter()
        .any(|evidence| evidence.contains("duplicate host entry")));

    // MCP visibility from `codex mcp list`: context7 present, codegraph absent.
    assert_eq!(
        find(&inv, "context7").host_visibility[0].status,
        HostVisibilityStatus::Visible
    );
    assert_eq!(
        find(&inv, "codegraph").host_visibility[0].status,
        HostVisibilityStatus::NotVisible
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn codex_skill_path_can_use_shared_agents_source() {
    let (ctx, base) = ctx_with("codexshared", canned_list());
    let inv = build_inventory(&ctx, &["codex"]);

    let lark = &find(&inv, "lark-shared").host_visibility[0];
    assert_eq!(lark.host, "codex");
    assert!(lark.supported);
    assert_eq!(lark.status, HostVisibilityStatus::Visible);
    assert!(
        lark.evidence
            .iter()
            .any(|e| e.contains("shared skill source visible")),
        "Codex visibility should cite the shared .agents source: {:?}",
        lark.evidence
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn codex_skill_path_can_use_plugin_source() {
    let (ctx, base) = ctx_with("codexplugin", canned_list());
    write_codex_plugin_skill(&ctx, "demo-skill");
    let inv = build_inventory(&ctx, &["codex"]);

    let demo = &find(&inv, "demo-skill").host_visibility[0];
    assert_eq!(demo.host, "codex");
    assert_eq!(demo.status, HostVisibilityStatus::Visible);
    assert!(
        demo.evidence
            .iter()
            .any(|e| e.contains(".codex/plugins/cache")),
        "Codex visibility should cite the plugin source: {:?}",
        demo.evidence
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn codex_disabled_plugin_cache_is_not_runtime_visible() {
    let (ctx, base) = ctx_with("codexplugindisabled", canned_list());
    write_codex_plugin_skill_with_enabled(&ctx, "demo-skill", false);
    let inv = build_inventory(&ctx, &["codex"]);

    let demo = &find(&inv, "demo-skill").host_visibility[0];
    assert_eq!(demo.host, "codex");
    assert_eq!(demo.status, HostVisibilityStatus::NotVisible);
    assert!(
        demo.evidence
            .iter()
            .all(|e| !e.contains(".codex/plugins/cache")),
        "a disabled plugin cache must not count as runtime visibility: {:?}",
        demo.evidence
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn codebuddy_skill_path_visibility_is_real() {
    let (ctx, base) = ctx_with("codebuddyreal", canned_list());
    link_skill_entry(
        &ctx,
        ".codebuddy/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    link_shared_skill_entry(&ctx, ".codebuddy/skills", "lark-shared");

    let inv = build_inventory(&ctx, &["codebuddy-code"]);
    let demo = &find(&inv, "demo-skill").host_visibility[0];
    assert_eq!(demo.host, "codebuddy-code");
    assert!(demo.supported);
    assert_eq!(demo.status, HostVisibilityStatus::Visible);

    let verify = verify_host(&ctx, "codebuddy-code");
    assert!(verify.supported);
    assert_eq!(verify.summary.failed, 0);
    assert!(verify.summary.all_visible);
    let expected_demo = verify
        .checks
        .iter()
        .find(|c| c.name == "demo-skill")
        .unwrap();
    assert!(expected_demo.expected);
    assert_eq!(expected_demo.visibility, HostVisibilityStatus::Visible);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn codex_verify_incomplete_when_expected_mcp_missing() {
    let (ctx, base) = ctx_with("codexverify", canned_list());
    // codegraph is codex-expected (installed_clients=[codex]) but absent
    // from the canned `codex mcp list` → verify must NOT report ok.
    let v = verify_host(&ctx, "codex");
    assert!(v.supported);
    assert_eq!(v.status, "incomplete");
    assert!(!v.summary.all_visible);
    let cg = v.checks.iter().find(|c| c.name == "codegraph").unwrap();
    assert!(cg.expected);
    assert_eq!(cg.visibility, HostVisibilityStatus::NotVisible);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn cursor_host_uses_real_visibility_and_verify_fields() {
    let (ctx, base) = ctx_with("cursor", canned_list());
    let inv = build_inventory(&ctx, &["cursor"]);
    let v = &find(&inv, "lark-shared").host_visibility[0];
    assert_eq!(v.host, "cursor");
    assert!(v.supported);
    assert_ne!(v.status, HostVisibilityStatus::Deferred);

    let verify = verify_host(&ctx, "cursor");
    assert!(verify.supported);
    assert_ne!(verify.status, "unsupported");
    assert!(!verify.checks.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn verify_host_claude_reports_per_capability_checks() {
    let (ctx, base) = ctx_with("verifyclaude", canned_list());
    let v = verify_host(&ctx, "claude-code");
    assert!(v.supported);
    assert!(v.summary.total > 0);
    assert!(v.checks.iter().any(|c| c.name == "context7"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn propose_dry_run_writes_nothing() {
    let (ctx, base) = ctx_with("dryrun", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", false);
    assert!(res.found);
    assert!(!res.applied);
    assert!(res.applied_writes.is_empty());
    assert!(
        !res.planned_writes.is_empty(),
        "dry-run still shows the plan"
    );
    // Crucially: nothing was written to the (injected) home.
    assert!(!ctx
        .home
        .join(".claude/skills/lark-shared/SKILL.md")
        .exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn propose_apply_writes_thin_index_symlink_on_all_hosts() {
    let (ctx, base) = ctx_with("applywrite", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
    assert!(res.applied);
    assert!(res.apply_errors.is_empty());
    // P1.1 + thin index: all supported skill hosts get a symlink (not a copy) into the
    // injected home, and SKILL.md is reachable THROUGH it (canonical body).
    for sub in [".claude/skills", ".codebuddy/skills"] {
        let entry = ctx.home.join(sub).join("lark-shared");
        let meta = std::fs::symlink_metadata(&entry).unwrap();
        assert!(
            meta.file_type().is_symlink(),
            "{sub} entry must be a symlink"
        );
        let md = std::fs::read_to_string(entry.join("SKILL.md")).unwrap();
        assert!(md.contains("name: lark-shared"));
    }
    assert!(
        std::fs::symlink_metadata(ctx.home.join(".codex/skills/lark-shared")).is_err(),
        "Codex must not receive a duplicate entry for the shared skill"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn propose_apply_replaces_existing_entry_without_backup() {
    let (ctx, base) = ctx_with("applyreplace", canned_list());
    // A pre-existing REAL dir entry on claude (e.g. a manual copy).
    let entry = ctx.home.join(".claude/skills/lark-shared");
    write_file(&entry.join("SKILL.md"), "OLD CONTENT");
    let res = propose_action(&ctx, ConsoleAction::Update, "lark-shared", true);
    assert!(res.applied);
    // Capability/skill thin-index relink replaces the host entry in place
    // and must not leave backup clutter in the host skills directory.
    assert!(
        !ctx.home.join(".claude/skills/lark-shared.bak").exists(),
        "thin-index relink must not leave .bak entries"
    );
    assert!(
        !ctx.home.join(".claude/skills/lark-shared.bak.1").exists(),
        "thin-index relink must not leave numbered .bak entries"
    );
    // The active entry is now a symlink to the canonical body.
    assert!(std::fs::symlink_metadata(&entry)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::read_to_string(entry.join("SKILL.md"))
        .unwrap()
        .contains("name: lark-shared"));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn missing_capability_apply_writes_nothing() {
    let (ctx, base) = ctx_with("missing", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "does-not-exist", true);
    assert!(!res.found);
    assert!(!res.applied);
    assert!(res.applied_writes.is_empty());
    assert!(!res.blocked_reasons.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn suite_interface_cannot_be_mutated() {
    let (ctx, base) = ctx_with("ifacelock", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Remove, "ags", true);
    assert!(res.found);
    assert!(!res.blocked_reasons.is_empty());
    assert!(!res.applied);
    assert!(res.applied_writes.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

// R3-1: an MCP `--apply` must NOT report applied — AGS only advised.
#[test]
fn mcp_action_advises_but_never_writes_or_runs() {
    // StrictMcpRunner would panic if apply tried to run anything other than
    // `<host> mcp list`, so a clean run proves no installer ran.
    let (ctx, base) = ctx_with("mcpadvise", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "context7", true);
    assert!(res.found);
    assert!(res.planned_writes.is_empty(), "AGS owns no file for an MCP");
    assert!(res.applied_writes.is_empty());
    // The high-severity finding: applied must be FALSE (AGS did nothing).
    assert!(!res.applied, "MCP apply must not report applied=true");
    assert_eq!(res.apply_status, "advised-only");
    assert!(res
        .advised_commands
        .iter()
        .any(|c| c.command.contains("claude mcp add")));
    let _ = std::fs::remove_dir_all(&base);
}

// R3-1: a successful skill apply reports applied=true / status "applied".
#[cfg(unix)]
#[test]
fn skill_apply_status_is_applied() {
    let (ctx, base) = ctx_with("applystatus", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "demo-skill", true);
    assert!(res.applied);
    assert_eq!(res.apply_status, "applied");
    assert!(!res.applied_writes.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn lark_distinction_is_explicit() {
    let (ctx, base) = ctx_with("lark", canned_list());
    // Host skill path present; lark is NOT an MCP in the list.
    link_shared_skill_entry(&ctx, ".claude/skills", "lark-shared");
    let inv = build_inventory(&ctx, &["claude-code"]);

    // 1. lark-* skill — fronted by lark-cli (risk note), claude skill path visible.
    let lark = find(&inv, "lark-shared");
    assert_eq!(lark.kind, ManagedKind::Skill);
    assert!(lark.risk_notes.iter().any(|r| r.contains("lark-cli")));
    assert_eq!(
        lark.host_visibility[0].status,
        HostVisibilityStatus::Visible
    );

    // 2. lark-cli binary — distinct CLI-backed capability, Feishu endpoint, degraded health.
    let cli = find(&inv, "lark-cli");
    assert_eq!(cli.kind, ManagedKind::CliBacked);
    assert_eq!(cli.managed_status, ManagedStatus::Unmanaged);
    assert_eq!(cli.health_status, HealthStatus::Degraded);
    assert!(cli.risk_notes.iter().any(|r| r.contains("Feishu")));

    // 3. There is no MCP named "lark" — lark is CLI-backed, not MCP-registered.
    assert!(!inv
        .capabilities
        .iter()
        .any(|c| c.name == "lark" && c.kind == ManagedKind::Mcp));

    let _ = std::fs::remove_dir_all(&base);
}

/// Read-only thin-index drift scan classifies `.bak` leftovers and dangling
/// symlinks as drift and a valid symlink as clean — never mutating. (point 2)
#[cfg(unix)]
#[test]
fn scan_thin_index_drift_classifies_bak_and_dangling() {
    let base = std::env::temp_dir().join(format!("ags-drift-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let skills = base.join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    let target = base.join("canon-target");
    std::fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&target, skills.join("clean-skill")).unwrap();
    std::os::unix::fs::symlink(&target, skills.join("clean-skill.bak")).unwrap();
    std::os::unix::fs::symlink(base.join("missing-target"), skills.join("auto-gone")).unwrap();

    let drift = scan_thin_index_drift(&base, "claude-code").expect("scan present");
    assert!(drift.has_drift);
    assert_eq!(drift.bak_leftovers, 1, "one .bak leftover");
    assert_eq!(drift.broken_symlinks, 1, "one dangling symlink");
    assert!(drift.clean_symlinks >= 1, "clean symlink counted");
    // unsupported host has no skills subdir → None.
    assert!(scan_thin_index_drift(&base, "cursor").is_none());

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn shared_thin_index_drift_degrades_codex_verify() {
    let (original, base) = ctx_with("shared-drift", canned_list());
    let ctx = ConsoleContext::new(
        original.repo_root.clone(),
        original.home.clone(),
        Box::new(StrictMcpRunner {
            claude: canned_list(),
            codex: CommandOutcome::Ran {
                success: true,
                stdout: "Name       Command   Args   Env   Cwd   Status   Auth\n\
                             ags        ags       mcp    -     -     enabled  Unsupported\n\
                             context7   npx       args   -     -     enabled  Unsupported\n\
                             codegraph  cg        mcp    -     -     enabled  Unsupported\n"
                    .to_string(),
            },
        }),
    );
    link_skill_entry(
        &ctx,
        ".agents/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    let shared = ctx.home.join(".agents/skills");
    std::fs::create_dir_all(&shared).unwrap();
    std::os::unix::fs::symlink(
        ctx.home.join("missing-retired-playbook"),
        shared.join("brainstorming"),
    )
    .unwrap();

    let verify = verify_host(&ctx, "codex");
    let json: serde_json::Value = serde_json::from_str(&render_verify_json(&verify)).unwrap();

    assert_eq!(json["status"], "degraded");
    assert_eq!(
        json["shared_thin_index_drift"]["skills_dir"],
        shared.display().to_string()
    );
    assert_eq!(json["shared_thin_index_drift"]["broken_symlinks"], 1);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn all_renders_produce_parseable_json() {
    let (ctx, base) = ctx_with("json", canned_list());
    let inv = build_inventory(&ctx, &["claude-code"]);
    let verify = verify_host(&ctx, "claude-code");
    let proposal = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", false);

    for json in [
        render_inventory_json(&inv),
        render_verify_json(&verify),
        render_proposal_json(&proposal),
    ] {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "render must be valid JSON: {json}");
    }
    // Round-trip the inventory through the public type.
    let reparsed: ManagedInventoryResult =
        serde_json::from_str(&render_inventory_json(&inv)).unwrap();
    assert_eq!(reparsed.schema_version, CONSOLE_SCHEMA_VERSION);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn console_action_parsing_roundtrips() {
    for a in CONSOLE_ACTIONS {
        let parsed = ConsoleAction::parse(a).expect("known action");
        assert_eq!(parsed.as_str(), *a);
    }
    assert!(ConsoleAction::parse("bogus").is_none());
}

// ── Adversarial-review regression tests ──────────────────────────────────

// Finding 1 + R3-2: apply must not report success when a write fails, AND
// the multi-host preflight must abort with ZERO cross-host drift.
#[test]
fn apply_failure_propagates_with_no_cross_host_drift() {
    let (ctx, base) = ctx_with("applyfail", canned_list());
    // Occupy ~/.codex/skills with a FILE so the codex destination fails
    // preflight — claude must NOT be mutated as a result.
    let codex_skills = ctx.home.join(".codex/skills");
    std::fs::create_dir_all(codex_skills.parent().unwrap()).unwrap();
    std::fs::write(&codex_skills, "not a dir").unwrap();

    let res = propose_action(&ctx, ConsoleAction::Adopt, "demo-skill", true);
    assert!(res.found);
    assert!(
        !res.apply_errors.is_empty(),
        "a host write failure must be recorded"
    );
    assert!(!res.applied, "applied must be false when any write errors");
    assert_eq!(res.apply_status, "failed");
    assert!(
        res.applied_writes.is_empty(),
        "preflight abort → zero writes"
    );
    // No cross-host drift: claude's entry was never created.
    assert!(
        std::fs::symlink_metadata(ctx.home.join(".claude/skills/demo-skill")).is_err(),
        "claude must be untouched when codex preflight fails"
    );
    assert!(
        std::fs::symlink_metadata(ctx.home.join(".claude")).is_err(),
        "read-only preflight must not create host directories"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// R3-2: a per-host relink failure leaves the existing entry intact (rollback).
#[cfg(unix)]
#[test]
fn relink_failure_leaves_existing_entry_intact() {
    use std::os::unix::fs::PermissionsExt;
    let (ctx, base) = ctx_with("relinkfail", canned_list());
    // A working pre-existing entry on claude.
    let skills = ctx.home.join(".claude/skills");
    let entry = skills.join("lark-shared");
    write_file(&entry.join("SKILL.md"), "ORIGINAL WORKING ENTRY");
    // Make the claude skills dir read-only so staging the new symlink fails
    // AFTER preflight (which only needs the dir to exist).
    std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o555)).unwrap();

    let res = propose_action(&ctx, ConsoleAction::Update, "lark-shared", true);

    // Restore perms so the entry stays readable for assertions + cleanup.
    std::fs::set_permissions(&skills, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(!res.applied);
    assert!(!res.apply_errors.is_empty(), "staging failure recorded");
    // The original entry is intact — NOT half-removed into a bare .bak.
    assert_eq!(
        std::fs::read_to_string(entry.join("SKILL.md")).unwrap(),
        "ORIGINAL WORKING ENTRY"
    );
    assert!(
        std::fs::symlink_metadata(skills.join("lark-shared.ags-tmp")).is_err(),
        "no staging leftover"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// R4-1: if a later host fails during execution, earlier host changes roll back.
#[cfg(unix)]
#[test]
fn later_host_execution_failure_rolls_back_earlier_host() {
    use std::os::unix::fs::PermissionsExt;
    let (ctx, base) = ctx_with("batchrollback", canned_list());
    std::fs::create_dir_all(&ctx.home).unwrap();
    let codex_skills = ctx.home.join(".codex/skills");
    std::fs::create_dir_all(&codex_skills).unwrap();
    std::fs::set_permissions(&codex_skills, std::fs::Permissions::from_mode(0o555)).unwrap();

    let res = propose_action(&ctx, ConsoleAction::Adopt, "demo-skill", true);

    std::fs::set_permissions(&codex_skills, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!res.applied);
    assert_eq!(res.apply_status, "failed");
    assert!(!res.apply_errors.is_empty());
    assert!(
        res.applied_writes.is_empty(),
        "failed batch must not report retained writes"
    );
    assert!(
        std::fs::symlink_metadata(ctx.home.join(".claude/skills/demo-skill")).is_err(),
        "claude relink must be rolled back when codex fails later"
    );
    assert!(
        std::fs::symlink_metadata(ctx.home.join(".claude")).is_err(),
        "directories created only for the rolled-back host must be removed"
    );
    assert!(
        std::fs::symlink_metadata(codex_skills.join("demo-skill.ags-tmp")).is_err(),
        "failed codex staging path must be cleaned"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// Finding 2: verify must not report ok when an expected capability is missing.
#[test]
fn verify_incomplete_when_expected_skill_not_visible() {
    let (ctx, base) = ctx_with("verifymissing", canned_list());
    // demo-skill is a required suite skill (expected) but no host entry exists.
    let v = verify_host(&ctx, "claude-code");
    assert!(v.supported);
    assert!(!v.summary.all_visible);
    assert!(v.summary.failed >= 1);
    assert_eq!(v.status, "incomplete");
    let demo = v.checks.iter().find(|c| c.name == "demo-skill").unwrap();
    assert!(demo.expected, "required skill is expected-visible");
    assert_eq!(demo.visibility, HostVisibilityStatus::NotVisible);
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn verify_ok_when_expected_skill_visible() {
    let (ctx, base) = ctx_with("verifyok", canned_list());
    // Distribute every expected skill entry → expected set satisfied.
    link_skill_entry(
        &ctx,
        ".claude/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    link_shared_skill_entry(&ctx, ".claude/skills", "lark-shared");
    let v = verify_host(&ctx, "claude-code");
    assert!(v.summary.all_visible, "no expected capability is missing");
    assert_eq!(v.summary.failed, 0);
    assert_eq!(v.status, "ok");
    let _ = std::fs::remove_dir_all(&base);
}

// Finding 3: unsafe capability names must never escape the skills dir.
#[test]
fn is_safe_path_component_rejects_traversal_and_separators() {
    assert!(is_safe_path_component("lark-shared"));
    assert!(is_safe_path_component("demo_skill"));
    for bad in [
        "",
        ".",
        "..",
        "../evil",
        "../../etc/passwd",
        "/etc/passwd",
        "a/b",
        "a\\b",
        "foo/..",
    ] {
        assert!(!is_safe_path_component(bad), "must reject {bad:?}");
    }
}

#[test]
fn within_rejects_escaping_paths() {
    let root = Path::new("/home/.claude/skills");
    assert!(within(Path::new("/home/.claude/skills/foo/SKILL.md"), root));
    assert!(!within(Path::new("/home/.claude/evil/SKILL.md"), root));
    assert!(!within(Path::new("/etc/passwd"), root));
}

#[test]
fn unsafe_discovered_name_blocks_apply_and_writes_nothing() {
    let (ctx, base) = ctx_with("traversal", canned_list());
    // A discovered on-disk skill whose front-matter NAME is a traversal.
    write_file(
        &ctx.repo_root.join("global-skills/evil-dir/SKILL.md"),
        "---\nname: ../../evil\ndescription: hostile name.\n---\n",
    );
    let res = propose_action(&ctx, ConsoleAction::Adopt, "../../evil", true);
    assert!(res.found, "the hostile-named capability is discovered");
    assert!(
        !res.blocked_reasons.is_empty(),
        "unsafe name must be blocked"
    );
    assert!(!res.applied);
    assert!(res.applied_writes.is_empty());
    assert!(res.apply_errors.is_empty(), "blocked before any write");
    // Nothing was created outside the skills dir.
    assert!(!base.join("home/.claude/evil/SKILL.md").exists());
    assert!(!ctx.home.join(".claude/evil/SKILL.md").exists());
    let _ = std::fs::remove_dir_all(&base);
}

// ── Canonical-store / thin-index regression tests ────────────────────────

// Goal 4: a thin index keeps reference files reachable (no SKILL.md-only copy).
#[cfg(unix)]
#[test]
fn thin_index_preserves_reference_files() {
    let (ctx, base) = ctx_with("refs", canned_list());
    // A canonical skill body with a dependency file under references/.
    write_file(
        &ctx.repo_root.join("global-skills/refskill/SKILL.md"),
        "---\nname: refskill\ndescription: needs references.\n---\n",
    );
    write_file(
        &ctx.repo_root
            .join("global-skills/refskill/references/workflow.md"),
        "the workflow lives here",
    );
    let res = propose_action(&ctx, ConsoleAction::Adopt, "refskill", true);
    assert!(res.applied, "{:?}", res.apply_errors);
    // The reference file is reachable THROUGH the host thin index.
    let via_host = ctx
        .home
        .join(".claude/skills/refskill/references/workflow.md");
    assert_eq!(
        std::fs::read_to_string(&via_host).unwrap(),
        "the workflow lives here"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// Goal 3: remove unlinks only the thin index; the canonical body survives.
#[cfg(unix)]
#[test]
fn remove_unlinks_thin_index_keeps_canonical() {
    let (ctx, base) = ctx_with("removeindex", canned_list());
    let canonical = ctx.home.join(".agents/skills/lark-shared/SKILL.md");
    assert!(canonical.is_file());

    assert!(propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true).applied);
    let entry = ctx.home.join(".claude/skills/lark-shared");
    assert!(std::fs::symlink_metadata(&entry)
        .unwrap()
        .file_type()
        .is_symlink());

    let res = propose_action(&ctx, ConsoleAction::Remove, "lark-shared", true);
    assert!(res.applied);
    // Active thin index is gone …
    assert!(std::fs::symlink_metadata(&entry).is_err());
    // … but the canonical body is untouched.
    assert!(canonical.is_file());
    let _ = std::fs::remove_dir_all(&base);
}

// P2.3: a non-zero `mcp list` exit is degraded, not an authoritative empty list.
#[test]
fn probe_failure_is_degraded_not_missing() {
    let failing = CommandOutcome::Ran {
        success: false,
        stdout: String::new(),
    };
    let (ctx, base) = ctx_with("probefail", failing);
    let inv = build_inventory(&ctx, &["claude-code"]);
    // context7 must be degraded (couldn't enumerate), NOT not-visible.
    assert_eq!(
        find(&inv, "context7").host_visibility[0].status,
        HostVisibilityStatus::Degraded
    );
    let _ = std::fs::remove_dir_all(&base);
}

// P2.4: a host entry whose front-matter name differs is not "visible".
#[test]
fn skill_name_mismatch_is_degraded() {
    let (ctx, base) = ctx_with("namemismatch", canned_list());
    // Entry path says lark-shared but the SKILL.md declares a different name.
    write_file(
        &ctx.home.join(".claude/skills/lark-shared/SKILL.md"),
        "---\nname: something-else\ndescription: wrong name.\n---\n",
    );
    let inv = build_inventory(&ctx, &["claude-code"]);
    assert_eq!(
        find(&inv, "lark-shared").host_visibility[0].status,
        HostVisibilityStatus::Degraded
    );
    let _ = std::fs::remove_dir_all(&base);
}

// R4-2: matching front matter is not enough; the host entry must point to the
// AGS canonical body, not a random external directory.
#[cfg(unix)]
#[test]
fn non_canonical_symlink_is_degraded() {
    let (ctx, base) = ctx_with("external-target", canned_list());
    let outside = base.join("outside/lark-shared");
    write_file(
        &outside.join("SKILL.md"),
        "---\nname: lark-shared\ndescription: external copy.\n---\n",
    );
    let skills = ctx.home.join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    make_symlink(&outside, &skills.join("lark-shared")).unwrap();

    let inv = build_inventory(&ctx, &["claude-code"]);
    let vis = &find(&inv, "lark-shared").host_visibility[0];
    assert_eq!(vis.status, HostVisibilityStatus::Degraded);
    assert!(
        vis.evidence
            .iter()
            .any(|e| e.contains("expected AGS canonical")),
        "{:?}",
        vis.evidence
    );
    let _ = std::fs::remove_dir_all(&base);
}

// The development private suite may verify a suite-owned skill whose thin
// index points at the stable runtime twin.
#[cfg(unix)]
#[test]
fn stable_runtime_twin_symlink_is_visible() {
    let (ctx, base) = ctx_with_repo_dir(
        "stable-runtime",
        canned_list(),
        &format!("agent-governance-suite-{}", "private"),
    );
    let stable_source = base.join(format!(
        "agent-governance-suite-{}/global-skills/demo-skill",
        "stable"
    ));
    write_file(
        &stable_source.join("SKILL.md"),
        "---\nname: demo-skill\ndescription: stable runtime body.\n---\n",
    );
    let skills = ctx.home.join(".claude/skills");
    std::fs::create_dir_all(&skills).unwrap();
    make_symlink(&stable_source, &skills.join("demo-skill")).unwrap();

    let inv = build_inventory(&ctx, &["claude-code"]);
    let vis = &find(&inv, "demo-skill").host_visibility[0];
    assert_eq!(vis.status, HostVisibilityStatus::Visible);
    assert!(
        vis.evidence
            .iter()
            .any(|e| e.contains("AGS stable/private runtime twin")),
        "{:?}",
        vis.evidence
    );
    let _ = std::fs::remove_dir_all(&base);
}

// Goal 2: canonical body status is modeled distinctly from host visibility.
#[test]
fn canonical_present_reflects_the_body() {
    let (ctx, base) = ctx_with("canonical", canned_list());
    let inv = build_inventory(&ctx, &["claude-code"]);
    // Suite skill has a canonical SKILL.md in the store.
    assert!(find(&inv, "lark-shared").canonical_present);
    // The synthetic lark-cli binary is external — AGS holds no canonical body.
    assert!(!find(&inv, "lark-cli").canonical_present);
    // Summary counts the canonical bodies present.
    assert!(inv.summary.canonical_present >= 2);
    let _ = std::fs::remove_dir_all(&base);
}

// R3-3: canonical containment helper accepts in-store, rejects out-of-store.
#[test]
fn canonical_within_store_helper() {
    let (ctx, base) = ctx_with("withinstore", canned_list());
    let inside = ctx.repo_root.join("global-skills/demo-skill");
    assert!(canonical_within_store(&ctx.repo_root, &inside));
    let outside = base.join("outside-store");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(!canonical_within_store(&ctx.repo_root, &outside));
    let _ = std::fs::remove_dir_all(&base);
}

// R3-3: a manifest source pointing outside the approved stores is blocked,
// even with a valid SKILL.md — AGS must not link a host to an arbitrary dir.
#[test]
fn canonical_source_outside_store_is_blocked() {
    let (ctx, base) = ctx_with("outsidesrc", canned_list());
    let evil = base.join("evil-store/sneaky");
    write_file(
        &evil.join("SKILL.md"),
        "---\nname: sneaky\ndescription: x.\n---\n",
    );
    // Register it with an ABSOLUTE outside source.
    write_file(
            &ctx.repo_root.join("manifests/suite.yaml"),
            &format!(
                "schema_version: \"1.0\"\nsuite:\n  name: t\n  version: \"9\"\n  optional:\n\
                 \x20   - name: \"sneaky\"\n      version: \"1\"\n      source: {:?}\n      hash: h\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: r\n",
                evil.to_string_lossy()
            ),
        );
    let res = propose_action(&ctx, ConsoleAction::Adopt, "sneaky", true);
    assert!(res.found);
    assert!(
        res.blocked_reasons
            .iter()
            .any(|b| b.contains("outside the store approved")),
        "{:?}",
        res.blocked_reasons
    );
    assert!(!res.applied);
    assert!(res.applied_writes.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

// R3-3: a canonical body whose SKILL.md declares a different name is blocked.
#[test]
fn canonical_name_mismatch_is_blocked() {
    let (ctx, base) = ctx_with("canonmismatch", canned_list());
    // Corrupt the canonical body so its declared name no longer matches.
    write_file(
        &ctx.home.join(".agents/skills/lark-shared/SKILL.md"),
        "---\nname: not-lark-shared\ndescription: mislabeled.\n---\n",
    );
    let res = propose_action(&ctx, ConsoleAction::Adopt, "lark-shared", true);
    assert!(res.found);
    assert!(
        res.blocked_reasons
            .iter()
            .any(|b| b.contains("declares name")),
        "{:?}",
        res.blocked_reasons
    );
    assert!(!res.applied);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn inventory_host_scoping_limits_visibility() {
    let (ctx, base) = ctx_with("hostscope", canned_list());

    // Scope to codex only → every capability's host_visibility is codex.
    let codex_only = build_inventory(&ctx, &["codex"]);
    assert_eq!(codex_only.hosts, vec!["codex".to_string()]);
    for cap in &codex_only.capabilities {
        for v in &cap.host_visibility {
            assert_eq!(v.host, "codex", "host visibility scoped to requested host");
        }
    }

    // Both hosts requested → claude-code visibility is present again.
    let both = build_inventory(&ctx, &["claude-code", "codex"]);
    assert!(
        both.capabilities
            .iter()
            .any(|c| c.host_visibility.iter().any(|v| v.host == "claude-code")),
        "claude-code visibility present when requested"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// ── Cross-Agent capability sync ──────────────────────────────────────

#[test]
fn sync_plan_covers_adopted_and_governed_only() {
    let (ctx, base) = ctx_with("syncdry", canned_list());
    let result = sync_plan(&ctx, &["claude-code", "codex"], false);

    // Syncable = suite-managed demo-skill + externally governed lark-shared
    // + governed MCPs (context7, codegraph). orphan-skill (discovered) and ags
    // (suite-interface) are excluded.
    let names: Vec<&str> = result.items.iter().map(|i| i.capability.as_str()).collect();
    assert!(names.contains(&"demo-skill"));
    assert!(names.contains(&"lark-shared"));
    assert!(names.contains(&"context7"));
    assert!(names.contains(&"codegraph"));
    assert!(!names.contains(&"orphan-skill"), "discovered is not synced");
    assert!(!names.contains(&"ags"), "AGS self is never synced");
    assert_eq!(result.summary.considered, result.items.len());

    // Dry-run: nothing applied; skills plan thin-index writes; MCPs advise.
    assert!(!result.apply_requested);
    assert!(result.items.iter().all(|i| i.apply_status == "dry-run"));
    assert!(result.summary.planned_writes > 0, "skills need thin-index");
    assert!(result.summary.advised_only >= 2, "two governed MCPs advise");
    assert_eq!(result.summary.applied, 0);

    // Renders without panic and reflects dry-run.
    assert!(render_sync_text(&result).contains("Cross-Agent Capability Sync"));
    let json: serde_json::Value = serde_json::from_str(&render_sync_json(&result)).unwrap();
    assert_eq!(json["apply_requested"], false);

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_apply_writes_skill_thin_index_only() {
    let (ctx, base) = ctx_with("syncapply", canned_list());
    let result = sync_plan(&ctx, &["claude-code", "codex", "codebuddy-code"], true);

    // At least one suite skill's thin-index was applied; MCPs stayed advised.
    assert!(result.summary.applied >= 1, "skill thin-index applied");
    assert!(result.apply_requested);
    // A governed MCP item must remain advised-only (AGS ran nothing for it).
    let context7 = result
        .items
        .iter()
        .find(|i| i.capability == "context7")
        .expect("context7 considered");
    assert_eq!(context7.apply_status, "advised-only");
    assert!(!context7.applied);
    // demo-skill thin index now exists under a host skills dir.
    let claude_entry = ctx.home.join(".claude/skills/demo-skill");
    assert!(
        claude_entry.exists(),
        "claude thin-index created for demo-skill"
    );
    let codebuddy_entry = ctx.home.join(".codebuddy/skills/demo-skill");
    assert!(
        codebuddy_entry.exists(),
        "codebuddy thin-index created for demo-skill"
    );
    // Safety invariant: every planned write for a synced skill stays within
    // the injected temp home — AGS never escapes to the real $HOME.
    let home_str = ctx.home.to_string_lossy().to_string();
    let demo = result
        .items
        .iter()
        .find(|i| i.capability == "demo-skill")
        .expect("demo-skill considered");
    assert!(!demo.planned_writes.is_empty());
    for w in &demo.planned_writes {
        assert!(
            w.path.contains(&home_str),
            "planned write escaped temp home: {}",
            w.path
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
fn superpowers_migration_ctx(tag: &str) -> (ConsoleContext, PathBuf) {
    let (ctx, base) = ctx_with(tag, canned_list());
    write_file(
        &ctx.repo_root.join("global-skills/superpowers/SKILL.md"),
        "---\nname: superpowers\ndescription: parent router.\n---\n",
    );
    for playbook in ["brainstorming", "writing-plans"] {
        write_file(
            &ctx.repo_root
                .join("global-skills/superpowers/playbooks")
                .join(playbook)
                .join("PLAYBOOK.md"),
            &format!("# {playbook}\n"),
        );
    }
    write_file(
            &ctx.repo_root.join("manifests/suite.yaml"),
            "schema_version: \"1.0\"\n\
             suite:\n  name: \"test-suite\"\n  version: \"9.9.9\"\n  required:\n\
             \x20   - name: \"superpowers\"\n      version: \"1.0\"\n      source: \"global-skills/superpowers\"\n      hash: \"h1\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"superpowers-ref\"\n",
        );
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\nskills:\n\
             \x20 - name: superpowers\n\
             \x20   profile: required\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20   source:\n\
             \x20     type: bundled\n\
             \x20     path: global-skills/superpowers\n\
             route_targets:\n\
             \x20 - name: brainstorming\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: brainstorming }\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n\
             \x20 - name: writing-plans\n\
             \x20   routing:\n\
             \x20     route_state: routable\n\
             \x20     parent: { kind: skill, name: superpowers }\n\
             \x20     entrypoint: { kind: playbook, name: writing-plans }\n\
             \x20     invoke_hint: \"[skill: superpowers]\"\n",
    );
    let shared = ctx.home.join(".agents/skills");
    std::fs::create_dir_all(&shared).unwrap();
    for name in ["brainstorming", "writing-plans"] {
        std::os::unix::fs::symlink(ctx.home.join(format!("missing-{name}")), shared.join(name))
            .unwrap();
    }
    write_file(
        &shared.join("user-owned/SKILL.md"),
        "---\nname: user-owned\ndescription: preserve me.\n---\n",
    );
    (ctx, base)
}

#[cfg(unix)]
#[test]
fn sync_plan_dry_run_plans_shared_parent_and_retired_dangling_cleanup() {
    let (ctx, base) = superpowers_migration_ctx("sync-shared-dry-run");
    let result = sync_plan(&ctx, &["codex"], false);
    let json: serde_json::Value = serde_json::from_str(&render_sync_json(&result)).unwrap();
    let writes = json["shared_store_hygiene"]["planned_writes"]
        .as_array()
        .expect("shared hygiene writes");

    assert!(writes.iter().any(|write| {
        write["op"] == "relink"
            && write["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("/.agents/skills/superpowers"))
    }));
    let parent = result
        .items
        .iter()
        .find(|item| item.capability == "superpowers")
        .expect("superpowers sync item");
    assert!(parent
        .planned_writes
        .iter()
        .all(|write| !write.path.ends_with("/.codex/skills/superpowers")));
    for name in ["brainstorming", "writing-plans"] {
        assert!(writes.iter().any(|write| {
            write["op"] == "unlink"
                && write["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(&format!("/.agents/skills/{name}")))
        }));
        assert!(ctx.home.join(".agents/skills").join(name).is_symlink());
    }
    assert!(!ctx.home.join(".agents/skills/superpowers").exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_apply_migrates_shared_parent_and_only_dangling_playbooks() {
    let (ctx, base) = superpowers_migration_ctx("sync-shared-apply");
    let shared = ctx.home.join(".agents/skills");
    link_skill_entry(
        &ctx,
        ".codex/skills",
        "superpowers",
        "global-skills/superpowers",
    );

    let result = sync_plan(&ctx, &["codex"], true);
    let json: serde_json::Value = serde_json::from_str(&render_sync_json(&result)).unwrap();

    assert_eq!(json["shared_store_hygiene"]["apply_status"], "applied");
    assert!(shared.join("superpowers/SKILL.md").is_file());
    assert!(!ctx.home.join(".codex/skills/superpowers").is_symlink());
    assert!(!shared.join("brainstorming").is_symlink());
    assert!(!shared.join("writing-plans").is_symlink());
    assert!(
        !ctx.home.join(".codex/skills/superpowers.bak").exists()
            && std::fs::symlink_metadata(ctx.home.join(".codex/skills/superpowers.bak")).is_err(),
        "successful unlink must not leave a Codex backup entry"
    );
    for name in ["brainstorming", "writing-plans"] {
        let backup = shared.join(format!("{name}.bak"));
        assert!(
            !backup.exists() && std::fs::symlink_metadata(&backup).is_err(),
            "successful unlink must not leave shared backup drift: {}",
            backup.display()
        );
    }
    assert!(shared.join("user-owned/SKILL.md").is_file());

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_skips_codex_thin_index_when_shared_agents_source_exists() {
    let (ctx, base) = ctx_with("syncsharedcodex", canned_list());
    link_skill_entry(
        &ctx,
        ".agents/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );

    let result = sync_plan(&ctx, &["codex"], false);
    let demo = result
        .items
        .iter()
        .find(|i| i.capability == "demo-skill")
        .expect("demo-skill considered");

    assert!(
            demo.planned_writes
                .iter()
                .all(|w| !w.path.contains(".codex/skills/demo-skill")),
            "sync must not create a duplicate Codex thin-index when .agents already exposes the skill: {:?}",
            demo.planned_writes
        );
    assert!(
        demo.note.contains("shared skill source already visible"),
        "operator note should explain why Codex was skipped: {}",
        demo.note
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_omp_shared_entry_requires_exact_canonical_identity() {
    let (ctx, base) = ctx_with("sync-shared-omp-identity", canned_list());
    link_skill_entry(
        &ctx,
        ".agents/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    let exact = sync_plan(&ctx, &["omp"], false);
    let demo = exact
        .items
        .iter()
        .find(|item| item.capability == "demo-skill")
        .expect("demo-skill considered");
    assert!(demo.blocked_reasons.is_empty(), "{demo:?}");
    assert!(demo
        .planned_writes
        .iter()
        .all(|write| !write.path.contains(".omp/agent/skills/demo-skill")));

    link_skill_entry(
        &ctx,
        ".omp/agent/skills",
        "demo-skill",
        "global-skills/demo-skill",
    );
    let duplicate_inventory = build_inventory(&ctx, &["omp"]);
    assert_eq!(
        find(&duplicate_inventory, "demo-skill").host_visibility[0].status,
        HostVisibilityStatus::Degraded,
        "OMP verify must reject duplicate native and shared picker entries"
    );
    let duplicate_sync = sync_plan(&ctx, &["omp"], false);
    let demo = duplicate_sync
        .items
        .iter()
        .find(|item| item.capability == "demo-skill")
        .expect("demo-skill considered");
    assert!(demo.planned_writes.iter().any(|write| {
        write.op == "unlink" && write.path.contains(".omp/agent/skills/demo-skill")
    }));

    std::fs::remove_file(ctx.home.join(".agents/skills/demo-skill")).unwrap();
    write_file(
        &ctx.home.join(".agents/skills/demo-skill/SKILL.md"),
        "---\nname: demo-skill\ndescription: unrelated shared body.\n---\n",
    );
    let conflict = sync_plan(&ctx, &["omp"], false);
    let demo = conflict
        .items
        .iter()
        .find(|item| item.capability == "demo-skill")
        .expect("demo-skill considered");
    assert!(demo
        .blocked_reasons
        .iter()
        .any(|reason| reason.contains("does not resolve to the canonical body")));
    assert!(demo
        .planned_writes
        .iter()
        .all(|write| !write.path.contains(".omp/agent/skills/demo-skill")));
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_skips_codex_thin_index_when_plugin_source_exists() {
    let (ctx, base) = ctx_with("syncplugincodex", canned_list());
    write_codex_plugin_skill(&ctx, "demo-skill");

    let result = sync_plan(&ctx, &["codex"], false);
    let demo = result
        .items
        .iter()
        .find(|i| i.capability == "demo-skill")
        .expect("demo-skill considered");

    assert!(
            demo.planned_writes
                .iter()
                .all(|w| !w.path.contains(".codex/skills/demo-skill")),
            "sync must not create a duplicate Codex thin-index when a plugin already exposes the skill: {:?}",
            demo.planned_writes
        );
    assert!(
        demo.note.contains(".codex/plugins/cache"),
        "operator note should explain the plugin source: {}",
        demo.note
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_repairs_codex_thin_index_when_plugin_is_disabled() {
    let (ctx, base) = ctx_with("syncdisabledplugincodex", canned_list());
    write_codex_plugin_skill_with_enabled(&ctx, "demo-skill", false);

    let result = sync_plan(&ctx, &["codex"], false);
    let demo = result
        .items
        .iter()
        .find(|i| i.capability == "demo-skill")
        .expect("demo-skill considered");

    assert!(
        demo.planned_writes
            .iter()
            .any(|w| w.path.contains(".codex/skills/demo-skill")),
        "a disabled plugin cache must not suppress the canonical Codex thin-index: {:?}",
        demo.planned_writes
    );
    assert!(
        !demo.note.contains(".codex/plugins/cache"),
        "disabled plugin cache must not be reported as runtime-visible: {}",
        demo.note
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_apply_replaces_existing_thin_index_without_backup() {
    let (ctx, base) = ctx_with("syncnobak", canned_list());
    let entry = ctx.home.join(".claude/skills/demo-skill");
    write_file(&entry.join("SKILL.md"), "OLD CONTENT");

    let result = sync_plan(&ctx, &["claude-code"], true);

    assert!(result.apply_requested);
    assert_eq!(result.summary.failed, 0);
    assert!(
        std::fs::symlink_metadata(&entry)
            .unwrap()
            .file_type()
            .is_symlink(),
        "sync apply should replace the existing host entry with a thin-index symlink"
    );
    assert!(
        std::fs::read_to_string(entry.join("SKILL.md"))
            .unwrap()
            .contains("name: demo-skill"),
        "active entry should resolve to the canonical skill body"
    );
    assert!(
        !ctx.home.join(".claude/skills/demo-skill.bak").exists(),
        "capability sync must not leave .bak backups"
    );
    assert!(
        !ctx.home.join(".claude/skills/demo-skill.bak.1").exists(),
        "capability sync must not leave numbered .bak backups"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// Regression (adversarial review): a capability marked `route_state: retired`
/// must never be (re)adopted/synced into a host, even though its canonical
/// body is still on disk — this closes the resurrection path
/// (`ags capability install --capability <retired>`).
#[test]
fn retired_capability_is_blocked_from_adoption_and_sync() {
    let base = std::env::temp_dir().join(format!("ags-retired-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let home = base.join("home");

    // A suite-required skill whose routing is `retired` (a deliberately
    // constructed edge case: even a still-required retired skill must be
    // gated) plus a normal required skill as a control.
    write_file(
            &repo.join("manifests/suite.yaml"),
            "schema_version: \"1.0\"\n\
             suite:\n  name: \"t\"\n  version: \"9.9.9\"\n  required:\n\
             \x20   - name: \"retired-demo\"\n      version: \"1.0\"\n      source: \"global-skills/retired-demo\"\n      hash: \"h\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"retired-demo-ref\"\n\
             \x20   - name: \"demo-skill\"\n      version: \"1.0\"\n      source: \"global-skills/demo-skill\"\n      hash: \"h\"\n      adopted: \"2026-01-01T00:00:00Z\"\n      entry_ref: \"demo-skill-ref\"\n",
        );
    write_file(
        &repo.join("global-skills/retired-demo/SKILL.md"),
        "---\nname: retired-demo\ndescription: retired body still on disk.\n---\nbody\n",
    );
    write_file(
        &repo.join("global-skills/demo-skill/SKILL.md"),
        "---\nname: demo-skill\ndescription: control.\n---\nbody\n",
    );
    write_file(
            &repo.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: retired-demo\n    routing:\n      route_state: retired\n      capability_group: [ags-governance-ops]\n\
             \x20 - name: demo-skill\n    routing:\n      route_state: not-routable\n",
        );
    write_file(&repo.join("manifests/mcp-registry.yaml"), "mcps: []\n");

    let ctx = ConsoleContext::new(
        repo,
        home,
        Box::new(StrictMcpRunner {
            claude: canned_list(),
            codex: canned_codex_list(),
        }),
    );

    // Adopt / Update / Repair of the retired skill are blocked with NO writes.
    for action in [
        ConsoleAction::Adopt,
        ConsoleAction::Update,
        ConsoleAction::Repair,
    ] {
        let res = propose_action(&ctx, action, "retired-demo", false);
        assert!(res.found, "retired body is still discovered: {action:?}");
        assert!(
            !res.blocked_reasons.is_empty(),
            "retired skill must be blocked for {action:?}"
        );
        assert!(
            res.planned_writes.is_empty(),
            "retired skill must plan no host writes for {action:?}"
        );
        assert!(
            res.blocked_reasons.iter().any(|r| r.contains("retired")),
            "block reason must name retirement: {:?}",
            res.blocked_reasons
        );
    }

    // Sync never considers the retired skill; the control IS synced.
    let sync = sync_plan(&ctx, &["claude-code", "codex"], false);
    let names: Vec<&str> = sync.items.iter().map(|i| i.capability.as_str()).collect();
    assert!(
        !names.contains(&"retired-demo"),
        "retired skill must be excluded from sync: {names:?}"
    );
    assert!(
        names.contains(&"demo-skill"),
        "non-retired required skill still syncs: {names:?}"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn sync_plan_removes_only_proven_retired_suite_thin_indexes() {
    let (ctx, base) = ctx_with_repo_dir(
        "retired-thin-index-cleanup",
        canned_list(),
        "example-private-suite",
    );
    write_file(
        &ctx.repo_root.join("manifests/skills-registry.yaml"),
        "schema_version: \"1.0\"\nskills:\n\
             \x20 - name: retired-safe\n    routing: { route_state: retired }\n\
             \x20 - name: retired-real\n    routing: { route_state: retired }\n\
             \x20 - name: retired-external\n    routing: { route_state: retired }\n\
             \x20 - name: retired-mismatch\n    routing: { route_state: retired }\n",
    );

    let stable = base.join("example-stable-suite");
    let runtime = base.join("agent-governance-suite-runtime");
    write_file(
        &stable.join("global-skills/retired-safe/SKILL.md"),
        "---\nname: retired-safe\ndescription: old stable body.\n---\n",
    );
    write_file(
        &runtime.join("global-skills/retired-safe/SKILL.md"),
        "---\nname: retired-safe\ndescription: old runtime body.\n---\n",
    );

    let claude_safe = ctx.home.join(".claude/skills/retired-safe");
    let codex_safe = ctx.home.join(".codex/skills/retired-safe");
    let shared_dangling = ctx.home.join(".agents/skills/retired-safe");
    std::fs::create_dir_all(claude_safe.parent().unwrap()).unwrap();
    std::fs::create_dir_all(codex_safe.parent().unwrap()).unwrap();
    std::fs::create_dir_all(shared_dangling.parent().unwrap()).unwrap();
    make_symlink(&stable.join("global-skills/retired-safe"), &claude_safe).unwrap();
    make_symlink(&runtime.join("global-skills/retired-safe"), &codex_safe).unwrap();
    make_symlink(
        &ctx.repo_root.join("global-skills/retired-safe"),
        &shared_dangling,
    )
    .unwrap();

    let real_entry = ctx.home.join(".codebuddy/skills/retired-real");
    write_file(
        &real_entry.join("SKILL.md"),
        "---\nname: retired-real\ndescription: user-owned.\n---\n",
    );
    let outside = base.join("external/retired-external");
    write_file(
        &outside.join("SKILL.md"),
        "---\nname: retired-external\ndescription: external.\n---\n",
    );
    let external_entry = ctx.home.join(".codebuddy/skills/retired-external");
    make_symlink(&outside, &external_entry).unwrap();
    let mismatch_entry = ctx.home.join(".codebuddy/skills/retired-mismatch");
    make_symlink(&stable.join("global-skills/retired-safe"), &mismatch_entry).unwrap();

    let dry_run = sync_plan(&ctx, &["claude-code", "codex", "codebuddy-code"], false);
    let retired_writes: Vec<&PlannedWrite> = dry_run
        .shared_store_hygiene
        .planned_writes
        .iter()
        .filter(|write| write.detail.contains("retired suite thin index"))
        .collect();
    assert_eq!(
        retired_writes.len(),
        3,
        "only the three proven links plan cleanup"
    );
    assert!(retired_writes.iter().all(|write| {
        write.op == "unlink-retired-suite-thin-index" && write.path.ends_with("/retired-safe")
    }));
    for entry in [&claude_safe, &codex_safe, &shared_dangling] {
        assert!(
            std::fs::symlink_metadata(entry).is_ok(),
            "dry-run preserves {entry:?}"
        );
    }
    let forged_cleanup = guarded_apply(
        true,
        &[PlannedWrite {
            op: "unlink-retired-suite-thin-index".to_string(),
            path: real_entry.display().to_string(),
            from: None,
            detail: "forged cleanup request".to_string(),
        }],
        &ctx,
    );
    assert!(
        !forged_cleanup.errors.is_empty(),
        "the mutation guard must independently reject a real directory"
    );
    assert!(
        real_entry.is_dir(),
        "guard rejection preserves the real directory"
    );
    let active_entry = ctx.home.join(".codebuddy/skills/demo-skill");
    make_symlink(
        &ctx.repo_root.join("global-skills/demo-skill"),
        &active_entry,
    )
    .unwrap();
    let forged_active_cleanup = guarded_apply(
        true,
        &[PlannedWrite {
            op: "unlink-retired-suite-thin-index".to_string(),
            path: active_entry.display().to_string(),
            from: None,
            detail: "forged cleanup request for active suite skill".to_string(),
        }],
        &ctx,
    );
    assert!(
        !forged_active_cleanup.errors.is_empty(),
        "the mutation guard must independently require registry retirement"
    );
    assert!(
        active_entry.is_symlink(),
        "active suite links are never retired"
    );

    let applied = sync_plan(&ctx, &["claude-code", "codex", "codebuddy-code"], true);
    assert_eq!(applied.shared_store_hygiene.apply_status, "applied");
    for entry in [&claude_safe, &codex_safe, &shared_dangling] {
        assert!(
            std::fs::symlink_metadata(entry).is_err(),
            "proven retired AGS thin index should be removed: {entry:?}"
        );
    }
    assert!(real_entry.is_dir(), "real directories are never removed");
    assert!(
        std::fs::symlink_metadata(&external_entry)
            .unwrap()
            .file_type()
            .is_symlink(),
        "external symlinks are never removed"
    );
    assert!(
        std::fs::symlink_metadata(&mismatch_entry)
            .unwrap()
            .file_type()
            .is_symlink(),
        "suite links whose target name does not match are never removed"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn mcp_adopt_advises_both_claude_and_codex_hosts() {
    let (ctx, base) = ctx_with("mcphosts", canned_list());
    let res = propose_action(&ctx, ConsoleAction::Adopt, "context7", false);
    assert!(res.found);
    let cmds: Vec<&str> = res
        .advised_commands
        .iter()
        .map(|c| c.command.as_str())
        .collect();
    assert!(
        cmds.iter()
            .any(|c| c.starts_with("claude mcp add context7")),
        "{cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| c.starts_with("codex mcp add context7")),
        "{cmds:?}"
    );
    // Still advise-only — AGS never registers MCP servers itself.
    assert!(res.planned_writes.is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

/// Full-machine discovery classifies host-dir skills into the taxonomy and is
/// fail-closed: every discovered host-dir skill is `routing: None`
/// (not-routable) until adopted. Also exercises symlink safety — dangling
/// links, external targets, and symlink loops are recognized without panic
/// and never linked into the AGS store.
#[cfg(unix)]
#[test]
fn discovers_host_dir_system_user_and_unmanaged_skills_fail_closed() {
    let (ctx, base) = ctx_with("hostdir-discovery", canned_list());
    let skills = ctx.home.join(".codex/skills");
    // Real-dir user skill → discovered-local.
    write_file(
        &skills.join("myuserskill/SKILL.md"),
        "---\nname: myuserskill\ndescription: x.\n---\nbody\n",
    );
    // System skill under `.system` → host-system.
    write_file(
        &skills.join(".system/sys-creator/SKILL.md"),
        "---\nname: sys-creator\ndescription: x.\n---\nbody\n",
    );
    // Dangling symlink → unmanaged (no panic).
    std::os::unix::fs::symlink(base.join("does-not-exist-xyz"), skills.join("broken")).unwrap();
    // Symlink to an external location outside any store → unmanaged.
    let external = base.join("external/extskill");
    write_file(
        &external.join("SKILL.md"),
        "---\nname: extskill\ndescription: x.\n---\nbody\n",
    );
    std::os::unix::fs::symlink(&external, skills.join("extskill")).unwrap();
    // Symlink loop → must not panic.
    std::os::unix::fs::symlink(skills.join("loopb"), skills.join("loopa")).unwrap();
    std::os::unix::fs::symlink(skills.join("loopa"), skills.join("loopb")).unwrap();

    let inv = build_inventory(&ctx, &["codex"]);
    let by = |n: &str| {
        inv.capabilities
            .iter()
            .find(|c| c.name == n)
            .cloned()
            .unwrap_or_else(|| panic!("capability {n} not discovered"))
    };

    assert_eq!(by("myuserskill").managed_status, ManagedStatus::Discovered);
    assert_eq!(by("sys-creator").managed_status, ManagedStatus::HostSystem);
    assert_eq!(by("extskill").managed_status, ManagedStatus::Unmanaged);
    assert_eq!(by("broken").managed_status, ManagedStatus::Unmanaged);
    // Fail-closed: NONE of the discovered host-dir skills are routable.
    for n in ["myuserskill", "sys-creator", "extskill", "broken"] {
        assert!(
            by(n).routing.is_none(),
            "{n} must be fail-closed not-routable until adopted"
        );
        assert_eq!(by(n).registry_status, RegistryStatus::NotRegistered);
    }
    // The system skill is canonical-present (its body exists) but never
    // copied — its source is the external `.system` path, not the AGS store.
    assert!(by("sys-creator").canonical_present);
    assert!(by("sys-creator").source.unwrap().contains(".system"));
    // Public boundary: the snapshot hash (a recordable attestation token)
    // embeds capability NAMES + statuses only — never an absolute machine
    // path or a system-skill body — so it is safe to publish / record.
    let hash = inventory_snapshot_hash(&inv);
    assert!(hash.starts_with("fnv1a64:"));
    assert!(!hash.contains('/') && !hash.contains("Users") && !hash.contains(".system"));
    let _ = std::fs::remove_dir_all(&base);
}

/// A discovered host-system `.system/<name>` whose SKILL.md front-matter
/// declares a DIFFERENT name must read Degraded (not Visible): a mismatched
/// or replaced body cannot masquerade as the adopted capability for the
/// runtime skill-tag gate. A matching body reads Visible. (adversarial-review
/// hardening — host-dir visibility now validates SKILL.md identity like
/// `skill_path_visibility`.)
#[test]
fn host_dir_skill_visibility_validates_front_matter_identity() {
    let (ctx, base) = ctx_with("hostdir-frontmatter", canned_list());
    let skills = ctx.home.join(".codex/skills");
    let codex_vis = |inv: &ManagedInventoryResult| -> HostVisibilityStatus {
        inv.capabilities
            .iter()
            .find(|c| c.name == "skill-creator")
            .and_then(|c| c.host_visibility.iter().find(|v| v.host == "codex"))
            .map(|v| v.status.clone())
            .expect("skill-creator codex visibility")
    };

    // Directory named `skill-creator` but the body declares another name.
    write_file(
        &skills.join(".system/skill-creator/SKILL.md"),
        "---\nname: not-skill-creator\ndescription: impostor.\n---\nbody\n",
    );
    assert_eq!(
        codex_vis(&build_inventory(&ctx, &["codex"])),
        HostVisibilityStatus::Degraded,
        "mismatched SKILL.md front-matter name must be Degraded, not Visible"
    );

    // A body whose front-matter name matches the directory reads Visible.
    write_file(
        &skills.join(".system/skill-creator/SKILL.md"),
        "---\nname: skill-creator\ndescription: real.\n---\nbody\n",
    );
    assert_eq!(
        codex_vis(&build_inventory(&ctx, &["codex"])),
        HostVisibilityStatus::Visible,
        "matching SKILL.md front-matter name must be Visible"
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// If a host-system skill is explicitly adopted in skills-registry.yaml, the
/// inventory row must reflect that registry authority consistently: routable
/// AND registered. It remains host-system/read-only; only the routing
/// authority changes.
#[test]
fn adopted_host_system_skill_is_registered_in_inventory() {
    let (ctx, base) = ctx_with("hostdir-adopted-registered", canned_list());
    let skills = ctx.home.join(".codex/skills");
    write_file(
        &skills.join(".system/skill-creator/SKILL.md"),
        "---\nname: skill-creator\ndescription: real.\n---\nbody\n",
    );
    write_file(
            &ctx.repo_root.join("manifests/skills-registry.yaml"),
            "skills:\n\
             \x20 - name: skill-creator\n    routing:\n      route_state: routable\n      intent_tags: [skill-authoring]\n      invoke_hint: \"[skill: skill-creator]\"\n",
        );

    let inv = build_inventory(&ctx, &["codex"]);
    let cap = find(&inv, "skill-creator");
    assert_eq!(cap.managed_status, ManagedStatus::HostSystem);
    assert_eq!(cap.registry_status, RegistryStatus::Registered);
    assert_eq!(
        cap.routing.as_ref().map(|r| r.route_state),
        Some(RouteState::Routable)
    );
    assert!(cap
        .risk_notes
        .iter()
        .any(|note| note.contains("registry-adopted for routing")));
    assert!(cap
        .risk_notes
        .iter()
        .all(|note| !note.contains("Adopt via the registry to make it routable")));

    let _ = std::fs::remove_dir_all(&base);
}
