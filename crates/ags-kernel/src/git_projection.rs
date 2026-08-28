//! Repository-local Git projection for AGS entry blocks.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::sync::{BLOCK_BEGIN, ENTRY_FILES};

const ATTR_BEGIN: &str = "# BEGIN AGS LOCAL ENTRY FILTER";
const ATTR_END: &str = "# END AGS LOCAL ENTRY FILTER";
const EXCLUDE_BEGIN: &str = "# BEGIN AGS LOCAL RUNTIME";
const EXCLUDE_END: &str = "# END AGS LOCAL RUNTIME";
const CLEAN_FILTER: &str = "ags entry-filter clean || { echo 'AGS entry filter unavailable; reinstall AGS, then run: ags update --workspace .' >&2; exit 1; }";
const SMUDGE_FILTER: &str = "ags entry-filter smudge || { echo 'AGS entry filter unavailable; reinstall AGS, then run: ags update --workspace .' >&2; exit 1; }";
const LOCAL_EXCLUDES: &[&str] = &[
    "/ags.toml",
    "/.ags/",
    "/.claude/settings.json",
    "/.codex/hooks.json",
    "/.cursor/hooks.json",
    "/.codebuddy/settings.local.json",
    "/.omp/extensions/ags-policy.js",
    "/.dsh/",
    "/.workbuddy/memory/",
];

fn local_excludes() -> Vec<String> {
    ENTRY_FILES
        .iter()
        .map(|name| format!("/{name}"))
        .chain(std::iter::once("/AGENT_SUITE_PROTOCOL.md".to_string()))
        .chain(LOCAL_EXCLUDES.iter().map(|path| (*path).to_string()))
        .collect()
}

pub fn install(root: &Path) -> Result<Vec<String>> {
    if !is_git_worktree(root) {
        return Ok(Vec::new());
    }
    run_git(
        root,
        &["config", "--local", "filter.ags-entry.clean", CLEAN_FILTER],
    )?;
    run_git(
        root,
        &[
            "config",
            "--local",
            "filter.ags-entry.smudge",
            SMUDGE_FILTER,
        ],
    )?;
    run_git(
        root,
        &["config", "--local", "filter.ags-entry.required", "true"],
    )?;

    let attributes = git_path(root, "info/attributes")?;
    let attribute_body = ENTRY_FILES
        .iter()
        .map(|name| format!("/{name} filter=ags-entry"))
        .collect::<Vec<_>>()
        .join("\n");
    write_local_block(&attributes, ATTR_BEGIN, ATTR_END, &attribute_body)?;

    let exclude = git_path(root, "info/exclude")?;
    write_local_block(
        &exclude,
        EXCLUDE_BEGIN,
        EXCLUDE_END,
        &local_excludes().join("\n"),
    )?;
    clear_legacy_skip_worktree(root)?;
    if entry_filter_available() {
        refresh_clean_entries(root)?;
    }
    Ok(vec![
        "git-local:filter.ags-entry".to_string(),
        "git-local:info/attributes".to_string(),
        "git-local:info/exclude".to_string(),
    ])
}

pub fn preflight(root: &Path) -> Result<()> {
    if !is_git_worktree(root) {
        return Ok(());
    }
    let index_lock = git_path(root, "index.lock")?;
    if index_lock.exists() {
        return Err(Error::new(
            "git_index_locked",
            format!(
                "{} exists; finish or stop the active Git operation",
                index_lock.display()
            ),
        ));
    }
    if !entry_filter_available() {
        return Err(Error::new(
            "entry_filter_unavailable",
            "`ags entry-filter` is unavailable on PATH; reinstall AGS before update",
        ));
    }
    for relative in ["info/attributes", "info/exclude"] {
        let path = git_path(root, relative)?;
        if let Some(parent) = path.parent() {
            let readonly = parent.exists()
                && parent
                    .metadata()
                    .map_err(|e| crate::error::io("git_projection_preflight_failed", &e))?
                    .permissions()
                    .readonly();
            if readonly {
                return Err(Error::new(
                    "git_projection_not_writable",
                    format!("{} is read-only", parent.display()),
                ));
            }
        }
    }
    for name in ENTRY_FILES {
        if git_success(root, &["ls-files", "--error-unmatch", name]) {
            git_output(root, &["show", &format!(":{name}")])?;
        }
    }
    Ok(())
}

