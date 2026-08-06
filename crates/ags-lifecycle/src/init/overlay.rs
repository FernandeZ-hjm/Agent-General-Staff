use super::InitFile;
use std::path::{Path, PathBuf};

const OVERLAY_BLOCK_BEGIN: &str = "# >>> AGS local governance overlay (managed by `ags init`) >>>";
const OVERLAY_BLOCK_END: &str = "# <<< AGS local governance overlay (managed by `ags init`) <<<";
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayMode {
    /// Default: AGS files are added to `.git/info/exclude` (local, uncommitted).
    Local,
    /// Opt-in: AGS files are left tracked/committed (shared with the repo).
    Shared,
}
impl OverlayMode {
    pub(crate) fn parse(value: &str) -> OverlayMode {
        match value {
            "shared" => OverlayMode::Shared,
            _ => OverlayMode::Local,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            OverlayMode::Local => "local",
            OverlayMode::Shared => "shared",
        }
    }
}
/// Repo-root-anchored gitignore entries for every AGS overlay file that lives
/// inside the target repository. Memory-capsule files (under `$HOME`) and any
/// path outside the target are skipped. Result is sorted and de-duplicated.
fn overlay_exclude_entries(target: &Path, files: &[InitFile]) -> Vec<String> {
    overlay_exclude_paths(target, files.iter().map(|file| file.path.as_path()))
}

fn overlay_exclude_paths<'a>(target: &Path, paths: impl Iterator<Item = &'a Path>) -> Vec<String> {
    let mut entries = Vec::new();
    for path in paths {
        if let Ok(rel) = path.strip_prefix(target) {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if !rel.is_empty() {
                entries.push(format!("/{rel}"));
            }
        }
    }
    entries.sort();
    entries.dedup();
    entries
}
/// Result of merging the AGS-managed overlay block into a `.git/info/exclude`
/// body.
struct OverlayExcludeMerge {
    content: String,
    /// True when the existing body contained an unpaired/malformed AGS marker
    /// (a begin without a matching end, or a stray end). In that case the
    /// existing content is preserved verbatim and a fresh block is appended;
    /// no user lines are ever deleted.
    had_malformed_markers: bool,
}
/// Insert or replace the AGS-managed overlay block in an existing
/// `.git/info/exclude` body. Only a **well-formed** managed block — a `BEGIN`
/// line followed by a matching `END` line with no intervening `BEGIN` — is
/// stripped and replaced. Unpaired markers (a begin without an end, or a stray
/// end) are treated as ordinary content and preserved, so user ignore lines
/// after a truncated block are never silently dropped; a fresh block is
/// appended instead and `had_malformed_markers` is set. Idempotent:
/// re-running with the same entries yields byte-identical output, even when
/// stray markers remain.
fn merge_overlay_exclude(existing: &str, entries: &[String]) -> OverlayExcludeMerge {
    let lines: Vec<&str> = existing.lines().collect();
    let mut marker_depth = 0usize;
    let mut had_malformed_markers = false;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed == OVERLAY_BLOCK_BEGIN {
            if marker_depth != 0 {
                had_malformed_markers = true;
                break;
            }
            marker_depth = 1;
        } else if trimmed == OVERLAY_BLOCK_END {
            if marker_depth == 0 {
                had_malformed_markers = true;
                break;
            }
            marker_depth = 0;
        }
    }
    if marker_depth != 0 {
        had_malformed_markers = true;
    }

    if had_malformed_markers {
        let mut content = existing.trim_end_matches('\n').to_string();
        if !entries.is_empty() && !ends_with_overlay_block(&content, entries) {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            push_overlay_block(&mut content, entries);
        } else if !content.is_empty() {
            content.push('\n');
        }
        return OverlayExcludeMerge {
            content,
            had_malformed_markers: true,
        };
    }

    let mut kept: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == OVERLAY_BLOCK_BEGIN {
            let mut j = i + 1;
            while j < lines.len() {
                let inner = lines[j].trim();
                if inner == OVERLAY_BLOCK_END {
                    break;
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        kept.push(lines[i]);
        i += 1;
    }
    while matches!(kept.last(), Some(l) if l.trim().is_empty()) {
        kept.pop();
    }

    let mut content = String::new();
    if !kept.is_empty() {
        content.push_str(&kept.join("\n"));
        content.push('\n');
    }
    if !entries.is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        push_overlay_block(&mut content, entries);
    }
    OverlayExcludeMerge {
        content,
        had_malformed_markers: false,
    }
}
fn push_overlay_block(content: &mut String, entries: &[String]) {
    content.push_str(OVERLAY_BLOCK_BEGIN);
    content.push('\n');
    for entry in entries {
        content.push_str(entry);
        content.push('\n');
    }
    content.push_str(OVERLAY_BLOCK_END);
    content.push('\n');
}
fn ends_with_overlay_block(content: &str, entries: &[String]) -> bool {
    let mut expected = String::new();
    push_overlay_block(&mut expected, entries);
    content.trim_end_matches('\n') == expected.trim_end_matches('\n')
        || content
            .trim_end_matches('\n')
            .ends_with(&format!("\n\n{}", expected.trim_end_matches('\n')))
}
fn git_command(target: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(target);
    cmd
}
fn git_is_repo(target: &Path) -> bool {
    git_command(target)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}
