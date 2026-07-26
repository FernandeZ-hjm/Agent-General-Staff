//! Read-only update planning and verification decisions.

use super::{UpdateLane, UpdateLanePlan};

pub fn select_lanes(
    lanes: Vec<UpdateLanePlan>,
    selected: Option<UpdateLane>,
) -> Vec<UpdateLanePlan> {
    lanes
        .into_iter()
        .filter(|plan| selected.map(|lane| lane == plan.lane).unwrap_or(true))
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct VerificationFacts {
    pub runtime_present: bool,
    pub auth_boundary_clean: bool,
    pub skill_snapshot_current: bool,
    pub projects_drift: bool,
}

impl VerificationFacts {
    pub const fn drift(self) -> bool {
        !self.runtime_present
            || !self.auth_boundary_clean
            || !self.skill_snapshot_current
            || self.projects_drift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_drift_requires_every_update_fact_to_be_current() {
        let clean = VerificationFacts {
            runtime_present: true,
            auth_boundary_clean: true,
            skill_snapshot_current: true,
            projects_drift: false,
        };
        assert!(!clean.drift());
        assert!(VerificationFacts {
            runtime_present: false,
            ..clean
        }
        .drift());
        assert!(VerificationFacts {
            auth_boundary_clean: false,
            ..clean
        }
        .drift());
        assert!(VerificationFacts {
            skill_snapshot_current: false,
            ..clean
        }
        .drift());
        assert!(VerificationFacts {
            projects_drift: true,
            ..clean
        }
        .drift());
    }
}