fn clear_legacy_skip_worktree(root: &Path) -> Result<()> {
    for name in ENTRY_FILES {
        if git_success(root, &["ls-files", "--error-unmatch", name]) {
            run_git(root, &["update-index", "--no-skip-worktree", "--", name])?;
        }
    }
    Ok(())
}

fn entry_filter_available() -> bool {
    Command::new("ags")
        .args(["entry-filter", "clean"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn refresh_clean_entries(root: &Path) -> Result<()> {
    for name in ENTRY_FILES {
        if !git_success(root, &["ls-files", "--error-unmatch", name]) {
            continue;
        }
        let working = fs::read_to_string(root.join(name)).unwrap_or_default();
        let indexed = git_output(root, &["show", &format!(":{name}")])?;
        if crate::sync::strip_entry_text(&working) == indexed {
            run_git(root, &["add", "--", name])?;
        }
    }
    Ok(())
}

pub fn drift(root: &Path) -> Result<Vec<String>> {
    if !is_git_worktree(root) {
        return Ok(Vec::new());
    }
    let mut findings = Vec::new();
    for (key, expected) in [
        ("filter.ags-entry.clean", CLEAN_FILTER),
        ("filter.ags-entry.smudge", SMUDGE_FILTER),
        ("filter.ags-entry.required", "true"),
    ] {
        let output = git_output_optional(root, &["config", "--local", "--get", key])?;
        if output.as_deref().map(str::trim) != Some(expected) {
            findings.push(format!("git local filter {key} is not installed"));
        }
    }

    let attributes = fs::read_to_string(git_path(root, "info/attributes")?).unwrap_or_default();
    if !attributes.contains(ATTR_BEGIN) || !attributes.contains(ATTR_END) {
        findings.push("git info/attributes lacks AGS local entry filter".to_string());
    }
    for name in ENTRY_FILES {
        let expected = format!("/{name} filter=ags-entry");
        if !attributes.lines().any(|line| line.trim() == expected) {
            findings.push(format!("git info/attributes lacks filter for {name}"));
        }
    }
    let exclude = fs::read_to_string(git_path(root, "info/exclude")?).unwrap_or_default();
    if !exclude.contains(EXCLUDE_BEGIN) || !exclude.contains(EXCLUDE_END) {
        findings.push("git info/exclude lacks AGS local runtime block".to_string());
    }
    for expected in local_excludes() {
        if !exclude.lines().any(|line| line.trim() == expected) {
            findings.push(format!("git info/exclude lacks {expected}"));
        }
    }

    for name in ENTRY_FILES {
        if git_success(root, &["ls-files", "--error-unmatch", name]) {
            let indexed = git_output(root, &["show", &format!(":{name}")])?;
            if indexed.contains(BLOCK_BEGIN) {
                findings.push(format!(
                    "git baseline for {name} still contains an AGS managed block"
                ));
            }
        }
    }
    Ok(findings)
}

fn is_git_worktree(root: &Path) -> bool {
    git_success(root, &["rev-parse", "--is-inside-work-tree"])
}

fn git_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let output = git_output(root, &["rev-parse", "--git-path", relative])?;
    let path = PathBuf::from(output.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn run_git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| crate::error::io("git_local_projection_failed", &e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::new(
            "git_local_projection_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn git_output(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| crate::error::io("git_local_projection_failed", &e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::new(
            "git_local_projection_failed",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn git_output_optional(root: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| crate::error::io("git_local_projection_failed", &e))?;
    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()));
    }
    if output.status.code() == Some(1) && output.stderr.is_empty() {
        return Ok(None);
    }
    Err(Error::new(
        "git_local_projection_failed",
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

fn git_success(root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn write_local_block(path: &Path, begin: &str, end: &str, body: &str) -> Result<()> {
    let current = fs::read_to_string(path).unwrap_or_default();
    let block = format!("{begin}\n{body}\n{end}");
    let next = if let (Some(start), Some(finish)) = (current.find(begin), current.find(end)) {
        let finish = finish + end.len();
        format!("{}{}{}", &current[..start], block, &current[finish..])
    } else {
        let prefix = current.trim_end();
        if prefix.is_empty() {
            block
        } else {
            format!("{prefix}\n{block}")
        }
    };
    let mut next = next.trim_end().to_string();
    next.push('\n');
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| crate::error::io("git_local_projection_failed", &e))?;
    }
    fs::write(path, next).map_err(|e| crate::error::io("git_local_projection_failed", &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        assert!(Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn installs_repository_local_filter_and_excludes() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q"]);
        fs::create_dir_all(tmp.path().join("nested")).unwrap();
        fs::write(tmp.path().join("nested/AGENTS.md"), "nested\n").unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "root\n").unwrap();
        // Establish the tracked baseline before installing the required
        // filter. The product intentionally fails closed when `ags` is not on
        // PATH, while a library unit test must not depend on a globally
        // installed CLI being present on the CI runner.
        git(tmp.path(), &["add", "-f", "AGENTS.md"]);
        git(
            tmp.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "baseline",
            ],
        );
        install(tmp.path()).unwrap();

        let attributes =
            fs::read_to_string(git_path(tmp.path(), "info/attributes").unwrap()).unwrap();
        assert!(attributes.contains("/AGENTS.md filter=ags-entry"));
        assert!(attributes.contains("/codebuddy.md filter=ags-entry"));
        assert_eq!(
            git_output(
                tmp.path(),
                &["check-attr", "filter", "--", "nested/AGENTS.md"]
            )
            .unwrap()
            .trim(),
            "nested/AGENTS.md: filter: unspecified"
        );

        git(
            tmp.path(),
            &["update-index", "--skip-worktree", "--", "AGENTS.md"],
        );
        install(tmp.path()).unwrap();
        assert!(git_output(tmp.path(), &["ls-files", "-v", "AGENTS.md"])
            .unwrap()
            .starts_with("H "));

        let exclude = fs::read_to_string(git_path(tmp.path(), "info/exclude").unwrap()).unwrap();
        assert!(exclude.contains("/ags.toml"));
        assert!(exclude.contains("/.ags/"));
        assert!(exclude.contains("/.claude/settings.json"));
        assert!(exclude.contains("/AGENTS.md"));
        assert!(exclude.contains("/codebuddy.md"));
        assert!(exclude.contains("/AGENT_SUITE_PROTOCOL.md"));
        assert!(!exclude.contains("/.claude/\n"));

        fs::write(tmp.path().join("codebuddy.md"), "local AGS entry\n").unwrap();
        assert!(git_success(tmp.path(), &["check-ignore", "codebuddy.md"]));

        fs::create_dir_all(tmp.path().join(".claude/skills/example")).unwrap();
        fs::write(
            tmp.path().join(".claude/skills/example/SKILL.md"),
            "project skill\n",
        )
        .unwrap();
        assert!(!git_success(
            tmp.path(),
            &["check-ignore", ".claude/skills/example/SKILL.md"]
        ));
        assert!(git_success(
            tmp.path(),
            &["check-ignore", ".claude/settings.json"]
        ));

        assert_eq!(
            git_output(
                tmp.path(),
                &["config", "--local", "--get", "filter.ags-entry.clean"]
            )
            .unwrap()
            .trim(),
            CLEAN_FILTER
        );
        assert!(drift(tmp.path()).unwrap().is_empty());
        let error = git_output(tmp.path(), &["show", ":missing-entry"]).unwrap_err();
        assert_eq!(error.code, "git_local_projection_failed");
        fs::write(git_path(tmp.path(), "index.lock").unwrap(), "locked").unwrap();
        let error = preflight(tmp.path()).unwrap_err();
        assert_eq!(error.code, "git_index_locked");
    }
}
