//! Closed typed handoff wire model.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const SCHEMA_VERSION: &str = "ags://schema/contract/v2/task-contract";

pub const HANDOFF_CONTRACT_SCHEMA_VERSION: &str = "ags://schema/contract/v2/handoff-contract";

/// Structured origin of a newly compiled task card.
///
/// Host Plan mode is an alternative structured handoff signal: its final,
/// decision-complete artifact is the canonical task card. It is not the task
/// card's `Execution mode` and grants no execution authority by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffSource {
    ExplicitHandoff,
    HostPlanMode,
}

impl HandoffSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitHandoff => "explicit-handoff",
            Self::HostPlanMode => "host-plan-mode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskLevel {
    Light,
    Medium,
    Heavy,
}

impl TaskLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Heavy => "Heavy",
        }
    }
}

/// Typed, closed handoff seam for 0.3.0. `task_level` and `task` are mandatory;
/// callers cannot omit the level and ask the compiler to infer it from prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffContract {
    pub schema_version: String,
    pub task_level: TaskLevel,
    pub task: String,
    #[serde(default)]
    pub fields: HashMap<String, String>,
}