fn git_info_exclude_path(target: &Path) -> Option<PathBuf> {
    let out = git_command(target)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(&raw);
    Some(if path.is_absolute() {
        path
    } else {
        target.join(path)
    })
}

pub fn local_overlay_active(target: &Path) -> bool {
    git_info_exclude_path(target)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|body| body.contains("AGS local governance overlay"))
}

pub fn git_tracked_set(target: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Ok(out) = git_command(target).args(["ls-files"]).output() {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let line = line.trim();
                if !line.is_empty() {
                    set.insert(line.to_string());
                }
            }
        }
    }
    set
}
pub struct OverlayPlan {
    mode: OverlayMode,
    is_git_repo: bool,
    exclude_path: Option<PathBuf>,
    entries: Vec<String>,
    tracked_entries: Vec<String>,
    warnings: Vec<String>,
}
/// Resolve the local overlay plan for the given target and AGS install files.
/// Read-only: it queries git state but performs no writes.
pub fn compute_overlay_plan(target: &Path, files: &[InitFile], mode: OverlayMode) -> OverlayPlan {
    compute_overlay_plan_with_paths(target, files, &[], mode)
}

pub fn compute_overlay_plan_with_paths(
    target: &Path,
    files: &[InitFile],
    extra_paths: &[PathBuf],
    mode: OverlayMode,
) -> OverlayPlan {
    let mut entries = overlay_exclude_entries(target, files);
    entries.extend(overlay_exclude_paths(
        target,
        extra_paths.iter().map(PathBuf::as_path),
    ));
    entries.sort();
    entries.dedup();
    let mut warnings = Vec::new();
    let is_git_repo = git_is_repo(target);

    if mode == OverlayMode::Shared {
        return OverlayPlan {
            mode,
            is_git_repo,
            exclude_path: None,
            entries,
            tracked_entries: Vec::new(),
            warnings,
        };
    }

    if !is_git_repo {
        return OverlayPlan {
            mode,
            is_git_repo,
            exclude_path: None,
            entries,
            tracked_entries: Vec::new(),
            warnings,
        };
    }

    let exclude_path = git_info_exclude_path(target);
    let tracked = git_tracked_set(target);
    let tracked_entries: Vec<String> = entries
        .iter()
        .filter(|e| tracked.contains(e.trim_start_matches('/')))
        .cloned()
        .collect();

    if !tracked_entries.is_empty() {
        warnings.push(format!(
            "{} overlay file(s) are already tracked by git; `.git/info/exclude` cannot hide tracked files: {}",
            tracked_entries.len(),
            tracked_entries.join(", ")
        ));
    }

    OverlayPlan {
        mode,
        is_git_repo,
        exclude_path,
        entries,
        tracked_entries,
        warnings,
    }
}
/// Apply the local overlay by writing the managed block into
/// `.git/info/exclude`. Returns findings.
pub fn apply_overlay(plan: &OverlayPlan) -> Vec<super::InitFinding> {
    use super::InitFinding as Finding;
    let mut findings = Vec::new();

    if plan.mode == OverlayMode::Shared {
        findings.push(Finding::info(
            "overlay-mode",
            "overlay mode: shared — AGS governance files are left tracked/committed",
        ));
        return findings;
    }

    if !plan.is_git_repo {
        findings.push(Finding::info(
            "overlay-not-applicable",
            "target is not a Git worktree; local .git/info/exclude overlay is not applicable",
        ));
        return findings;
    }

    let Some(exclude_path) = &plan.exclude_path else {
        findings.push(Finding::warn(
            "overlay-exclude",
            "could not resolve .git/info/exclude path",
            "AGS local overlay not written",
        ));
        return findings;
    };

    let existing = std::fs::read_to_string(exclude_path).unwrap_or_default();
    let merge = merge_overlay_exclude(&existing, &plan.entries);
    if merge.had_malformed_markers {
        findings.push(Finding::warn(
            "overlay-exclude-malformed",
            format!(
                "unpaired AGS overlay marker(s) in {}; preserved existing content and appended a fresh managed block (no user lines deleted) — remove stray markers manually",
                exclude_path.display()
            ),
            "malformed managed block detected",
        ));
    }
    if merge.content == existing {
        findings.push(Finding::pass(
            "overlay-exclude",
            format!(
                "unchanged: {} ({} overlay entries)",
                exclude_path.display(),
                plan.entries.len()
            ),
        ));
    } else {
        if let Some(parent) = exclude_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(exclude_path, &merge.content) {
            Ok(()) => findings.push(Finding::pass(
                "overlay-exclude",
                format!(
                    "wrote {} overlay entries to {}",
                    plan.entries.len(),
                    exclude_path.display()
                ),
            )),
            Err(e) => findings.push(Finding::fail(
                "overlay-exclude",
                format!("failed to write {}", exclude_path.display()),
                e.to_string(),
            )),
        }
    }

    for warning in &plan.warnings {
        findings.push(Finding::warn(
            "overlay-note",
            warning.clone(),
            "review overlay state",
        ));
    }
    findings
}

