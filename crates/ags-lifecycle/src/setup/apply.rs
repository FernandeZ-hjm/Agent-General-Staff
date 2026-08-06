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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::codex_skill_thin_index_ancestor;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    fn tmp(tag: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("ags-setup-apply-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[cfg(unix)]
    fn symlink_dir(src: &Path, dst: &Path) {
        std::os::unix::fs::symlink(src, dst).unwrap();
    }

    /// Regression: once installation has made `~/.codex/skills/<name>` a
    /// thin-index symlink to the canonical repo skill, setup must not write
    /// `SKILL.md` through that symlink and mutate the canonical body.
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

        assert_eq!(
            codex_skill_thin_index_ancestor(&host.join("ags-setup/SKILL.md")),
            Some(host.join("ags-setup"))
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
            "canonical\n"
        );
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

        assert_eq!(
            codex_skill_thin_index_ancestor(&host.join("ags-setup/agents/openai.yaml")),
            Some(host.join("ags-setup"))
        );
        assert_eq!(
            std::fs::read_to_string(canonical.join("agents/openai.yaml")).unwrap(),
            "canonical-meta\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
