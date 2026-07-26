use super::inventory::SkillInventoryResult;
use super::model::{
    FileStatus, SkillCheckResult, SkillProposalResult, SkillScanResult, SkillStatus,
};
use super::upstream::UpstreamProposalResult;

/// Render scan result as human-readable text.
pub fn render_scan_text(result: &SkillScanResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Scan Report".to_string());
    lines.push("================================".to_string());
    lines.push(format!("Schema:       {}", result.schema_version));
    lines.push(format!("Suite:        {}", result.suite_name));
    lines.push(format!("Version:      {}", result.suite_version));
    lines.push(String::new());

    lines.push("─ Summary ─".to_string());
    lines.push(format!(
        "  Total:     {} (available: {}, optional: {}, personal: {}, missing: {}, disabled: {}, degraded: {})",
        result.summary.total,
        result.summary.available,
        result.summary.optional,
        result.summary.personal,
        result.summary.missing,
        result.summary.disabled,
        result.summary.degraded,
    ));
    lines.push(String::new());

    if result.skills.is_empty() {
        lines.push("No skills found in suite manifest.".to_string());
        lines.push("(This is expected for a Phase 1 suite with an empty manifest.)".to_string());
    } else {
        lines.push("─ Skills ─".to_string());
        for skill in &result.skills {
            let status_icon = match skill.status {
                SkillStatus::Available => "[AVAILABLE]",
                SkillStatus::Optional => "[OPTIONAL]",
                SkillStatus::Personal => "[PERSONAL]",
                SkillStatus::Missing => "[MISSING]",
                SkillStatus::Disabled => "[DISABLED]",
                SkillStatus::Degraded => "[DEGRADED]",
            };
            lines.push(format!(
                "  {} {} (profile: {})",
                status_icon, skill.name, skill.profile
            ));
            if let Some(ref version) = skill.version {
                lines.push(format!("    version: {}", version));
            }
            if let Some(ref source) = skill.source {
                lines.push(format!("    source:  {}", source));
            }
            for warning in &skill.warnings {
                lines.push(format!("    ⚠ {}", warning));
            }
        }
    }

    lines.join("\n")
}

/// Render scan result as JSON.
pub fn render_scan_json(result: &SkillScanResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render check result as human-readable text.
pub fn render_check_text(result: &SkillCheckResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Check Report".to_string());
    lines.push("=================================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Passed:  {}", result.passed));
    lines.push(String::new());

    // Governance files
    lines.push("─ Governance Files ─".to_string());
    let render_file = |label: &str, fs: &FileStatus| {
        let status = if fs.present && fs.parseable {
            "OK"
        } else if fs.present {
            "PARSE_ERROR"
        } else {
            "MISSING"
        };
        format!(
            "  {}: {} (entries: {}, schema: {:?})",
            label,
            status,
            fs.entry_count,
            fs.schema_version.as_deref().unwrap_or("?")
        )
    };
    lines.push(render_file(
        "skill-adoption-log",
        &result.governance_files.skill_adoption_log,
    ));
    lines.push(render_file(
        "skill-ignore-list",
        &result.governance_files.skill_ignore_list,
    ));
    lines.push(render_file(
        "suite-manifest",
        &result.governance_files.suite_manifest,
    ));
    lines.push(String::new());

    // Consistency checks
    lines.push("─ Consistency Checks ─".to_string());
    for check in &result.consistency_checks {
        let icon = if check.passed { "✓" } else { "✗" };
        lines.push(format!("  {} {}: {}", icon, check.name, check.detail));
    }
    lines.push(String::new());

    // Issues
    if result.issues.is_empty() {
        lines.push("─ Issues ─".to_string());
        lines.push("  None".to_string());
    } else {
        lines.push("─ Issues ─".to_string());
        for issue in &result.issues {
            lines.push(format!(
                "  [{}] [{}] {}",
                issue.severity.to_uppercase(),
                issue.category,
                issue.detail
            ));
        }
    }

    lines.join("\n")
}