pub fn untrack_pure_overlay_files(
    target: &Path,
    candidates: &[(PathBuf, Vec<u8>)],
) -> Vec<super::InitFinding> {
    use super::InitFinding as Finding;
    let mut findings = Vec::new();
    let tracked = git_tracked_set(target);
    for (path, expected) in candidates {
        let Ok(relative) = path.strip_prefix(target) else {
            continue;
        };
        if !tracked.contains(&relative.to_string_lossy().replace('\\', "/")) {
            continue;
        }
        let observed = std::fs::read(path).ok();
        if observed.as_deref() != Some(expected.as_slice()) {
            findings.push(Finding::warn(
                "overlay-mixed-ownership",
                format!(
                    "tracked file kept because it is not pure AGS output: {}",
                    path.display()
                ),
                "preserved project-owned or mixed content",
            ));
            continue;
        }
        match git_command(target)
            .args(["rm", "--cached", "--ignore-unmatch", "--"])
            .arg(relative)
            .output()
        {
            Ok(output) if output.status.success() => findings.push(Finding::pass(
                "overlay-untrack-pure",
                format!("removed pure AGS output from Git index: {}", path.display()),
            )),
            Ok(output) => findings.push(Finding::fail(
                "overlay-untrack-pure",
                format!("could not remove {} from Git index", path.display()),
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            )),
            Err(error) => findings.push(Finding::fail(
                "overlay-untrack-pure",
                format!("could not inspect Git index for {}", path.display()),
                error.to_string(),
            )),
        }
    }
    findings
}
pub fn render_overlay_text(plan: &OverlayPlan) -> String {
    let mut lines = vec![
        "Overlay:".to_string(),
        format!("  Mode:    {}", plan.mode.as_str()),
        format!("  Git:     {}", if plan.is_git_repo { "yes" } else { "no" }),
    ];
    if plan.mode == OverlayMode::Local && !plan.is_git_repo {
        lines.push(
            "  Applicability: not-applicable (no Git worktree; no .git created).".to_string(),
        );
    }
    if plan.mode == OverlayMode::Local && plan.is_git_repo {
        if let Some(path) = &plan.exclude_path {
            lines.push(format!("  Exclude: {}", path.display()));
        }
        lines.push(format!(
            "  Entries: {} overlay path(s) git-ignored locally",
            plan.entries.len()
        ));
        for entry in &plan.entries {
            lines.push(format!("    - {entry}"));
        }
    } else if plan.mode == OverlayMode::Shared {
        lines.push("  AGS governance files are tracked/committed (shared).".to_string());
    }
    for warning in &plan.warnings {
        lines.push(format!("  ! {warning}"));
    }
    lines.join("\n")
}
pub fn overlay_json(plan: &OverlayPlan) -> serde_json::Value {
    serde_json::json!({
        "mode": plan.mode.as_str(),
        "is_git_repo": plan.is_git_repo,
        "exclude_path": plan.exclude_path.as_ref().map(|p| p.to_string_lossy()),
        "entries": plan.entries,
        "tracked_entries": plan.tracked_entries,
        "warnings": plan.warnings,
        "applicability": if plan.mode == OverlayMode::Local && !plan.is_git_repo {
            "not-applicable"
        } else {
            "applicable"
        },
    })
}
#[cfg(test)]
mod overlay_tests {
    use super::{
        apply_overlay, compute_overlay_plan, merge_overlay_exclude, overlay_exclude_entries,
        untrack_pure_overlay_files, InitFile, OverlayMode, OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_END,
    };
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn mk(path: PathBuf) -> InitFile {
        InitFile {
            path,
            description: String::new(),
            content: String::new(),
            mode: None,
        }
    }

