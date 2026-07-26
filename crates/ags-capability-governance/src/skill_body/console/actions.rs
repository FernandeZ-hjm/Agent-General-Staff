#[allow(unused_imports)]
use super::model::*;
// ── Action vocabulary ──────────────────────────────────────────────────────────

/// Management verbs the console understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleAction {
    Adopt,
    Update,
    Remove,
    Uninstall,
    Repair,
    Verify,
}

impl ConsoleAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "adopt" => Some(Self::Adopt),
            "update" => Some(Self::Update),
            "remove" => Some(Self::Remove),
            "uninstall" => Some(Self::Uninstall),
            "repair" => Some(Self::Repair),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Adopt => "adopt",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::Uninstall => "uninstall",
            Self::Repair => "repair",
            Self::Verify => "verify",
        }
    }
}

/// All console action keywords (for CLI value parsing).
pub const CONSOLE_ACTIONS: &[&str] =
    &["adopt", "update", "remove", "uninstall", "repair", "verify"];

/// Default management actions offered for a capability in the inventory.
pub(super) fn actions_for(kind: &ManagedKind, status: &ManagedStatus) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    match status {
        ManagedStatus::Discovered | ManagedStatus::Unmanaged | ManagedStatus::Ignored => {
            a.push("adopt".to_string());
        }
        ManagedStatus::SuiteManaged | ManagedStatus::Governed => {
            a.push("update".to_string());
            a.push("repair".to_string());
            a.push("remove".to_string());
        }
        // The AGS host initialization adapter cannot be adopted/removed here.
        ManagedStatus::SuiteInterface => {}
        // Host-system / project-local skills are recognized READ-ONLY: AGS never
        // holds the body, so the console offers no adopt/relink — making them
        // routable is a deliberate manifest adoption edit, not a console action.
        ManagedStatus::HostSystem | ManagedStatus::ProjectLocal => {}
        // Route targets are routing-only — no adopt / update / relink / verify.
        ManagedStatus::RouteTarget => return Vec::new(),
    }
    if matches!(kind, ManagedKind::Skill)
        && matches!(
            status,
            ManagedStatus::SuiteManaged | ManagedStatus::Discovered
        )
    {
        a.push("uninstall".to_string());
    }
    a.push("verify".to_string());
    a
}
