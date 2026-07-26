use super::*;

pub(in crate::skill_body::console) fn summarize(
    caps: &[ManagedCapability],
) -> ManagedInventorySummary {
    let claude_visible = caps
        .iter()
        .filter(|c| {
            c.host_visibility
                .iter()
                .any(|v| v.host == "claude-code" && v.status == HostVisibilityStatus::Visible)
        })
        .count();
    ManagedInventorySummary {
        total: caps.len(),
        skills: caps.iter().filter(|c| c.kind == ManagedKind::Skill).count(),
        mcps: caps.iter().filter(|c| c.kind == ManagedKind::Mcp).count(),
        suite_interfaces: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::SuiteInterface)
            .count(),
        cli_backed: caps
            .iter()
            .filter(|c| c.kind == ManagedKind::CliBacked)
            .count(),
        canonical_present: caps.iter().filter(|c| c.canonical_present).count(),
        claude_visible,
        risk_flagged: caps.iter().filter(|c| !c.risk_notes.is_empty()).count(),
        routing_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Routable)
            })
            .count(),
        routing_not_routable: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::NotRoutable)
            })
            .count(),
        routing_retired: caps
            .iter()
            .filter(|c| {
                c.routing
                    .as_ref()
                    .is_some_and(|r| r.route_state == RouteState::Retired)
            })
            .count(),
        routing_uncovered: caps
            .iter()
            .filter(|c| {
                matches!(
                    c.managed_status,
                    ManagedStatus::SuiteManaged | ManagedStatus::Governed
                ) && c.routing.is_none()
            })
            .count(),
    }
}

/// Deterministic content hash of the machine-local capability snapshot. Hashes a
/// CANONICAL projection (sorted `name|kind|managed_status|registry|route_state|
/// canonical|host=visibility…|health` lines) with FNV-1a — dependency-free and
/// stable across runs for identical machine state. Used as the task-card snapshot
/// attestation token. Contains capability NAMES + statuses only — NO absolute
/// paths — so it is safe to record in a (machine-local) snapshot or a task card.
pub fn inventory_snapshot_hash(inv: &ManagedInventoryResult) -> String {
    fn vis_str(s: &HostVisibilityStatus) -> &'static str {
        match s {
            HostVisibilityStatus::Visible => "visible",
            HostVisibilityStatus::NotVisible => "not-visible",
            HostVisibilityStatus::Degraded => "degraded",
            HostVisibilityStatus::Unsupported => "unsupported",
            HostVisibilityStatus::Deferred => "deferred",
        }
    }
    fn health_str(h: &HealthStatus) -> &'static str {
        match h {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unknown => "unknown",
            HealthStatus::Unhealthy => "unhealthy",
        }
    }
    fn kind_str(k: &ManagedKind) -> &'static str {
        match k {
            ManagedKind::Skill => "skill",
            ManagedKind::Mcp => "mcp",
            ManagedKind::SuiteInterface => "suite-interface",
            ManagedKind::CliBacked => "cli-backed",
        }
    }
    let route_str = |c: &ManagedCapability| -> &'static str {
        match c.routing.as_ref().map(|r| r.route_state) {
            Some(RouteState::Routable) => "routable",
            Some(RouteState::NotRoutable) => "not-routable",
            Some(RouteState::Retired) => "retired",
            None => "none",
        }
    };
    let mut lines: Vec<String> = inv
        .capabilities
        .iter()
        .map(|c| {
            let mut vis: Vec<String> = c
                .host_visibility
                .iter()
                .map(|v| format!("{}={}", v.host, vis_str(&v.status)))
                .collect();
            vis.sort();
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                c.name,
                kind_str(&c.kind),
                managed_status_str(&c.managed_status),
                if matches!(c.registry_status, RegistryStatus::Registered) {
                    "registered"
                } else {
                    "not-registered"
                },
                route_str(c),
                c.canonical_present,
                vis.join(","),
                health_str(&c.health_status),
            )
        })
        .collect();
    lines.sort();
    let joined = lines.join("\n");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in joined.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}
