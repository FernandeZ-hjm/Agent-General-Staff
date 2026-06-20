//! Shared project-onboarding templates (used by both `ags init` and the
//! `ags setup` global-entry plan). Kept module-neutral so the setup lifecycle
//! does not depend on the init lifecycle.

pub(crate) fn project_protocol_files() -> &'static [&'static str] {
    &[
        "agent-task-protocol.md",
        "task-card-template.md",
        "runtime-adapters.md",
        "task-routing.md",
        "project-profile.md",
        "context-memory.md",
        "cursor-skill-index.md",
    ]
}
pub(crate) fn portable_validate_script() -> String {
    "#!/usr/bin/env bash\n# AGS portable task-card validator wrapper.\nset -euo pipefail\nexec ags task validate \"$@\"\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_protocol_templates_include_skill_index() {
        assert!(
            project_protocol_files().contains(&"cursor-skill-index.md"),
            "ags init must copy the skill index referenced by task cards"
        );
    }
}