    #[test]
    fn overlay_entries_are_anchored_and_skip_paths_outside_repo() {
        let target = Path::new("/tmp/ags-repo");
        let files = vec![
            mk(target.join("AGENTS.md")),
            mk(target.join("protocol/agent-task-protocol.md")),
            mk(PathBuf::from("/home/u/.agents/memory/x/context-capsule.md")),
        ];
        let entries = overlay_exclude_entries(target, &files);
        assert_eq!(
            entries,
            vec![
                "/AGENTS.md".to_string(),
                "/protocol/agent-task-protocol.md".to_string(),
            ]
        );
    }

    #[test]
    fn merge_overlay_exclude_is_idempotent_and_preserves_user_lines() {
        let entries = vec!["/AGENTS.md".to_string(), "/WORKSPACE.md".to_string()];
        let once = merge_overlay_exclude("build/\n*.log\n", &entries);
        assert!(!once.had_malformed_markers);
        let once = once.content;
        assert!(once.contains("build/"));
        assert!(once.contains("*.log"));
        assert!(once.contains("/AGENTS.md"));
        assert!(once.contains("/WORKSPACE.md"));
        assert!(once.contains(OVERLAY_BLOCK_BEGIN));
        assert!(once.contains(OVERLAY_BLOCK_END));

        let twice = merge_overlay_exclude(&once, &entries).content;
        assert_eq!(once, twice, "overlay merge must be idempotent");
        assert_eq!(twice.matches(OVERLAY_BLOCK_BEGIN).count(), 1);
        assert_eq!(twice.matches(OVERLAY_BLOCK_END).count(), 1);
    }

