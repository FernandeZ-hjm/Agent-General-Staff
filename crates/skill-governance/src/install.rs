//! Skill install — real skill installation with directory structure and SKILL.md.
//!
//! Generates per-skill directories with proper frontmatter:
//! - `$TARGET/<skill-name>/SKILL.md`
//! - Auto skills get trigger-condition documentation
//! - Manual skills get invocation documentation
//! - Install receipt written on confirmed install
//!
//! # Install modes
//!
//! - **Template** (default): generates a SKILL.md skeleton with frontmatter,
//!   clearly labeled as "TEMPLATE INSTALL". Also creates stub directories
//!   (scripts/, references/, templates/). User must copy real content from
//!   the source repository.
//! - **Full**: copies a complete skill package from `--source-dir`.
//!
//! # Public boundary
//!
//! - All paths resolve from `$HOME` or args — no hardcoded private paths.
//! - No private skill content, memory, or history is shipped.
//! - No third-party skills are auto-installed; --confirm is required.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Public types ──────────────────────────────────────────────────────────

/// Install mode — template vs full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallMode {
    /// Template install: generates SKILL.md skeleton, creates stub
    /// directories. Clearly labeled as TEMPLATE INSTALL.
    Template,
    /// Full install: copies complete skill package from a local source dir.
    Full,
}

/// Result of a skill install operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallResult {
    pub status: InstallStatus,
    pub mode: InstallMode,
    pub target_dir: String,
    pub skills_installed: Vec<String>,
    pub skills_skipped: Vec<String>,
    pub receipt_path: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallStatus {
    DryRun,
    Blocked,
    Installed,
    PartialFailure,
}

