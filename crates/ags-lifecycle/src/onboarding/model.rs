use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentState {
    Absent,
    InstalledNotVisible,
    VisibleNotReady,
    ActiveReady,
    UpdateAvailable,
    BlockedUntrustedSource,
    BlockedMissingIntegrity,
    UnsupportedHost,
}

impl ComponentState {
    pub fn is_ready(self) -> bool {
        self == Self::ActiveReady
    }

    pub fn is_blocked(self) -> bool {
        matches!(
            self,
            Self::BlockedUntrustedSource | Self::BlockedMissingIntegrity | Self::UnsupportedHost
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OnboardingAction {
    ProjectInit {
        target: String,
    },
    RegisterAgsMcp {
        registrar: String,
        executable: String,
    },
    RegisterNpmMcp {
        registrar: String,
        server_name: String,
        package: String,
        integrity: String,
    },
    RegisterCommandMcp {
        registrar: String,
        server_name: String,
        command: String,
        args: Vec<String>,
    },
    InstallNpmCli {
        package: String,
        integrity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingItem {
    pub id: String,
    pub category: String,
    pub required: bool,
    pub state: ComponentState,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookReadiness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<OnboardingAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingReadiness {
    pub route_state: String,
    pub metadata_complete: bool,
    pub semantic_probe: String,
    pub boundary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookReadiness {
    pub host: String,
    pub events: Vec<String>,
    pub config_present: bool,
    pub health_probe: String,
    pub event_probe: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingPlan {
    pub schema_version: String,
    pub profile: String,
    pub host: String,
    pub target: String,
    pub manifest_source: String,
    pub manifest_hash: String,
    pub bootstrap_required: bool,
    pub ready: bool,
    pub plan_hash: String,
    pub items: Vec<OnboardingItem>,
    pub excluded_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionExecution {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}
