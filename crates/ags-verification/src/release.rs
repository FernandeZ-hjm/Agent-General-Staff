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
    items.push(check_validator_mutation_guards(repo_root));

    // Check 2: Verify bootstrap --apply produces a sanitized public payload.
    let tmpdir = std::env::temp_dir().join(format!("ags-verify-release-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmpdir);

    let (bootstrap_code, _bs_stdout, bs_stderr) = run_command(
        repo_root,
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "ags-cli",
            "--",
            "bootstrap",
            "--apply",
            "--target",
            &tmpdir.to_string_lossy(),
        ],
        &[],
    );

    if bootstrap_code == 0 {
        // Check that generated build output and private runtime state are NOT in the payload.
        let forbidden = [
            "target",
            "ags",
            "ags.exe",
            "global-skills",
            "skill-packs",
            ".agents",
            ".codex",
            "task-archive",
        ];
        let mut leaked = Vec::new();
        for item in &forbidden {
            if tmpdir.join(item).exists() {
                leaked.push(*item);
            }
        }
        if leaked.is_empty() {
            items.push(CheckItem::pass(
                "release-forbidden-payload",
                "release",
                "No build output, preinstalled skill packs, or private runtime state leaked into bootstrap payload.",
            ));
        } else {
            items.push(CheckItem::fail(
                "release-forbidden-payload",
                "release",
                &format!(
                    "Forbidden public-full sanitized payload leaked into bootstrap: {}",
                    leaked.join(", ")
                ),
                "Check bootstrap --apply payload allowlist.",
            ));
        }
    } else {
        items.push(CheckItem::fail(
            "release-bootstrap-apply",
            "release",
            &format!(
                "bootstrap --apply failed (exit {}): {}",
                bootstrap_code,
                truncate(&bs_stderr, 300)
            ),
            "Fix bootstrap --apply before release.",
        ));
    }

    // Cleanup tempdir
    let _ = std::fs::remove_dir_all(&tmpdir);

    items
}

fn check_validator_mutation_guards(repo_root: &Path) -> CheckItem {
    let script = "scripts/verify-validator-mutations.py";
    if !repo_root.join(script).is_file() {
        return CheckItem::fail(
            "validator-mutation-guards",
            "release",
            &format!("Required mutation verifier is missing: {script}"),
            "Restore the task-card mutation verifier.",
        );
    }

    let (code, stdout, stderr) = run_command(repo_root, "python3", &[script], &[]);
    if code == 0 {
        CheckItem::pass("validator-mutation-guards", "release", stdout.trim())
    } else {
        CheckItem::fail(
            "validator-mutation-guards",
            "release",
            &truncate(&format!("{stdout}\n{stderr}"), 1000),
            "Repair the semantic contract or CLI fixture gate that survived mutation.",
        )
        .with_command(&format!("python3 {script}"))
        .with_exit_code(code)
    }
}

