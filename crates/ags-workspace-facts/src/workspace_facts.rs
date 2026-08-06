use super::*;
use ags_host_integration::extract_profile_slug;
// ── Shared / common types ──────────────────────────────────────────────────

/// Known AGS protocol files under `protocol/`.
pub(super) const PROTOCOL_FILES: &[(&str, &str)] = &[
    (
        "agent-task-protocol.md",
        "Task card and review rules (canonical)",
    ),
    ("task-card-template.md", "Fixed task card skeleton"),
    (
        "runtime-adapters.md",
        "Executor, permission, review, resume rules + resolver protocol",
    ),
    ("task-routing.md", "Light/medium/heavy task routing"),
    ("project-profile.md", "Project profile schema"),
    ("context-memory.md", "Context memory protocol"),
];

/// Known root-level protocol entry-point documents.
pub(super) const ROOT_ENTRY_FILES: &[(&str, &str)] = &[
    ("AGENTS.md", "Agent entry point"),
    ("CLAUDE.md", "Agent execution protocol"),
    ("WORKSPACE.md", "Repository role map"),
    ("AGENT_SUITE_PROTOCOL.md", "Suite protocol overview"),
];

// ── Workspace detection ────────────────────────────────────────────────────

/// Parsed workspace identity row from WORKSPACE.md.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceIdentity {
    pub code: String,
    pub role: String,
    pub path: String,
}

/// Parse WORKSPACE.md content to extract the workspace identity table.
///
/// Looks for a markdown table with columns `Code | Role | Path` and
/// extracts each row until a blank line or non-table line.
pub(super) fn parse_workspace_table(content: &str) -> Vec<WorkspaceIdentity> {
    let mut identities = Vec::new();
    let mut in_table = false;
    let mut seen_header_sep = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect table header
        if trimmed.starts_with("| Code | Role | Path |")
            || trimmed.starts_with("| Code | Role | Path |")
        {
            in_table = true;
            continue;
        }

        if !in_table {
            continue;
        }

        // Header separator row
        if trimmed.starts_with("|---") || trimmed.starts_with("| ---") {
            seen_header_sep = true;
            continue;
        }

        // Table data row
        if trimmed.starts_with('|') && seen_header_sep {
            let cells: Vec<&str> = trimmed
                .split('|')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if cells.len() >= 3 {
                // Strip markdown backticks and whitespace from path cell
                let path = cells[2].trim_matches('`').trim().to_string();
                identities.push(WorkspaceIdentity {
                    code: cells[0].to_string(),
                    role: cells[1].to_string(),
                    path,
                });
            }
        } else if seen_header_sep && trimmed.is_empty() {
            // Blank line ends the table
            break;
        } else if seen_header_sep && !trimmed.starts_with('|') {
            // Non-table, non-blank line ends the table
            break;
        }
    }

    identities
}

// ── Project detection ──────────────────────────────────────────────────────

/// Integration status classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    /// This IS the AGS development suite.
    Suite,
    /// AGS-integrated project (has profile, AGENTS.md, memory).
    Integrated,
    /// Not integrated at all.
    NotIntegrated,
    /// Some AGS markers present but significant gaps.
    Partial,
}

/// Full project identity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectIdentity {
    pub target: PathBuf,
    #[serde(skip_serializing)]
    pub inferred_role: Option<WorkspaceIdentity>,
    pub integration_status: IntegrationStatus,
    pub is_ags_suite: bool,
    pub is_ags_integrated: bool,
    pub gaps: Vec<String>,
    pub workspace_identities: Vec<WorkspaceIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_profile_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_capsule_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_memory_path: Option<PathBuf>,
    pub protocol_files_found: Vec<String>,
    pub protocol_files_missing: Vec<String>,
    pub root_entry_files_found: Vec<String>,
    pub root_entry_files_missing: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
}

