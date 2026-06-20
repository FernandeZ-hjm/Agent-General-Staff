use crate::cli::UpdateLane;
use crate::context::AGS_VERSION;
use crate::managed_projects;
use std::path::Path;

#[derive(Debug, Clone)]
pub(in crate::update) struct UpdateLanePlan {
    pub(crate) lane: UpdateLane,
    pub(crate) auto_executes: bool,
    pub(crate) advice_only: bool,
    pub(crate) risk_tier: String,
    pub(crate) summary: String,
    pub(crate) drift: Option<bool>,
    pub(crate) commands: Vec<String>,
}
fn build_update_lane_plan(
    lane: UpdateLane,
    source_root: &Path,
    runtime_home: &Path,
) -> UpdateLanePlan {
    let auto = lane.auto_executes_locally();
    let (summary, drift, commands): (String, Option<bool>, Vec<String>) = match lane {
        UpdateLane::Core => (
            format!("AGS kernel {AGS_VERSION} — rebuild from the private source repo"),
            None,
            vec![
                format!("git -C \"{}\" pull --ff-only", source_root.display()),
                format!(
                    "cargo build --release --manifest-path \"{}\"",
                    source_root.join("Cargo.toml").display()
                ),
            ],
        ),
        UpdateLane::Runtime => {
            let present = runtime_home.is_dir();
            (
                format!("runtime snippets/templates at {}", runtime_home.display()),
                Some(!present),
                vec!["ags setup --yes".to_string()],
            )
        }
        UpdateLane::Agents => (
            "Agent host AGS MCP onboarding (advise-only)".to_string(),
            None,
            vec!["ags agents govern".to_string()],
        ),
        UpdateLane::Skills => (
            "skill thin-index distribution across hosts".to_string(),
            None,
            vec!["ags skill sync --apply".to_string()],
        ),
        UpdateLane::Projects => {
            let reg = managed_projects::load(&managed_projects::registry_path(runtime_home))
                .unwrap_or_default();
            let (existing, stale) = managed_projects::partition_existing(&reg);
            let remote = reg
                .projects
                .iter()
                .filter(|p| managed_projects::is_remote_backed(p))
                .count();
            (
                format!(
                    "managed projects: {} ({} present, {} stale, {} remote-backed) — local plan only, never auto-push",
                    reg.projects.len(),
                    existing.len(),
                    stale.len(),
                    remote
                ),
                None,
                vec!["ags doctor --scope project (per managed project)".to_string()],
            )
        }
        UpdateLane::Public => (
            "public-safe projection (plan/verify only; never push)".to_string(),
            None,
            vec!["review public boundary; AGS never publishes by default".to_string()],
        ),
    };
    UpdateLanePlan {
        lane,
        auto_executes: auto,
        advice_only: !auto,
        risk_tier: lane.risk_tier().to_string(),
        summary,
        drift,
        commands,
    }
}
pub(in crate::update) fn build_all_update_lanes(
    source_root: &Path,
    runtime_home: &Path,
) -> Vec<UpdateLanePlan> {
    UpdateLane::all()
        .iter()
        .map(|l| build_update_lane_plan(*l, source_root, runtime_home))
        .collect()
}
pub(in crate::update) fn update_lane_json(p: &UpdateLanePlan) -> serde_json::Value {
    serde_json::json!({
        "lane": p.lane.id(),
        "auto_executes_locally": p.auto_executes,
        "advice_only": p.advice_only,
        "risk_tier": p.risk_tier,
        "summary": p.summary,
        "drift": p.drift,
        "commands": p.commands,
    })
}

#[cfg(test)]
mod update_lane_tests {
    use super::*;
    use crate::cli::UpdateLane;
    use std::path::PathBuf;

    fn temp_home(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-xplat-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn update_lanes_mark_only_core_runtime_auto() {
        assert!(UpdateLane::Core.auto_executes_locally());
        assert!(UpdateLane::Runtime.auto_executes_locally());
        assert!(!UpdateLane::Agents.auto_executes_locally());
        assert!(!UpdateLane::Projects.auto_executes_locally());
        assert!(!UpdateLane::Public.auto_executes_locally());
        assert_eq!(UpdateLane::Core.risk_tier(), "heavy");
        assert_eq!(UpdateLane::Public.risk_tier(), "heavy");
        assert_eq!(UpdateLane::Runtime.risk_tier(), "medium");
        assert_eq!(UpdateLane::Agents.risk_tier(), "advice");
    }

    #[test]
    fn build_all_update_lanes_has_six_with_flags() {
        let src = temp_home("upd-src");
        let home = temp_home("upd-home");
        let lanes = build_all_update_lanes(&src, &home);
        assert_eq!(lanes.len(), 6);
        let core = lanes.iter().find(|l| l.lane == UpdateLane::Core).unwrap();
        assert!(core.auto_executes);
        let agents = lanes.iter().find(|l| l.lane == UpdateLane::Agents).unwrap();
        assert!(agents.advice_only);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&home);
    }
}