/// Render check result as JSON.
pub fn render_check_json(result: &SkillCheckResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render proposal result as human-readable text.
pub fn render_proposal_text(result: &SkillProposalResult) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push("Skill Governance — Proposal".to_string());
    lines.push("===========================".to_string());
    lines.push(format!("Schema:     {}", result.schema_version));
    lines.push(format!("Action:     {}", result.proposal_type));
    lines.push(format!("Dry run:    {}", result.dry_run));
    lines.push(String::new());

    lines.push("─ Target Skills ─".to_string());
    if result.target_skills.is_empty() {
        lines.push("  None".to_string());
    } else {
        for skill in &result.target_skills {
            lines.push(format!("  - {}", skill));
        }
    }
    lines.push(String::new());

    lines.push("─ Proposed Changes ─".to_string());
    if result.proposed_changes.is_empty() {
        lines.push("  No changes proposed.".to_string());
    } else {
        for change in &result.proposed_changes {
            lines.push(format!("  + {}", change));
        }
    }
    lines.push(String::new());

    if !result.blocked_reasons.is_empty() {
        lines.push("─ Blocked ─".to_string());
        for reason in &result.blocked_reasons {
            lines.push(format!("  ✗ {}", reason));
        }
        lines.push(String::new());
    }

    lines.push(format!("NOTE: {}", result.note));

    lines.join("\n")
}

/// Render proposal result as JSON.
pub fn render_proposal_json(result: &SkillProposalResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}
/// Render inventory as human-readable text.
pub fn render_inventory_text(result: &SkillInventoryResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Asset Inventory".to_string());
    lines.push("=====================".to_string());
    lines.push(format!("Schema:  {}", result.schema_version));
    lines.push(format!("Roots:   {}", result.roots_scanned.join(", ")));
    lines.push(String::new());
    lines.push(format!(
        "Summary: total {} (global {}, optional {}, personal {}); with SKILL.md {}, with description {}, public-allowed guess {}, risk-flagged {}",
        result.summary.total,
        result.summary.global,
        result.summary.optional,
        result.summary.personal,
        result.summary.with_skill_md,
        result.summary.with_description,
        result.summary.public_allowed,
        result.summary.flagged_risk,
    ));
    lines.push(String::new());
    for e in &result.entries {
        let md = if e.has_skill_md { "md" } else { "NO-md" };
        let desc = if e.description_present {
            "desc"
        } else {
            "no-desc"
        };
        let public = if e.public_allowed_guess {
            "public?"
        } else {
            "private"
        };
        lines.push(format!(
            "  [{}] {} ({}, {}, {})",
            e.source_category, e.name, md, desc, public
        ));
        if !e.risk_hints.is_empty() {
            lines.push(format!("      risk hints: {}", e.risk_hints.join(", ")));
        }
    }
    lines.join("\n")
}

/// Render inventory as JSON.
pub fn render_inventory_json(result: &SkillInventoryResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}