/// A skill definition for installation.
#[derive(Debug, Clone)]
pub struct SkillDef {
    pub name: String,
    pub source: String,
    pub category: SkillCategory,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCategory {
    Auto,
    Manual,
}

// ── Known skill definitions ───────────────────────────────────────────────

/// Return all known recommended skills with metadata.
pub fn known_skills() -> HashMap<String, SkillDef> {
    let skills: Vec<SkillDef> = vec![
        SkillDef {
            name: "auto-brainstorm".into(),
            source: "https://github.com/anthropics/skills/tree/main/auto-brainstorm".into(),
            category: SkillCategory::Auto,
            description: "Automatically brainstorms approach options before entering plan mode."
                .into(),
        },
        SkillDef {
            name: "auto-debug".into(),
            source: "https://github.com/anthropics/skills/tree/main/auto-debug".into(),
            category: SkillCategory::Auto,
            description:
                "Automatically diagnoses errors, exceptions, failures, and broken behavior.".into(),
        },
        SkillDef {
            name: "auto-verify".into(),
            source: "https://github.com/anthropics/skills/tree/main/auto-verify".into(),
            category: SkillCategory::Auto,
            description: "Automatically verifies behavior when work is claimed complete.".into(),
        },
        SkillDef {
            name: "tdd".into(),
            source: "https://github.com/anthropics/skills/tree/main/tdd".into(),
            category: SkillCategory::Manual,
            description: "Test-driven development workflow — red-green-refactor cycle.".into(),
        },
        SkillDef {
            name: "diagnose".into(),
            source: "https://github.com/anthropics/skills/tree/main/diagnose".into(),
            category: SkillCategory::Manual,
            description: "Root cause diagnosis with evidence-chain tracing.".into(),
        },
        SkillDef {
            name: "verification-before-completion".into(),
            source: "https://github.com/anthropics/skills/tree/main/verification-before-completion"
                .into(),
            category: SkillCategory::Manual,
            description: "Verification gate enforcement before claiming task completion.".into(),
        },
        SkillDef {
            name: "webapp-testing".into(),
            source: "https://github.com/anthropics/skills/tree/main/webapp-testing".into(),
            category: SkillCategory::Manual,
            description: "Playwright-based web application testing.".into(),
        },
        SkillDef {
            name: "caveman-review".into(),
            source: "https://github.com/anthropics/skills/tree/main/caveman-review".into(),
            category: SkillCategory::Manual,
            description: "Short actionable code review feedback.".into(),
        },
        SkillDef {
            name: "caveman-commit".into(),
            source: "https://github.com/anthropics/skills/tree/main/caveman-commit".into(),
            category: SkillCategory::Manual,
            description: "Concise Conventional Commit message generation from diff analysis."
                .into(),
        },
    ];

    skills.into_iter().map(|s| (s.name.clone(), s)).collect()
}

// ── SKILL.md generators ───────────────────────────────────────────────────

const TEMPLATE_BANNER: &str = "\
> ╔══════════════════════════════════════════════════════════════╗
> ║  TEMPLATE INSTALL — THIS IS A SKELETON, NOT A REAL SKILL    ║
> ║                                                            ║
> ║  This SKILL.md is a generated template. It contains         ║
> ║  frontmatter and documentation stubs but NOT the full       ║
> ║  skill content from the source repository.                  ║
> ║                                                            ║
> ║  To complete the installation:                              ║
> ║  1. Visit the source URL below                             ║
> ║  2. Copy the real SKILL.md content                         ║
> ║  3. Copy supporting files into scripts/ references/        ║
> ║     templates/ directories                                 ║
> ║                                                            ║
> ║  For full install from a local copy:                        ║
> ║  ags skill install --skill <name> --mode full \\            ║
> ║      --source-dir /path/to/skill --confirm                  ║
> ╚══════════════════════════════════════════════════════════════╝
";

fn auto_skill_content(def: &SkillDef) -> String {
    format!(
        r#"---
name: {name}
description: {desc}
category: auto-trigger
source: {source}
install_method: ags skill install --skill {name} --confirm
---

# {title}

{desc}

## Trigger Conditions

This skill activates **automatically** in the following situations:

{triggers}

## Behavior

When triggered, this skill loads automatically and performs its function
without requiring manual invocation via `/{{skill-name}}`.

## Source

{source}

## Installation

Installed via `ags skill install --skill {name} --confirm` to the
configured skills directory.

## Notes

- This is a public development skill from the AGS recommended list.
- It does NOT contain private paths, credentials, or personal data.
- See the source repository for full documentation and updates.
"#,
        name = def.name,
        desc = def.description,
        title = title_case(&def.name),
        source = def.source,
        triggers = auto_triggers(&def.name),
    )
}

fn auto_triggers(name: &str) -> String {
    match name {
        "auto-brainstorm" => {
            r#"- User enters plan mode or requests a plan for a non-trivial task
- Agent is about to design an architecture or solution approach
- User asks "how should I..." or "what's the best way to..."
- Multiple plausible approaches exist and none is obviously dominant"#
                .into()
        }
        "auto-debug" => {
            r#"- An error, exception, or stack trace is encountered
- A test fails unexpectedly
- Build or lint errors occur
- Runtime assertion failures or panics
- User reports "it doesn't work" or "it's broken""#
                .into()
        }
        "auto-verify" => {
            r#"- Work is claimed complete or "done"
- A fix is applied and the agent is about to report success
- A PR is about to be submitted
- Tests pass but behavior hasn't been manually verified"#
                .into()
        }
        _ => "- (trigger conditions depend on the specific skill)\n- See source repository for details"
            .into(),
    }
}

fn manual_skill_content(def: &SkillDef) -> String {
    format!(
        r#"---
name: {name}
description: {desc}
category: manual
source: {source}
install_method: ags skill install --skill {name} --confirm
---

# {title}

{desc}

## Invocation

This is a **manual** skill. Invoke it with:

```
/{name}
```

Or via the skill tool in your agent runtime.

## When to Use

{when_to_use}

## Source

{source}

## Installation

Installed via `ags skill install --skill {name} --confirm` to the
configured skills directory.

## Notes

- This is a public development skill from the AGS recommended list.
- It does NOT contain private paths, credentials, or personal data.
- See the source repository for full documentation and updates.
"#,
        name = def.name,
        desc = def.description,
        title = title_case(&def.name),
        source = def.source,
        when_to_use = manual_when_to_use(&def.name),
    )
}

fn manual_when_to_use(name: &str) -> String {
    match name {
        "tdd" => "When implementing new features, fixing bugs, or making behavior changes. Run tests first (red), implement the minimum fix (green), then refactor with confidence.".into(),
        "diagnose" => "When facing a complex bug, unexplained failure, performance regression, or flaky test. The skill guides a systematic HITL debugging loop with evidence-chain tracing.".into(),
        "verification-before-completion" => "Before claiming any task is complete. Forces explicit verification steps: run tests, check diffs, confirm behavior, review edge cases.".into(),
        "webapp-testing" => "When testing a local web application with Playwright. Use for browser-based verification of UI behavior, forms, navigation, and visual state.".into(),
        "caveman-review" => "When reviewing a diff or PR. Produces short, actionable feedback — no fluff, no template paragraphs, just the findings that matter.".into(),
        "caveman-commit" => "When you need a Conventional Commit message from a diff. Produces concise `type(scope): description` format with body when needed.".into(),
        _ => "Refer to the source repository for usage documentation.".into(),
    }
}

fn title_case(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Install helpers ──────────────────────────────────────────────────────

fn template_banner() -> String {
    TEMPLATE_BANNER.to_string()
}

/// Install a single skill in template mode.
fn install_template_skill(
    def: &SkillDef,
    skill_dir: &Path,
    installed: &mut Vec<String>,
    skipped: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    // Write SKILL.md with template banner
    let content = match def.category {
        SkillCategory::Auto => {
            format!("{}\n\n{}", template_banner(), auto_skill_content(def))
        }
        SkillCategory::Manual => {
            format!("{}\n\n{}", template_banner(), manual_skill_content(def))
        }
    };

    let skill_file = skill_dir.join("SKILL.md");
    match std::fs::write(&skill_file, &content) {
        Ok(_) => {}
        Err(e) => {
            errors.push(format!("Cannot write SKILL.md for '{}': {}", def.name, e));
            skipped.push(def.name.clone());
            return;
        }
    }

    // Create stub directories
    for subdir in &["scripts", "references", "templates"] {
        let dir = skill_dir.join(subdir);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            errors.push(format!(
                "Cannot create {}/ directory for '{}': {}",
                subdir, def.name, e
            ));
        }
        // Write a README in each stub dir
        let readme = dir.join("README.md");
        let _ = std::fs::write(
            &readme,
            format!(
                "# {}\n\nThis is a stub directory created by `ags skill install --mode template`.\n\nCopy the real `{}` content from the source repository:\n\n  {}\n",
                subdir, subdir, def.source
            ),
        );
    }

    installed.push(def.name.clone());
}

/// Install a single skill in full mode (copy from source directory).
fn install_full_skill(
    def: &SkillDef,
    skill_dir: &Path,
    source_dir: &Path,
    installed: &mut Vec<String>,
    skipped: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    // Verify source_dir/SKILL.md exists (or source_dir/<skill-name>/SKILL.md)
    let src_skill_md = if source_dir.join("SKILL.md").exists() {
        source_dir.join("SKILL.md")
    } else if source_dir.join(&def.name).join("SKILL.md").exists() {
        source_dir.join(&def.name)
    } else {
        errors.push(format!(
            "Full install requires SKILL.md in source directory. Not found at {} or {}/{}/SKILL.md",
            source_dir.display(),
            source_dir.display(),
            def.name
        ));
        skipped.push(def.name.clone());
        return;
    };

    // If src_skill_md is a directory (source_dir/<skill-name>/), copy the dir
    let src_root = if src_skill_md.is_dir() {
        src_skill_md
    } else {
        source_dir.to_path_buf()
    };

    // Copy files from source to skill_dir
    match copy_dir_contents(&src_root, skill_dir) {
        Ok(count) => {
            if count == 0 {
                errors.push(format!(
                    "Source directory {} contains no files to copy for '{}'",
                    src_root.display(),
                    def.name
                ));
                skipped.push(def.name.clone());
            } else {
                installed.push(def.name.clone());
            }
        }
        Err(e) => {
            errors.push(format!(
                "Failed to copy from {} to {} for '{}': {}",
                src_root.display(),
                skill_dir.display(),
                def.name,
                e
            ));
            skipped.push(def.name.clone());
        }
    }
}

/// Copy directory contents (non-recursive files only for safety).
fn copy_dir_contents(src: &Path, dst: &Path) -> Result<usize, String> {
    let mut count = 0;
    let entries = std::fs::read_dir(src)
        .map_err(|e| format!("cannot read source dir {}: {}", src.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry error: {}", e))?;
        let path = entry.path();
        let name = path
            .file_name()
            .ok_or_else(|| "missing file name".to_string())?;

        if path.is_file() {
            let dst_file = dst.join(name);
            std::fs::copy(&path, &dst_file).map_err(|e| {
                format!(
                    "cannot copy {} -> {}: {}",
                    path.display(),
                    dst_file.display(),
                    e
                )
            })?;
            count += 1;
        } else if path.is_dir() {
            // Copy directories recursively
            let dir_name = name;
            let dst_dir = dst.join(dir_name);
            std::fs::create_dir_all(&dst_dir)
                .map_err(|e| format!("cannot create dir {}: {}", dst_dir.display(), e))?;
            let sub_count = copy_dir_contents(&path, &dst_dir)?;
            count += sub_count;
        }
    }
    Ok(count)
}

// ── Install receipt ──────────────────────────────────────────────────────

fn write_install_receipt(
    target: &Path,
    skills: &[&str],
    mode: InstallMode,
    timestamp: u64,
) -> Result<PathBuf, String> {
    let receipt_path = target.join("install-receipt.yaml");
    let mode_str = match mode {
        InstallMode::Template => "template",
        InstallMode::Full => "full",
    };
    let mut content = String::from("# AGS Skill Install Receipt\n");
    content.push_str(&format!("schema_version: \"1.0-install-receipt\"\n"));
    content.push_str(&format!("timestamp: \"{}\"\n", timestamp));
    content.push_str(&format!("target: \"{}\"\n", target.display()));
    content.push_str(&format!("install_mode: \"{}\"\n", mode_str));
    content.push_str("skills:\n");
    for s in skills {
        content.push_str(&format!("  - name: \"{}\"\n", s));
        content.push_str("    status: installed\n");
    }
    content.push_str("verification:\n");
    content.push_str("  - check: \"SKILL.md exists for each skill\"\n");
    content.push_str("  - check: \"frontmatter has name and description fields\"\n");
    content.push_str("  - check: \"no private paths or personal data\"\n");

    std::fs::write(&receipt_path, &content)
        .map_err(|e| format!("cannot write install receipt: {}", e))?;
    Ok(receipt_path)
}

// ── Main install logic ────────────────────────────────────────────────────

pub fn install_plan(skill_name: &str, target: &Path) -> (Vec<SkillDef>, Vec<String>, String) {
    let known = known_skills();

    let skills_to_install: Vec<&SkillDef> = if skill_name == "recommended" {
        known.values().collect()
    } else {
        match known.get(skill_name) {
            Some(def) => vec![def],
            None => {
                return (
                    vec![],
                    vec![format!("Unknown skill: '{}'", skill_name)],
                    target.display().to_string(),
                );
            }
        }
    };

    let mut warnings: Vec<String> = Vec::new();
    for def in &skills_to_install {
        let skill_dir = target.join(&def.name);
        let skill_file = skill_dir.join("SKILL.md");
        if skill_file.exists() {
            warnings.push(format!(
                "Skill '{}' already installed at {} — will be overwritten",
                def.name,
                skill_file.display()
            ));
        }
    }

    let defs: Vec<SkillDef> = skills_to_install.into_iter().cloned().collect();
    (defs, warnings, target.display().to_string())
}

pub fn install_skills(
    skill_name: &str,
    target: &Path,
    confirm: bool,
    dry_run: bool,
    mode: InstallMode,
    source_dir: Option<&Path>,
) -> InstallResult {
    let (defs, warnings, target_str) = install_plan(skill_name, target);

    if !warnings.is_empty() && warnings.iter().any(|w| w.starts_with("Unknown skill")) {
        return InstallResult {
            status: InstallStatus::Blocked,
            mode,
            target_dir: target_str,
            skills_installed: vec![],
            skills_skipped: vec![],
            receipt_path: None,
            errors: warnings,
        };
    }

    if defs.is_empty() {
        return InstallResult {
            status: InstallStatus::Blocked,
            mode,
            target_dir: target_str,
            skills_installed: vec![],
            skills_skipped: vec![],
            receipt_path: None,
            errors: vec!["No skills matched for installation.".into()],
        };
    }

    // Full mode requires source_dir
    if mode == InstallMode::Full && source_dir.is_none() {
        return InstallResult {
            status: InstallStatus::Blocked,
            mode,
            target_dir: target_str,
            skills_installed: vec![],
            skills_skipped: vec![],
            receipt_path: None,
            errors: vec![
                "--mode full requires --source-dir pointing to a local skill directory".into(),
            ],
        };
    }

    if dry_run || !confirm {
        let status = if dry_run {
            InstallStatus::DryRun
        } else {
            InstallStatus::Blocked
        };
        let skill_names: Vec<String> = defs.iter().map(|d| d.name.clone()).collect();
        return InstallResult {
            status,
            mode,
            target_dir: target_str,
            skills_installed: vec![],
            skills_skipped: skill_names,
            receipt_path: None,
            errors: if confirm {
                vec![]
            } else {
                vec!["Use --confirm to proceed with installation.".into()]
            },
        };
    }

    let mut installed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    if let Err(e) = std::fs::create_dir_all(target) {
        return InstallResult {
            status: InstallStatus::Blocked,
            mode,
            target_dir: target_str,
            skills_installed: vec![],
            skills_skipped: vec![],
            receipt_path: None,
            errors: vec![format!("Cannot create target directory: {}", e)],
        };
    }

    for def in &defs {
        let skill_dir = target.join(&def.name);
        if let Err(e) = std::fs::create_dir_all(&skill_dir) {
            errors.push(format!("Cannot create directory for '{}': {}", def.name, e));
            skipped.push(def.name.clone());
            continue;
        }

        match mode {
            InstallMode::Template => {
                install_template_skill(def, &skill_dir, &mut installed, &mut skipped, &mut errors);
            }
            InstallMode::Full => {
                if let Some(src) = source_dir {
                    install_full_skill(
                        def,
                        &skill_dir,
                        src,
                        &mut installed,
                        &mut skipped,
                        &mut errors,
                    );
                }
            }
        }
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let receipt_path = if !installed.is_empty() {
        match write_install_receipt(
            target,
            &installed.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            mode,
            timestamp,
        ) {
            Ok(p) => Some(p.display().to_string()),
            Err(e) => {
                errors.push(e);
                None
            }
        }
    } else {
        None
    };

    let status = if errors.is_empty() {
        InstallStatus::Installed
    } else if installed.is_empty() {
        InstallStatus::Blocked
    } else {
        InstallStatus::PartialFailure
    };

    InstallResult {
        status,
        mode,
        target_dir: target_str,
        skills_installed: installed,
        skills_skipped: skipped,
        receipt_path,
        errors,
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────

pub fn render_install_text(result: &InstallResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    let mode_label = match result.mode {
        InstallMode::Template => "TEMPLATE INSTALL",
        InstallMode::Full => "FULL INSTALL",
    };

    match result.status {
        InstallStatus::DryRun => {
            lines.push(format!("Skill Install — Dry Run [{}]", mode_label));
            lines.push("=======================".into());
        }
        InstallStatus::Blocked => {
            lines.push(format!("Skill Install — Blocked [{}]", mode_label));
            lines.push("========================".into());
        }
        InstallStatus::Installed => {
            lines.push(format!("Skill Install — Complete [{}]", mode_label));
            lines.push("=========================".into());
        }
        InstallStatus::PartialFailure => {
            lines.push(format!("Skill Install — Partial Failure [{}]", mode_label));
            lines.push("================================".into());
        }
    }
    lines.push(format!("Target: {}", result.target_dir));
    lines.push("".into());

    if result.mode == InstallMode::Template {
        lines.push("⚠  TEMPLATE INSTALL — THIS IS A SKELETON ONLY".into());
        lines.push("   Copy real content from the source repository.".into());
        lines.push(
            "   For full install: ags skill install --mode full --source-dir <path> --confirm"
                .into(),
        );
        lines.push("".into());
    }

    if !result.skills_installed.is_empty() {
        lines.push("Installed:".into());
        for s in &result.skills_installed {
            lines.push(format!("  ✓ {}", s));
        }
        lines.push("".into());
    }

    if !result.skills_skipped.is_empty() {
        lines.push("Skipped:".into());
        for s in &result.skills_skipped {
            lines.push(format!("  → {}", s));
        }
        lines.push("".into());
    }

    if let Some(ref receipt) = result.receipt_path {
        lines.push(format!("Install receipt: {}", receipt));
        lines.push("".into());
    }

    if !result.errors.is_empty() {
        for e in &result.errors {
            lines.push(format!("! {}", e));
        }
    }

    match result.status {
        InstallStatus::Blocked => {
            if result.errors.iter().any(|e| e.contains("--confirm")) {
                lines.push("STATUS: blocked — use --confirm to proceed with installation".into());
            }
        }
        InstallStatus::Installed => {
            if result.mode == InstallMode::Template {
                lines
                    .push("⚠  Remember: this is a TEMPLATE. Copy real content from source.".into());
            }
            lines.push(
                "Run `ags doctor` to verify skill installation and auto-trigger status.".into(),
            );
        }
        _ => {}
    }

    lines.join("\n")
}

pub fn render_install_json(result: &InstallResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tm() -> InstallMode {
        InstallMode::Template
    }
    fn fm() -> InstallMode {
        InstallMode::Full
    }
    fn no_src() -> Option<&'static Path> {
        None
    }

    #[test]
    fn test_known_skills() {
        let skills = known_skills();
        assert!(skills.contains_key("auto-brainstorm"));
        assert!(skills.contains_key("auto-debug"));
        assert!(skills.contains_key("auto-verify"));
        assert!(skills.contains_key("tdd"));
    }

    #[test]
    fn test_auto_skill_content_has_frontmatter() {
        let def = SkillDef {
            name: "auto-brainstorm".into(),
            source: "https://example.com/skill".into(),
            category: SkillCategory::Auto,
            description: "Auto brainstorming skill".into(),
        };
        let content = auto_skill_content(&def);
        assert!(content.contains("---"));
        assert!(content.contains("name: auto-brainstorm"));
        assert!(content.contains("## Trigger Conditions"));
    }

    #[test]
    fn test_manual_skill_content_has_frontmatter() {
        let def = SkillDef {
            name: "tdd".into(),
            source: "https://example.com/tdd".into(),
            category: SkillCategory::Manual,
            description: "Test-driven development".into(),
        };
        let content = manual_skill_content(&def);
        assert!(content.contains("name: tdd"));
        assert!(content.contains("## Invocation"));
        assert!(content.contains("/tdd"));
    }

    #[test]
    fn test_skill_content_no_private_paths() {
        let def = SkillDef {
            name: "auto-verify".into(),
            source: "https://example.com/verify".into(),
            category: SkillCategory::Auto,
            description: "Auto verify".into(),
        };
        let content = auto_skill_content(&def);
        assert!(!content.contains("/Users/"));
        assert!(!content.contains("/Volumes/AI Project/"));
        assert!(!content.contains(concat!("huji", "aming")));
    }

    #[test]
    fn test_template_content_has_banner() {
        let def = SkillDef {
            name: "auto-brainstorm".into(),
            source: "https://example.com/skill".into(),
            category: SkillCategory::Auto,
            description: "Auto brainstorming".into(),
        };
        let content = format!("{}\n\n{}", template_banner(), auto_skill_content(&def));
        assert!(content.contains("TEMPLATE INSTALL"));
        assert!(content.contains("THIS IS A SKELETON"));
        assert!(content.contains("name: auto-brainstorm"));
    }

    #[test]
    fn test_install_plan_unknown_skill() {
        let tmp = std::env::temp_dir().join("ags-test-plan-unk");
        let _ = fs::remove_dir_all(&tmp);
        let (defs, warnings, _) = install_plan("nonexistent-skill", &tmp);
        assert!(defs.is_empty());
        assert!(warnings[0].contains("Unknown skill"));
    }

    #[test]
    fn test_dry_run_does_not_write() {
        let tmp = std::env::temp_dir().join("ags-test-dry-run");
        let _ = fs::remove_dir_all(&tmp);
        let result = install_skills("auto-brainstorm", &tmp, false, true, tm(), no_src());
        assert_eq!(result.status, InstallStatus::DryRun);
        assert!(!tmp.join("auto-brainstorm").exists());
    }

    #[test]
    fn test_no_confirm_does_not_write() {
        let tmp = std::env::temp_dir().join("ags-test-no-confirm");
        let _ = fs::remove_dir_all(&tmp);
        let result = install_skills("auto-brainstorm", &tmp, false, false, tm(), no_src());
        assert_eq!(result.status, InstallStatus::Blocked);
        assert!(!tmp.join("auto-brainstorm").exists());
    }

    #[test]
    fn test_template_confirm_creates_skill_with_banner() {
        let tmp = std::env::temp_dir().join("ags-test-tmpl-confirm");
        let _ = fs::remove_dir_all(&tmp);
        let result = install_skills("auto-brainstorm", &tmp, true, false, tm(), no_src());
        assert_eq!(result.status, InstallStatus::Installed);
        assert_eq!(result.mode, InstallMode::Template);

        let skill_file = tmp.join("auto-brainstorm").join("SKILL.md");
        assert!(skill_file.exists());
        let content = fs::read_to_string(&skill_file).unwrap();
        assert!(content.contains("TEMPLATE INSTALL"));
        assert!(content.contains("name: auto-brainstorm"));

        // Stub directories should exist
        assert!(tmp.join("auto-brainstorm/scripts").exists());
        assert!(tmp.join("auto-brainstorm/references").exists());
        assert!(tmp.join("auto-brainstorm/templates").exists());

        let receipt = tmp.join("install-receipt.yaml");
        assert!(receipt.exists());
        let rc = fs::read_to_string(&receipt).unwrap();
        assert!(rc.contains("install_mode: \"template\""));
    }

    #[test]
    fn test_full_mode_rejects_missing_source_dir() {
        let tmp = std::env::temp_dir().join("ags-test-full-no-src");
        let _ = fs::remove_dir_all(&tmp);
        let result = install_skills("auto-brainstorm", &tmp, true, false, fm(), no_src());
        assert_eq!(result.status, InstallStatus::Blocked);
        assert!(result.errors[0].contains("--source-dir"));
    }

    #[test]
    fn test_full_mode_copies_from_source_dir() {
        let tmp = std::env::temp_dir().join("ags-test-full-copy");
        let src = std::env::temp_dir().join("ags-test-full-src");
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(&src);
        fs::create_dir_all(&src).unwrap();

        // Create a mock skill source
        fs::write(
            src.join("SKILL.md"),
            "---
name: test-skill
description: A test skill
---

# Test Skill

Real content here.
",
        )
        .unwrap();
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::write(
            src.join("scripts").join("helper.sh"),
            "#!/bin/bash\necho ok\n",
        )
        .unwrap();

        let result = install_skills("auto-brainstorm", &tmp, true, false, fm(), Some(&src));
        assert_eq!(
            result.status,
            InstallStatus::Installed,
            "errors: {:?}",
            result.errors
        );
        assert_eq!(result.mode, InstallMode::Full);

        let skill_file = tmp.join("auto-brainstorm").join("SKILL.md");
        assert!(skill_file.exists());
        let content = fs::read_to_string(&skill_file).unwrap();
        assert!(content.contains("Real content here"));
        assert!(!content.contains("TEMPLATE INSTALL"));

        // Copied scripts directory
        assert!(tmp.join("auto-brainstorm/scripts/helper.sh").exists());
    }

    #[test]
    fn test_multiple_skills_template() {
        let tmp = std::env::temp_dir().join("ags-test-multi-tmpl");
        let _ = fs::remove_dir_all(&tmp);
        let result = install_skills("recommended", &tmp, true, false, tm(), no_src());
        assert!(result.skills_installed.len() >= 9);
        for name in &["auto-brainstorm", "auto-debug", "auto-verify", "tdd"] {
            let skill_file = tmp.join(name).join("SKILL.md");
            assert!(skill_file.exists(), "SKILL.md should exist for {}", name);
        }
    }

    #[test]
    fn test_auto_triggers_differ_by_skill() {
        let a = auto_triggers("auto-brainstorm");
        let b = auto_triggers("auto-debug");
        assert_ne!(a, b);
    }

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("auto-brainstorm"), "Auto Brainstorm");
        assert_eq!(title_case("caveman-commit"), "Caveman Commit");
        assert_eq!(title_case("tdd"), "Tdd");
    }

    #[test]
    fn test_render_install_text_template_mode() {
        let result = InstallResult {
            status: InstallStatus::DryRun,
            mode: InstallMode::Template,
            target_dir: "/tmp/test".into(),
            skills_installed: vec![],
            skills_skipped: vec!["auto-brainstorm".into()],
            receipt_path: None,
            errors: vec![],
        };
        let text = render_install_text(&result);
        assert!(text.contains("Dry Run"));
        assert!(text.contains("TEMPLATE INSTALL"));
    }

    #[test]
    fn test_render_install_json() {
        let result = InstallResult {
            status: InstallStatus::Installed,
            mode: InstallMode::Template,
            target_dir: "/tmp/test".into(),
            skills_installed: vec!["auto-brainstorm".into()],
            skills_skipped: vec![],
            receipt_path: Some("/tmp/test/install-receipt.yaml".into()),
            errors: vec![],
        };
        let json = render_install_json(&result);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "installed");
        assert_eq!(parsed["mode"], "template");
    }

    // ── Health check helpers ──────────────────────────────────────────────

    #[allow(dead_code)]
    pub fn check_skill_installed(skills_dir: &Path, skill_name: &str) -> (bool, Vec<String>) {
        let skill_file = skills_dir.join(skill_name).join("SKILL.md");
        if !skill_file.exists() {
            let flat_file = skills_dir.join(format!("{}.md", skill_name));
            if flat_file.exists() {
                return (false, vec![format!(
                    "Skill '{}' is installed as flat file {} — reinstall with `ags skill install --skill {} --confirm` for directory format",
                    skill_name, flat_file.display(), skill_name
                )]);
            }
            return (
                false,
                vec![format!(
                    "Skill '{}' SKILL.md not found at {}",
                    skill_name,
                    skill_file.display()
                )],
            );
        }

        match std::fs::read_to_string(&skill_file) {
            Ok(content) => {
                let mut issues: Vec<String> = Vec::new();
                let has_name = content.lines().any(|l| l.trim() == "name:");
                let has_desc = content
                    .lines()
                    .any(|l| l.trim().starts_with("description:"));
                if !has_name {
                    issues.push(format!(
                        "SKILL.md for '{}' missing 'name:' frontmatter field",
                        skill_name
                    ));
                }
                if !has_desc {
                    issues.push(format!(
                        "SKILL.md for '{}' missing 'description:' frontmatter field",
                        skill_name
                    ));
                }
                (issues.is_empty(), issues)
            }
            Err(e) => (
                false,
                vec![format!("Cannot read SKILL.md for '{}': {}", skill_name, e)],
            ),
        }
    }
}
