use super::*;
#[allow(unused_imports)]
use super::{host_probe::*, inventory::*, model::*};
// ── Host verify ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCheck {
    pub name: String,
    pub kind: String,
    pub visibility: HostVisibilityStatus,
    /// Whether this capability is expected to be visible on this host (drives
    /// the failure signal). Opt-in / not-applicable capabilities are false.
    pub expected: bool,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVerifySummary {
    pub total: usize,
    pub visible: usize,
    pub not_visible: usize,
    pub degraded: usize,
    /// Capabilities expected to be visible on this host.
    pub expected: usize,
    /// Expected capabilities that are NOT visible (the failure count).
    pub failed: usize,
    /// True when every expected capability is visible.
    pub all_visible: bool,
}

/// Read-only host thin-index report for dangling symlinks and real-directory
/// copies in a host's skills dir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinIndexDrift {
    pub host: String,
    pub skills_dir: String,
    pub total_entries: usize,
    /// Single thin-index symlinks pointing at a valid target (clean).
    pub clean_symlinks: usize,
    /// Dangling symlinks (target missing) — e.g. retired-skill fallout.
    pub broken_symlinks: usize,
    /// Real-directory copies (non-symlink, not `.system`) — informational: may be
    /// a legitimate local/external skill, not necessarily drift.
    pub real_dir_copies: usize,
    /// True when dangling symlinks exist.
    pub has_drift: bool,
    /// Capped sample of drift entry names for operator triage.
    pub drift_samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostVerifyResult {
    pub schema_version: String,
    pub host: String,
    pub supported: bool,
    /// "ok" | "degraded" | "incomplete" | "unsupported"
    pub status: String,
    pub checks: Vec<HostCheck>,
    pub summary: HostVerifySummary,
    /// Read-only thin-index drift report (None for unsupported hosts / absent
    /// skills dir). Reported, never auto-cleaned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thin_index_drift: Option<ThinIndexDrift>,
    /// Shared multi-agent index drift. Codex, OMP, and Cursor load
    /// `~/.agents/skills` in addition to their host-specific directories, so
    /// stale entries here are part of host visibility integrity rather than
    /// invisible machine clutter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_thin_index_drift: Option<ThinIndexDrift>,
    pub note: String,
}

/// Read-only scan of a host's thin-index dir for drift. NEVER mutates. Returns
/// `None` for hosts without a skills subdir or when the dir is absent. Counts
/// Dangling symlinks are drift; real directories are informational because
/// they may be legitimate local skills.
pub(super) fn scan_thin_index_drift(home: &Path, host: &str) -> Option<ThinIndexDrift> {
    let sub = host_skills_subdir(host)?;
    scan_skill_dir_drift(&home.join(sub), host)
}

pub(super) fn scan_shared_thin_index_drift(home: &Path, host: &str) -> Option<ThinIndexDrift> {
    ags_host_integration::platform_spec(host)
        .is_some_and(|spec| spec.loads_shared_agent_skills)
        .then(|| home.join(".agents/skills"))
        .and_then(|dir| scan_skill_dir_drift(&dir, "shared"))
}

pub(super) fn scan_skill_dir_drift(dir: &Path, label: &str) -> Option<ThinIndexDrift> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut total = 0usize;
    let (mut clean, mut broken, mut realdir) = (0usize, 0usize, 0usize);
    let mut samples: Vec<String> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".system" || name.starts_with(".ags-drift-quarantine") {
            continue;
        }
        total += 1;
        let path = entry.path();
        let is_link = std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let target_exists = path.exists(); // follows the symlink
        if is_link && !target_exists {
            broken += 1;
            if samples.len() < 12 {
                samples.push(format!("{name} (dangling symlink)"));
            }
        } else if is_link {
            clean += 1;
        } else if path.is_dir() {
            realdir += 1;
        }
    }
    Some(ThinIndexDrift {
        host: label.to_string(),
        skills_dir: dir.to_string_lossy().to_string(),
        total_entries: total,
        clean_symlinks: clean,
        broken_symlinks: broken,
        real_dir_copies: realdir,
        has_drift: broken > 0,
        drift_samples: samples,
    })
}