/// Render inventory as a Markdown report (for governance/skills-inventory.md).
pub fn render_inventory_markdown(result: &SkillInventoryResult) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push("# Skill Asset Inventory".to_string());
    out.push(String::new());
    out.push(format!(
        "_Generated by `ags skill inventory --write`. Schema `{}`. Read-only scan of `SKILL.md` files; no secrets, tokens, or runtime files are read._",
        result.schema_version
    ));
    out.push(String::new());
    out.push("## Summary".to_string());
    out.push(String::new());
    out.push("| Metric | Count |".to_string());
    out.push("|---|---|".to_string());
    out.push(format!("| Total skills | {} |", result.summary.total));
    out.push(format!("| global-skills | {} |", result.summary.global));
    out.push(format!(
        "| skill-packs/optional | {} |",
        result.summary.optional
    ));
    out.push(format!(
        "| skill-packs/personal | {} |",
        result.summary.personal
    ));
    out.push(format!(
        "| With SKILL.md | {} |",
        result.summary.with_skill_md
    ));
    out.push(format!(
        "| With description | {} |",
        result.summary.with_description
    ));
    out.push(format!(
        "| Public-allowed (guess) | {} |",
        result.summary.public_allowed
    ));
    out.push(format!(
        "| Risk-flagged | {} |",
        result.summary.flagged_risk
    ));
    out.push(String::new());
    out.push("## Skills".to_string());
    out.push(String::new());
    out.push(
        "| Category | Name | SKILL.md | Description | Public-allowed (guess) | Risk hints |"
            .to_string(),
    );
    out.push("|---|---|---|---|---|---|".to_string());
    for e in &result.entries {
        let risk = if e.risk_hints.is_empty() {
            "—".to_string()
        } else {
            e.risk_hints.join(", ")
        };
        out.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            e.source_category,
            e.name,
            if e.has_skill_md { "yes" } else { "no" },
            if e.description_present { "yes" } else { "no" },
            if e.public_allowed_guess { "yes" } else { "no" },
            risk,
        ));
    }
    out.push(String::new());
    out.join("\n")
}
/// Render upstream proposal as human-readable text.
pub fn render_upstream_text(result: &UpstreamProposalResult) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("Skill Governance — Upstream Update Proposal (stub)".to_string());
    lines.push("=================================================".to_string());
    lines.push(format!("Schema:        {}", result.schema_version));
    lines.push(format!("Registry:      {}", result.registry_path));
    let status = if !result.registry_present {
        "MISSING"
    } else if !result.registry_parseable {
        "PARSE_ERROR"
    } else {
        "OK"
    };
    lines.push(format!("Status:        {status}"));
    lines.push(format!(
        "Update policy: {}",
        result.update_policy.as_deref().unwrap_or("?")
    ));
    lines.push(format!(
        "Summary:       upstreams {}, watched skills {}, candidates {}, crawl performed {}",
        result.summary.upstreams,
        result.summary.watched_skills,
        result.summary.candidates,
        result.summary.crawl_performed,
    ));
    lines.push(String::new());

    lines.push("─ Upstream Sources ─".to_string());
    if result.upstreams.is_empty() {
        lines.push("  None declared.".to_string());
    } else {
        for u in &result.upstreams {
            let crawl = if u.crawl { "crawl" } else { "no-crawl" };
            lines.push(format!(
                "  - {} ({}, {})",
                u.name,
                u.kind.as_deref().unwrap_or("?"),
                crawl
            ));
            if let Some(ref web) = u.web {
                lines.push(format!("      web: {web}"));
            }
        }
    }
    lines.push(String::new());

    lines.push("─ Watched Skills ─".to_string());
    if result.watched_skills.is_empty() {
        lines.push("  None.".to_string());
    } else {
        for s in &result.watched_skills {
            lines.push(format!(
                "  - {} → upstream {} (policy: {})",
                s.name,
                s.upstream.as_deref().unwrap_or("?"),
                s.update_policy.as_deref().unwrap_or("?"),
            ));
        }
    }
    lines.push(String::new());

    lines.push("─ Candidate Skills ─".to_string());
    if result.candidates.is_empty() {
        lines.push("  None.".to_string());
    } else {
        for c in &result.candidates {
            lines.push(format!(
                "  - {} (upstream: {}, priority: {}, mode: {})",
                c.name,
                c.upstream.as_deref().unwrap_or("?"),
                c.adoption_priority.as_deref().unwrap_or("?"),
                c.adoption_mode.as_deref().unwrap_or("?"),
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!("NOTE: {}", result.note));
    lines.join("\n")
}

/// Render upstream proposal as JSON.
pub fn render_upstream_json(result: &UpstreamProposalResult) -> String {
    serde_json::to_string_pretty(result)
        .unwrap_or_else(|e| format!(r#"{{"error":"JSON serialization failed: {}"}}"#, e))
}
