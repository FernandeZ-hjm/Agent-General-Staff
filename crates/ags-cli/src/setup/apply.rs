use crate::context::sanitize_name;
use crate::file_plan::InstallFile;
use crate::host_probe::command_in_path;
use std::path::{Path, PathBuf};

fn is_codex_skill_path(path: &Path) -> bool {
    let parts: Vec<_> = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect();
    parts
        .windows(2)
        .any(|window| window[0] == ".codex" && window[1] == "skills")
}

pub(in crate::setup) fn symlink_ancestor(path: &Path) -> Option<PathBuf> {
    path.parent()?.ancestors().find_map(|ancestor| {
        std::fs::symlink_metadata(ancestor)
            .ok()
            .filter(|meta| meta.file_type().is_symlink())
            .map(|_| ancestor.to_path_buf())
    })
}

pub(in crate::setup) fn codex_skill_thin_index_ancestor(path: &Path) -> Option<PathBuf> {
    if is_codex_skill_path(path) {
        symlink_ancestor(path)
    } else {
        None
    }
}

pub(in crate::setup) fn write_install_file(
    file: &InstallFile,
    force: bool,
    backup_stamp: u64,
) -> suite_doctor::Finding {
    if let Some(link) = codex_skill_thin_index_ancestor(&file.path) {
        return suite_doctor::Finding::pass(
            format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
            format!(
                "skipped thin-index symlink: {} (ancestor {}; canonical skill body remains authoritative)",
                file.path.display(),
                link.display()
            ),
        );
    }

    if let Some(parent) = file.path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return suite_doctor::Finding::fail(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("cannot create directory {}", parent.display()),
                e.to_string(),
            );
        }
    }

    match std::fs::read(&file.path) {
        Ok(existing) if existing == file.content.as_bytes() => {
            return suite_doctor::Finding::pass(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("unchanged: {}", file.path.display()),
            );
        }
        Ok(_) if !force => {
            return suite_doctor::Finding::fail(
                format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                format!("exists with different content: {}", file.path.display()),
                "Review `ags setup`, then rerun setup with --force --yes if replacement is intended.",
            );
        }
        Ok(_) => {
            let backup = file.path.with_extension(format!(
                "{}.bak.{backup_stamp}",
                file.path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
            ));
            if let Err(e) = std::fs::copy(&file.path, &backup) {
                return suite_doctor::Finding::fail(
                    format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
                    format!("backup failed for {}", file.path.display()),
                    e.to_string(),
                );
            }
        }
        Err(_) => {}
    }

    if let Err(e) = std::fs::write(&file.path, &file.content) {
        return suite_doctor::Finding::fail(
            format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
            format!("write failed: {}", file.path.display()),
            e.to_string(),
        );
    }

    #[cfg(unix)]
    if let Some(mode) = file.mode {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&file.path) {
            let mut perms = metadata.permissions();
            perms.set_mode(mode);
            let _ = std::fs::set_permissions(&file.path, perms);
        }
    }

    suite_doctor::Finding::pass(
        format!("install-{}", sanitize_name(&file.path.to_string_lossy())),
        format!("written: {}", file.path.display()),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::write_install_file;
    use crate::file_plan::InstallFile;
    use std::path::{Path, PathBuf};

    fn tmp(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("ags-setup-apply-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn install_file(path: PathBuf, content: &str) -> InstallFile {
        InstallFile {
            path,
            description: "test".to_string(),
            content: content.to_string(),
            mode: None,
        }
    }

    #[cfg(unix)]
    fn symlink_dir(src: &Path, dst: &Path) {
        std::os::unix::fs::symlink(src, dst).unwrap();
    }

    /// Regression: once `ags capability sync --apply` has made
    /// `~/.codex/skills/<name>` a thin-index symlink to the canonical repo skill,
    /// setup must not write `SKILL.md` through that symlink and mutate the
    /// canonical body.
    #[cfg(unix)]
    #[test]
    fn setup_skips_codex_skill_files_under_symlink_thin_index() {
        let root = tmp("codex-symlink");
        let canonical = root.join("repo/global-skills/ags-setup");
        let host = root.join("home/.codex/skills");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(canonical.join("SKILL.md"), "canonical\n").unwrap();
        symlink_dir(&canonical, &host.join("ags-setup"));

        let file = install_file(host.join("ags-setup/SKILL.md"), "generated\n");
        let finding = write_install_file(&file, true, 123);

        assert_eq!(finding.status, suite_doctor::CheckStatus::Pass);
        assert!(finding.message.contains("skipped thin-index symlink"));
        assert_eq!(
            std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "canonical\n"
        );
        assert!(!canonical.join("SKILL.md.bak.123").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn setup_skips_codex_skill_metadata_under_symlink_thin_index() {
        let root = tmp("codex-symlink-metadata");
        let canonical = root.join("repo/global-skills/ags-setup");
        let host = root.join("home/.codex/skills");
        std::fs::create_dir_all(canonical.join("agents")).unwrap();
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(canonical.join("agents/openai.yaml"), "canonical-meta\n").unwrap();
        symlink_dir(&canonical, &host.join("ags-setup"));

        let file = install_file(
            host.join("ags-setup/agents/openai.yaml"),
            "generated-meta\n",
        );
        let finding = write_install_file(&file, true, 123);

        assert_eq!(finding.status, suite_doctor::CheckStatus::Pass);
        assert!(finding.message.contains("skipped thin-index symlink"));
        assert_eq!(
            std::fs::read_to_string(canonical.join("agents/openai.yaml")).unwrap(),
            "canonical-meta\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
fn run_claude_mcp_command(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("claude")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(combined.trim().to_string())
    } else {
        Err(combined.trim().to_string())
    }
}
fn register_claude_mcp_server(
    report: &mut suite_doctor::HealthReport,
    server: &str,
    command: String,
    args: &[&str],
) {
    let remove_args = vec![
        "mcp".to_string(),
        "remove".to_string(),
        server.to_string(),
        "-s".to_string(),
        "user".to_string(),
    ];
    let _ = run_claude_mcp_command(&remove_args);

    let mut add_args = vec![
        "mcp".to_string(),
        "add".to_string(),
        "-s".to_string(),
        "user".to_string(),
        server.to_string(),
        "--".to_string(),
        command.clone(),
    ];
    add_args.extend(args.iter().map(|arg| (*arg).to_string()));

    match run_claude_mcp_command(&add_args) {
        Ok(output) => {
            let mut finding = suite_doctor::Finding::pass(
                format!("install-claude-mcp-register-{server}"),
                format!("Claude Code MCP registered {server}: {command}"),
            );
            finding.detail = if output.trim().is_empty() {
                None
            } else {
                Some(output)
            };
            report.add(finding);
        }
        Err(e) => report.add(suite_doctor::Finding::fail(
            format!("install-claude-mcp-register-{server}"),
            format!("failed to register Claude Code MCP {server}"),
            e,
        )),
    }
}
pub(in crate::setup) fn add_claude_registration_checks(report: &mut suite_doctor::HealthReport) {
    match command_in_path("claude") {
        Ok(path) => report.add(suite_doctor::Finding::pass(
            "install-claude-code-cli",
            format!("Claude Code CLI available at {path}"),
        )),
        Err(e) => {
            report.add(suite_doctor::Finding::fail(
                "install-claude-code-cli",
                "Claude Code CLI is required for --register-claude",
                e,
            ));
            return;
        }
    }

    match command_in_path("ags") {
        Ok(ags_path) => register_claude_mcp_server(
            report,
            "ags",
            ags_path,
            &["mcp", "serve", "--transport", "stdio"],
        ),
        Err(e) => report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-ags",
            "cannot register AGS MCP because `ags` is not on PATH",
            e,
        )),
    }

    match command_in_path("codegraph") {
        Ok(codegraph_path) => {
            register_claude_mcp_server(report, "codegraph", codegraph_path, &["serve", "--mcp"])
        }
        Err(e) => report.add(suite_doctor::Finding::fail(
            "install-claude-mcp-register-codegraph",
            "cannot register codegraph MCP because `codegraph` is not on PATH",
            format!("install codegraph first, then rerun setup. {e}"),
        )),
    }
}