    #[test]
    fn merge_overlay_exclude_empty_entries_removes_block() {
        let with = merge_overlay_exclude("user.txt\n", &["/AGENTS.md".to_string()]).content;
        assert!(with.contains(OVERLAY_BLOCK_BEGIN));
        let without = merge_overlay_exclude(&with, &[]).content;
        assert!(!without.contains(OVERLAY_BLOCK_BEGIN));
        assert!(!without.contains(OVERLAY_BLOCK_END));
        assert!(without.contains("user.txt"));
    }

    #[test]
    fn malformed_overlay_markers_preserve_user_content_and_remain_idempotent() {
        let cases = [
            (
                format!(
                    "secret.key\n{}\n/AGENTS.md\nkeep-me.txt\nbuild/\n",
                    OVERLAY_BLOCK_BEGIN
                ),
                vec!["secret.key", "/AGENTS.md", "keep-me.txt", "build/"],
            ),
            (
                format!("a.txt\n{}\nb.txt\n", OVERLAY_BLOCK_END),
                vec!["a.txt", "b.txt"],
            ),
            (
                format!(
                    "before\n{}\nouter-user\n{}\ninner-user-should-stay\n{}\nafter\n",
                    OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_BEGIN, OVERLAY_BLOCK_END
                ),
                vec!["before", "outer-user", "inner-user-should-stay", "after"],
            ),
        ];
        let entries = vec!["/WORKSPACE.md".to_string()];

        for (malformed, preserved_lines) in cases {
            let merged = merge_overlay_exclude(&malformed, &entries);
            assert!(merged.had_malformed_markers);
            for line in preserved_lines {
                assert!(
                    merged.content.contains(line),
                    "line {line:?} must be preserved, got:\n{}",
                    merged.content
                );
            }
            assert!(merged.content.contains("/WORKSPACE.md"));
            assert!(merged.content.contains(OVERLAY_BLOCK_END));

            let again = merge_overlay_exclude(&merged.content, &entries);
            assert_eq!(
                merged.content, again.content,
                "malformed-input merge must remain idempotent"
            );
            assert_eq!(again.content.matches("/WORKSPACE.md").count(), 1);
        }
    }

    fn unique_repo(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ags-overlay-{name}-{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["init", "-q"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git init failed");
        dir
    }

    #[test]
    fn local_overlay_hides_files_and_is_idempotent() {
        let target = unique_repo("local");
        std::fs::write(target.join("WORKSPACE.md"), "ags").unwrap();
        std::fs::write(target.join("AGENTS.md"), "ags").unwrap();
        let files = vec![
            mk(target.join("WORKSPACE.md")),
            mk(target.join("AGENTS.md")),
        ];
        assert!(std::process::Command::new("git")
            .current_dir(&target)
            .args(["add", "-f", "WORKSPACE.md"])
            .status()
            .unwrap()
            .success());

        let plan = compute_overlay_plan(&target, &files, OverlayMode::Local);
        assert!(plan.is_git_repo);
        let untracked =
            untrack_pure_overlay_files(&target, &[(target.join("WORKSPACE.md"), b"ags".to_vec())]);
        assert!(untracked
            .iter()
            .any(|finding| finding.check_name == "overlay-untrack-pure"));
        let _ = apply_overlay(&plan);

        let exclude = plan.exclude_path.clone().unwrap();
        let body = std::fs::read_to_string(&exclude).unwrap();
        assert!(body.contains("/WORKSPACE.md"));
        assert!(body.contains("/AGENTS.md"));

        let status = std::process::Command::new("git")
            .current_dir(&target)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&status.stdout).trim().is_empty(),
            "git status should be clean, got: {}",
            String::from_utf8_lossy(&status.stdout)
        );

        // Re-running must not change the exclude file.
        let plan2 = compute_overlay_plan(&target, &files, OverlayMode::Local);
        let _ = apply_overlay(&plan2);
        let body2 = std::fs::read_to_string(&exclude).unwrap();
        assert_eq!(body, body2, "second apply must be idempotent");

        let _ = std::fs::remove_dir_all(&target);
    }
}
