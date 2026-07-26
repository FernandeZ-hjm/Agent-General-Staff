use super::*;

/// Whether AGS holds the canonical skill body: the resolved source dir contains
/// a `SKILL.md`. Read-only.
pub(in crate::skill_body::console) fn canonical_skill_present(
    repo_root: &Path,
    source: Option<&str>,
) -> bool {
    source
        .map(|s| resolve_source(repo_root, s).join("SKILL.md").is_file())
        .unwrap_or(false)
}