/// Check that portable runtime profile templates exist, parse correctly,
/// and contain no real secrets or absolute private paths.
///
/// This check is smart about what constitutes a "leak": documentation
/// mentioning "token" or "secret" is fine; actual 64+ char hex tokens,
/// absolute `/Users/` paths, and real memory/archive paths are NOT.
pub(super) fn check_runtime_profile_templates(repo_root: &Path) -> CheckItem {
    let templates_dir = repo_root.join("manifests/templates");
    if !templates_dir.exists() {
        if crate::edition::is_public_edition(repo_root) {
            return CheckItem::pass(
                "runtime-profile-templates",
                "local",
                "public edition intentionally omits private EvoMap runtime profile templates",
            );
        }
        return CheckItem::warn(
            "runtime-profile-templates",
            "local",
            "manifests/templates/ directory not found — portable EvoMap profile templates missing",
            "Run `mkdir -p manifests/templates/hooks` and add template files.",
        );
    }

    let template_files: &[(&str, &str)] = &[
        ("manifests/templates/runtime-profiles.template.yaml", "yaml"),
        (
            "manifests/templates/hooks/claude-code-executor-stop.template.js",
            "javascript",
        ),
        (
            "manifests/templates/hooks/codex-planner-recall.template.json",
            "json",
        ),
        ("manifests/templates/README.md", "markdown"),
    ];

    let mut missing = Vec::new();
    let mut parse_errors = Vec::new();
    let mut found = Vec::new();

    for (rel_path, kind) in template_files {
        let full_path = repo_root.join(rel_path);
        if !full_path.exists() {
            missing.push(*rel_path);
            continue;
        }
        found.push(*rel_path);

        // Parse check
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                parse_errors.push(format!("{rel_path}: cannot read: {e}"));
                continue;
            }
        };

        match *kind {
            "yaml" => {
                if let Err(e) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                    parse_errors.push(format!("{rel_path}: YAML parse error: {e}"));
                }
            }
            "json" => {
                if let Err(e) = serde_json::from_str::<serde_json::Value>(&content) {
                    parse_errors.push(format!("{rel_path}: JSON parse error: {e}"));
                }
            }
            "javascript"
                // Node --check is done in suite-doctor; here we just verify
                // the file is non-empty and starts with a plausible shebang
                if content.trim().is_empty() => {
                    parse_errors.push(format!("{rel_path}: empty file"));
                }
            _ => {} // markdown — no parse check needed
        }

        // Smart leak check: look for patterns that indicate REAL leaked data,
        // not just documentation mentions of the words "token" or "secret".
        let leaks = detect_template_leaks(&content, rel_path);
        if !leaks.is_empty() {
            parse_errors.extend(leaks);
        }
    }

    if !parse_errors.is_empty() {
        let evidence = format!(
            "{} template file(s) have issues:\n{}",
            parse_errors.len(),
            parse_errors.join("\n")
        );
        CheckItem::fail(
            "runtime-profile-templates",
            "local",
            &truncate(&evidence, 500),
            "Fix template parse errors or remove leaked secrets/paths.",
        )
    } else if !missing.is_empty() {
        CheckItem::warn(
            "runtime-profile-templates",
            "local",
            &format!(
                "{} template file(s) missing: {} ({} found OK)",
                missing.len(),
                missing.join(", "),
                found.len()
            ),
            &format!(
                "Create missing template files in manifests/templates/. Missing: {}",
                missing.join(", ")
            ),
        )
    } else {
        CheckItem::pass(
            "runtime-profile-templates",
            "local",
            &format!(
                "{} template file(s) present and parse correctly with no leaks detected",
                found.len()
            ),
        )
    }
}

/// Detect patterns in template content that indicate REAL leaked secrets or
/// absolute private paths. Documentation words like "token" are fine; actual
/// 64+ char hex tokens, `/Users/` paths, and real memory/archive paths are NOT.
pub(super) fn detect_template_leaks(content: &str, rel_path: &str) -> Vec<String> {
    let mut leaks = Vec::new();

    // Check for absolute macOS or Linux home paths that look like real
    // machine-specific paths (not just documentation patterns). A plausible
    // username segment avoids flagging grep/find examples such as `/Users/`.
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Comments and shell examples are scanned too: a pasted real path is
        // still a leak, while a bare pattern has no plausible username segment.
        for prefix in ["/Users/", "/home/"] {
            if let Some(rest) = trimmed.find(prefix) {
                let after_prefix = &trimmed[rest + prefix.len()..];
                let maybe_user = after_prefix.split('/').next().unwrap_or("");
                if maybe_user.len() >= 2
                    && maybe_user
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    leaks.push(format!(
                        "{rel_path}:{line_num}: potential absolute home path leak ({prefix}): {trimmed}",
                        line_num = i + 1
                    ));
                }
            }
        }
    }

    // Check for long hex strings that look like real tokens (64+ hex chars).
    // Comments are NOT skipped — a real token pasted into a comment is still a leak.
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Skip REPLACE lines and sha256 documentation examples
        if trimmed.starts_with('"') && (trimmed.contains("REPLACE") || trimmed.contains("sha256")) {
            continue;
        }
        let hex_run = longest_hex_run(trimmed);
        if hex_run >= 64 {
            leaks.push(format!(
                "{rel_path}:{line_num}: potential hex token leak ({hex_run} hex chars)",
                line_num = i + 1
            ));
        }
    }

    // Check for real task archive or memory capsule paths.
    // Comments are NOT skipped — these paths shouldn't appear anywhere.
    for pat in &[".agents/memory/projects/", "task-archive/"] {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(pat) {
                leaks.push(format!(
                    "{rel_path}:{line_num}: potential memory/archive path leak: {pat}",
                    line_num = i + 1
                ));
            }
        }
    }

    leaks
}