/// Detect project identity for the given target directory.
///
/// This is a read-only inspection; it does not modify any files.
pub fn detect_project(target: &Path) -> ProjectIdentity {
    let canonical = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let mut identity = ProjectIdentity {
        target: canonical.clone(),
        inferred_role: None,
        integration_status: IntegrationStatus::NotIntegrated,
        is_ags_suite: false,
        is_ags_integrated: false,
        gaps: Vec::new(),
        workspace_identities: Vec::new(),
        project_profile_path: None,
        memory_capsule_path: None,
        task_memory_path: None,
        protocol_files_found: Vec::new(),
        protocol_files_missing: Vec::new(),
        root_entry_files_found: Vec::new(),
        root_entry_files_missing: Vec::new(),
        project_slug: None,
    };

    // ── Check root entry files ─────────────────────────────────────────
    for (name, _desc) in ROOT_ENTRY_FILES {
        let path = canonical.join(name);
        if path.exists() {
            identity.root_entry_files_found.push(name.to_string());
        } else {
            identity.root_entry_files_missing.push(name.to_string());
        }
    }

    // ── Check protocol/ files ──────────────────────────────────────────
    let protocol_dir = canonical.join("protocol");
    for (name, _desc) in PROTOCOL_FILES {
        let path = protocol_dir.join(name);
        if path.exists() {
            identity
                .protocol_files_found
                .push(format!("protocol/{}", name));
        } else {
            identity
                .protocol_files_missing
                .push(format!("protocol/{}", name));
        }
    }

    // ── Parse WORKSPACE.md for role identities ─────────────────────────
    let workspace_path = canonical.join("WORKSPACE.md");
    if workspace_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&workspace_path) {
            identity.workspace_identities = parse_workspace_table(&content);
        }
    }

    // ── Check for Cargo.toml with AGS workspace members ────────────────
    let cargo_toml = canonical.join("Cargo.toml");
    let has_ags_workspace = if cargo_toml.exists() {
        if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
            content.contains("ags-cli") || content.contains("task-card-validator")
        } else {
            false
        }
    } else {
        false
    };

    // ── Check for crates/ directory ────────────────────────────────────
    let crates_dir = canonical.join("crates");
    let has_crates_dir = crates_dir.is_dir();
    let has_suite_manifest = canonical.join("manifests/suite.yaml").is_file();

    // ── Suite detection ────────────────────────────────────────────────
    // Product identity comes from repository-owned typed facts. The optional
    // WORKSPACE role table is machine-local role inference and must not be a
    // prerequisite for a sanitized public Suite checkout.
    identity.is_ags_suite = identity
        .root_entry_files_found
        .contains(&"WORKSPACE.md".to_string())
        && identity
            .root_entry_files_found
            .contains(&"AGENT_SUITE_PROTOCOL.md".to_string())
        && has_ags_workspace
        && has_crates_dir
        && has_suite_manifest;

    // ── Integration markers ────────────────────────────────────────────
    // Check for project profile
    let profile_path = canonical.join("config").join("agent-project-profile.yaml");
    if profile_path.exists() {
        identity.project_profile_path = Some(profile_path.clone());
        identity.project_slug = extract_profile_slug(&canonical);
    }

    // Check for memory capsule
    let slug = identity
        .project_slug
        .clone()
        .unwrap_or_else(|| slug_from_path(&canonical));
    let home = ags_platform::home_dir_or_temp();
    let capsule_path = home
        .join(".agents/memory/projects")
        .join(&slug)
        .join("context-capsule.md");
    if capsule_path.exists() {
        identity.memory_capsule_path = Some(capsule_path.clone());
    }
    let task_mem_path = home
        .join(".agents/memory/projects")
        .join(&slug)
        .join("task-memory.md");
    if task_mem_path.exists() {
        identity.task_memory_path = Some(task_mem_path);
    }

    // Check AGENTS.md references AGS
    let agents_md = canonical.join("AGENTS.md");
    let agents_refs_ags = if agents_md.exists() {
        if let Ok(content) = std::fs::read_to_string(&agents_md) {
            content.contains("AGENT_SUITE_PROTOCOL.md")
                || content.contains("agent-governance")
                || content.contains("task-card-validator")
        } else {
            false
        }
    } else {
        false
    };

    // Count integration markers
    let mut integration_markers = 0u8;
    if identity.project_profile_path.is_some() {
        integration_markers += 1;
    }
    if identity.memory_capsule_path.is_some() {
        integration_markers += 1;
    }
    if agents_refs_ags {
        integration_markers += 1;
    }

    // ── Inferred role from the repository-owned workspace table ────────
    // Machine-specific A/S/B paths belong in WORKSPACE.md or host config,
    // never in the public runtime binary.
    identity.inferred_role = identity
        .workspace_identities
        .iter()
        .find(|role| {
            let declared = PathBuf::from(
                role.path
                    .trim()
                    .trim_matches('`')
                    .replace("$HOME", &ags_platform::home_dir_or_temp().to_string_lossy()),
            );
            declared.canonicalize().unwrap_or(declared).eq(&canonical)
        })
        .cloned();

    // ── Classify integration status ────────────────────────────────────
    if identity.is_ags_suite {
        identity.integration_status = IntegrationStatus::Suite;
    } else if integration_markers >= 3 {
        identity.integration_status = IntegrationStatus::Integrated;
    } else if integration_markers > 0 {
        identity.integration_status = IntegrationStatus::Partial;
    } else {
        identity.integration_status = IntegrationStatus::NotIntegrated;
    }

    // is_ags_integrated must be consistent with integration_status
    identity.is_ags_integrated = matches!(
        identity.integration_status,
        IntegrationStatus::Suite | IntegrationStatus::Integrated
    );

    // ── Build gaps list ────────────────────────────────────────────────
    if identity.integration_status != IntegrationStatus::Suite
        && identity.integration_status != IntegrationStatus::Integrated
    {
        if identity.project_profile_path.is_none() {
            identity
                .gaps
                .push("Missing config/agent-project-profile.yaml".to_string());
        }
        if identity.memory_capsule_path.is_none() {
            identity
                .gaps
                .push("Missing context-capsule.md in local memory".to_string());
        }
        if !agents_refs_ags {
            identity
                .gaps
                .push("AGENTS.md does not reference AGS protocols".to_string());
        }
        if !identity
            .root_entry_files_found
            .contains(&"CLAUDE.md".to_string())
        {
            identity
                .gaps
                .push("Missing CLAUDE.md (agent execution protocol)".to_string());
        }
    }

    // For suite repos, list any missing protocol files
    if identity.is_ags_suite {
        for m in &identity.protocol_files_missing {
            identity.gaps.push(format!("Missing protocol file: {}", m));
        }
        for m in &identity.root_entry_files_missing {
            identity.gaps.push(format!("Missing root entry: {}", m));
        }
    }

    identity
}

/// Derive a project slug from a path (fallback when no profile exists).
pub(super) fn slug_from_path(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