/// Verify host visibility for one host. Read-only.
pub fn verify_host(ctx: &ConsoleContext, host: &str) -> HostVerifyResult {
    let supported = host_skills_subdir(host).is_some();
    if !supported {
        return HostVerifyResult {
            schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
            host: host.to_string(),
            supported: false,
            status: "unsupported".to_string(),
            checks: Vec::new(),
            summary: HostVerifySummary {
                total: 0,
                visible: 0,
                not_visible: 0,
                degraded: 0,
                expected: 0,
                failed: 0,
                all_visible: true,
            },
            thin_index_drift: None,
            shared_thin_index_drift: None,
            note: format!("Host '{host}' is not a recognized AGS host."),
        };
    }

    let inventory = build_inventory(ctx, &[host]);
    let mut checks = Vec::new();
    for cap in &inventory.capabilities {
        if let Some(vis) = cap.host_visibility.iter().find(|v| v.host == host) {
            checks.push(HostCheck {
                name: cap.name.clone(),
                kind: kind_str(&cap.kind).to_string(),
                visibility: vis.status.clone(),
                expected: cap.expected_hosts.iter().any(|h| h == host),
                evidence: vis.evidence.clone(),
            });
        }
    }

    let visible = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::Visible)
        .count();
    let degraded = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::Degraded)
        .count();
    let not_visible = checks
        .iter()
        .filter(|c| c.visibility == HostVisibilityStatus::NotVisible)
        .count();
    let expected = checks.iter().filter(|c| c.expected).count();
    // `failed` = an expected capability the host definitively cannot see
    // (NotVisible) → status incomplete. An expected capability we merely
    // couldn't confirm (Degraded) does not count as failed but does prevent an
    // "ok" verdict (→ degraded).
    let failed = checks
        .iter()
        .filter(|c| c.expected && c.visibility == HostVisibilityStatus::NotVisible)
        .count();
    let expected_degraded = checks
        .iter()
        .filter(|c| c.expected && c.visibility == HostVisibilityStatus::Degraded)
        .count();
    let thin_index_drift = scan_thin_index_drift(&ctx.home, host);
    let shared_thin_index_drift = scan_shared_thin_index_drift(&ctx.home, host);
    let removable_drift = thin_index_drift
        .as_ref()
        .is_some_and(|drift| drift.has_drift)
        || shared_thin_index_drift
            .as_ref()
            .is_some_and(|drift| drift.has_drift);
    let all_visible = failed == 0 && expected_degraded == 0;
    let status = if failed > 0 {
        "incomplete"
    } else if expected_degraded > 0 || removable_drift {
        "degraded"
    } else {
        "ok"
    }
    .to_string();

    HostVerifyResult {
        schema_version: CONSOLE_SCHEMA_VERSION.to_string(),
        host: host.to_string(),
        supported: true,
        status,
        summary: HostVerifySummary {
            total: checks.len(),
            visible,
            not_visible,
            degraded,
            expected,
            failed,
            all_visible,
        },
        checks,
        thin_index_drift,
        shared_thin_index_drift,
        note: "Read-only host-visibility verify. status=incomplete means an expected capability is not visible; dangling host/shared symlinks degrade strict verification. Restart the host or open a new task after snapshot refresh so it re-scans entry points; use --strict to gate (exit nonzero unless status=ok).".to_string(),
    }
}

pub(super) fn kind_str(k: &ManagedKind) -> &'static str {
    match k {
        ManagedKind::Skill => "skill",
        ManagedKind::Mcp => "mcp",
        ManagedKind::SuiteInterface => "suite-interface",
        ManagedKind::CliBacked => "cli-backed",
    }
}
