use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateLane {
    Core,
    Runtime,
    Agents,
    Skills,
    Projects,
    Public,
}

impl UpdateLane {
    pub const fn all() -> [Self; 6] {
        [
            Self::Core,
            Self::Runtime,
            Self::Agents,
            Self::Skills,
            Self::Projects,
            Self::Public,
        ]
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Runtime => "runtime",
            Self::Agents => "agents",
            Self::Skills => "skills",
            Self::Projects => "projects",
            Self::Public => "public",
        }
    }

    pub const fn auto_executes_locally(self) -> bool {
        matches!(self, Self::Core | Self::Runtime | Self::Projects)
    }

    pub const fn risk_tier(self) -> &'static str {
        match self {
            Self::Core | Self::Public => "heavy",
            Self::Runtime | Self::Skills | Self::Projects => "medium",
            Self::Agents => "advice",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProjectInventory {
    pub registered: usize,
    pub present: usize,
    pub stale: usize,
    pub remote_backed: usize,
    pub reports: Vec<ProjectUpdate>,
    pub stale_reports: Vec<ProjectUpdate>,
    pub registry_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CapabilityInventory {
    pub summary: String,
    pub details: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ProjectUpdate {
    pub target: String,
    pub slug: String,
    pub status: String,
    pub drift: bool,
    pub changed_files: Vec<String>,
    pub unchanged_files: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateLanePlan {
    pub lane: UpdateLane,
    pub auto_executes: bool,
    pub advice_only: bool,
    pub risk_tier: String,
    pub summary: String,
    pub drift: Option<bool>,
    pub commands: Vec<String>,
    pub details: Vec<serde_json::Value>,
}

fn build_update_lane_plan(
    lane: UpdateLane,
    source_root: &Path,
    runtime_home: &Path,
    current_version: &str,
    projects: &ProjectInventory,
    capabilities: &CapabilityInventory,
) -> UpdateLanePlan {
    let auto = lane.auto_executes_locally();
    let mut details = Vec::new();
    let (summary, drift, commands): (String, Option<bool>, Vec<String>) = match lane {
        UpdateLane::Core => (
            format!("AGS kernel {current_version} — rebuild from the private source repo"),
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
        UpdateLane::Skills => {
            details.extend(capabilities.details.clone());
            (
                capabilities.summary.clone(),
                None,
                vec![
                    "ags onboarding plan".to_string(),
                    "ags capability snapshot --write --host <host>".to_string(),
                ],
            )
        }
        UpdateLane::Projects => {
            let drifted = projects
                .reports
                .iter()
                .filter(|report| report.drift)
                .count();
            let blocked = projects
                .reports
                .iter()
                .filter(|report| !report.blocked_reasons.is_empty())
                .count();
            details = projects
                .reports
                .iter()
                .map(|report| {
                    serde_json::json!({
                        "target": report.target,
                        "slug": report.slug,
                        "status": report.status,
                        "drift": report.drift,
                        "changed_files": report.changed_files,
                        "blocked_reasons": report.blocked_reasons,
                    })
                })
                .collect();
            (
                format!(
                    "managed projects: {} ({} present, {} drifted, {} blocked, {} stale, {} remote-backed) — refreshes AGS-owned files only, never auto-pushes",
                    projects.registered,
                    projects.present,
                    drifted,
                    blocked,
                    projects.stale,
                    projects.remote_backed
                ),
                Some(drifted > 0 || blocked > 0 || projects.stale > 0),
                vec!["ags update apply --lane projects --apply".to_string()],
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
        details,
    }
}
pub fn build_all_update_lanes(
    source_root: &Path,
    runtime_home: &Path,
    current_version: &str,
    projects: &ProjectInventory,
    capabilities: &CapabilityInventory,
) -> Vec<UpdateLanePlan> {
    UpdateLane::all()
        .iter()
        .map(|lane| {
            build_update_lane_plan(
                *lane,
                source_root,
                runtime_home,
                current_version,
                projects,
                capabilities,
            )
        })
        .collect()
}
pub fn update_lane_json(p: &UpdateLanePlan) -> serde_json::Value {
    serde_json::json!({
        "lane": p.lane.id(),
        "auto_executes_locally": p.auto_executes,
        "advice_only": p.advice_only,
        "risk_tier": p.risk_tier,
        "summary": p.summary,
        "drift": p.drift,
        "commands": p.commands,
        "details": p.details,
    })
}

#[cfg(test)]
mod update_lane_tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_home(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-xplat-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn update_lanes_mark_core_runtime_and_projects_auto() {
        assert!(UpdateLane::Core.auto_executes_locally());
        assert!(UpdateLane::Runtime.auto_executes_locally());
        assert!(UpdateLane::Projects.auto_executes_locally());
        assert!(!UpdateLane::Agents.auto_executes_locally());
        assert!(!UpdateLane::Public.auto_executes_locally());
        assert_eq!(UpdateLane::Core.risk_tier(), "heavy");
        assert_eq!(UpdateLane::Public.risk_tier(), "heavy");
        assert_eq!(UpdateLane::Runtime.risk_tier(), "medium");
        assert_eq!(UpdateLane::Agents.risk_tier(), "advice");
        assert_eq!(UpdateLane::Projects.risk_tier(), "medium");
    }

    #[test]
    fn build_all_update_lanes_has_six_with_flags() {
        let src = temp_home("upd-src");
        let home = temp_home("upd-home");
        let lanes = build_all_update_lanes(
            &src,
            &home,
            env!("CARGO_PKG_VERSION"),
            &ProjectInventory::default(),
            &CapabilityInventory {
                summary: "third-party capability registry unavailable".to_string(),
                details: Vec::new(),
            },
        );
        assert_eq!(lanes.len(), 6);
        let core = lanes.iter().find(|l| l.lane == UpdateLane::Core).unwrap();
        assert!(core.auto_executes);
        let agents = lanes.iter().find(|l| l.lane == UpdateLane::Agents).unwrap();
        assert!(agents.advice_only);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&home);
    }
}
