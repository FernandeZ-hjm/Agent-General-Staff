//! Capability registry — read-only auto-discovery of project capabilities.
//!
//! Scans the current repository for capabilities in three categories:
//! - **policy files** under `protocol/` → `policy:<file-stem>`
//! - **scripts** under `scripts/` → `script:<file-name>`
//! - **Rust crates** under `crates/` → `rust:<crate-name>`
//!
//! Never scans `$HOME`, `.claude/`, or external paths.  Never installs,
//! updates, or deletes anything.
//!
//! # Stable ID namespaces
//!
//! | Kind | ID pattern | Example |
//! |---|---|---|
//! | `policy_file` | `policy:<file-stem>` | `policy:agent-task-protocol` |
//! | `script` | `script:<file-name>` | `script:verify.sh` |
//! | `rust_tool` | `rust:<crate-name>` | `rust:task-card-validator` |
//!
//! # Trust levels
//!
//! | Level | Meaning | Default for |
//! |---|---|---|
//! | `trusted` | AGS core deliverable | protocol files, AGS crates |
//! | `reviewed` | Human-reviewed external capability | — |
//! | `observed` | Detected but unreviewed | scripts |
//! | `external` | Known external dependency | — |

use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Data model ──────────────────────────────────────────────────────────────

/// The kind of a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    PolicyFile,
    Script,
    RustTool,
}

impl std::fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityKind::PolicyFile => write!(f, "policy_file"),
            CapabilityKind::Script => write!(f, "script"),
            CapabilityKind::RustTool => write!(f, "rust_tool"),
        }
    }
}

/// Trust level of a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    Trusted,
    Reviewed,
    Observed,
    External,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrustLevel::Trusted => write!(f, "trusted"),
            TrustLevel::Reviewed => write!(f, "reviewed"),
            TrustLevel::Observed => write!(f, "observed"),
            TrustLevel::External => write!(f, "external"),
        }
    }
}

/// A single discovered capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable namespaced identifier.
    pub id: String,
    /// Capability kind.
    pub kind: CapabilityKind,
    /// Repository-relative path.
    pub path: String,
    /// Trust level.
    pub trust_level: TrustLevel,
    /// Human-readable evidence of detection.
    pub evidence: String,
    /// Name of the detector that found this capability.
    pub detector: String,
}

/// Full capability registry result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRegistry {
    pub schema_version: String,
    pub capabilities: Vec<Capability>,
}

// ── Scanners ────────────────────────────────────────────────────────────────

/// Discover all capabilities in the given project root.
///
/// Runs all scanners and returns merged results.  Only scans `protocol/`,
/// `scripts/`, and `crates/` directories under `project_root`.
pub fn discover_all(project_root: &Path) -> CapabilityRegistry {
    let mut capabilities = Vec::new();
    capabilities.extend(scan_protocol(project_root));
    capabilities.extend(scan_scripts(project_root));
    capabilities.extend(scan_crates(project_root));
    // Stable sort by id for deterministic output
    capabilities.sort_by(|a, b| a.id.cmp(&b.id));

    CapabilityRegistry {
        schema_version: "2.0-m5".to_string(),
        capabilities,
    }
}

/// Scan `protocol/` directory for policy files.
fn scan_protocol(root: &Path) -> Vec<Capability> {
    let protocol_dir = root.join("protocol");
    let mut caps = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&protocol_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    caps.push(Capability {
                        id: format!("policy:{}", stem),
                        kind: CapabilityKind::PolicyFile,
                        path: format!("protocol/{}.md", stem),
                        trust_level: TrustLevel::Trusted,
                        evidence: format!("found protocol file at protocol/{}.md", stem),
                        detector: "protocol-scanner".to_string(),
                    });
                }
            }
        }
    }

    caps
}

/// Scan `scripts/` directory for executable scripts.
fn scan_scripts(root: &Path) -> Vec<Capability> {
    let scripts_dir = root.join("scripts");
    let mut caps = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // Only include files with known script extensions or no extension
                    if name.ends_with(".sh") || name.ends_with(".py") || name.ends_with(".rb") {
                        caps.push(Capability {
                            id: format!("script:{}", name),
                            kind: CapabilityKind::Script,
                            path: format!("scripts/{}", name),
                            trust_level: TrustLevel::Observed,
                            evidence: format!("found script at scripts/{}", name),
                            detector: "script-scanner".to_string(),
                        });
                    }
                }
            }
        }
    }

    caps
}

/// Scan `crates/` directory for Rust crates (each subdirectory with a Cargo.toml).
fn scan_crates(root: &Path) -> Vec<Capability> {
    let crates_dir = root.join("crates");
    let mut caps = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let cargo_toml = path.join("Cargo.toml");
                if cargo_toml.exists() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        caps.push(Capability {
                            id: format!("rust:{}", name),
                            kind: CapabilityKind::RustTool,
                            path: format!("crates/{}/", name),
                            trust_level: TrustLevel::Trusted,
                            evidence: format!("found Cargo.toml at crates/{}/", name),
                            detector: "crate-scanner".to_string(),
                        });
                    }
                }
            }
        }
    }

    caps
}

// ── Render functions ────────────────────────────────────────────────────────

/// Render capability list as human-readable text.
pub fn render_text(registry: &CapabilityRegistry) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Capability Registry (schema {})",
        registry.schema_version
    ));
    lines.push(format!(
        "{} capabilities found",
        registry.capabilities.len()
    ));
    lines.push(String::new());

    for cap in &registry.capabilities {
        lines.push(format!("  [{}] {}", cap.kind, cap.id));
        lines.push(format!("    path:        {}", cap.path));
        lines.push(format!("    trust_level: {}", cap.trust_level));
        lines.push(format!("    detector:    {}", cap.detector));
        lines.push(format!("    evidence:    {}", cap.evidence));
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Render capability list as JSON string.
pub fn render_json(registry: &CapabilityRegistry) -> String {
    serde_json::to_string_pretty(registry)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// Render a single capability as human-readable text.
pub fn render_one_text(cap: &Capability) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Capability: {}", cap.id));
    lines.push(format!("  Kind:        {}", cap.kind));
    lines.push(format!("  Path:        {}", cap.path));
    lines.push(format!("  Trust level: {}", cap.trust_level));
    lines.push(format!("  Evidence:    {}", cap.evidence));
    lines.push(format!("  Detector:    {}", cap.detector));
    lines.join("\n")
}

/// Render a single capability as JSON string.
pub fn render_one_json(cap: &Capability) -> String {
    serde_json::to_string_pretty(cap)
        .unwrap_or_else(|e| format!(r#"{{"error": "JSON serialization failed: {}"}}"#, e))
}

/// Find a capability by stable ID.
pub fn find_by_id<'a>(registry: &'a CapabilityRegistry, id: &str) -> Option<&'a Capability> {
    registry.capabilities.iter().find(|c| c.id == id)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a minimal temp project structure for discovery testing.
    fn setup_temp_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        // protocol/
        let proto = dir.path().join("protocol");
        fs::create_dir_all(&proto).unwrap();
        fs::write(proto.join("agent-task-protocol.md"), "# Test\n").unwrap();
        fs::write(proto.join("runtime-adapters.md"), "# Test\n").unwrap();
        // Non-.md file — should be ignored
        fs::write(proto.join("notes.txt"), "ignored").unwrap();

        // scripts/
        let scripts = dir.path().join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("verify.sh"), "#!/bin/bash\necho ok\n").unwrap();
        fs::write(scripts.join("govern.py"), "print('ok')\n").unwrap();
        // Non-script file — should be ignored
        fs::write(scripts.join("README.md"), "# Scripts\n").unwrap();

        // crates/
        let crates = dir.path().join("crates");
        fs::create_dir_all(&crates).unwrap();
        let crate_a = crates.join("my-tool");
        fs::create_dir_all(&crate_a).unwrap();
        fs::write(
            crate_a.join("Cargo.toml"),
            "[package]\nname = \"my-tool\"\n",
        )
        .unwrap();

        let crate_b = crates.join("other-lib");
        fs::create_dir_all(&crate_b).unwrap();
        fs::write(
            crate_b.join("Cargo.toml"),
            "[package]\nname = \"other-lib\"\n",
        )
        .unwrap();

        // Directory without Cargo.toml — should be ignored
        let not_a_crate = crates.join("not-a-crate");
        fs::create_dir_all(&not_a_crate).unwrap();
        fs::write(not_a_crate.join("README.md"), "# Not a crate\n").unwrap();

        // Empty directory without any AGS structure
        let empty = dir.path().join("src");
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("main.rs"), "fn main() {}\n").unwrap();

        dir
    }

    #[test]
    fn discovers_protocol_files() {
        let dir = setup_temp_project();
        let caps = scan_protocol(dir.path());
        let ids: Vec<&str> = caps.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"policy:agent-task-protocol"));
        assert!(ids.contains(&"policy:runtime-adapters"));
        // notes.txt should not appear
        assert!(!ids.contains(&"policy:notes"));
    }

    #[test]
    fn protocol_capability_has_correct_shape() {
        let dir = setup_temp_project();
        let caps = scan_protocol(dir.path());
        let cap = caps
            .iter()
            .find(|c| c.id == "policy:agent-task-protocol")
            .unwrap();
        assert_eq!(cap.kind, CapabilityKind::PolicyFile);
        assert_eq!(cap.path, "protocol/agent-task-protocol.md");
        assert_eq!(cap.trust_level, TrustLevel::Trusted);
        assert_eq!(cap.detector, "protocol-scanner");
        assert!(cap.evidence.contains("protocol/agent-task-protocol.md"));
    }

    #[test]
    fn discovers_scripts() {
        let dir = setup_temp_project();
        let caps = scan_scripts(dir.path());
        let ids: Vec<&str> = caps.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"script:verify.sh"));
        assert!(ids.contains(&"script:govern.py"));
        // README.md should NOT appear (not a script extension)
        assert!(!ids.contains(&"script:README.md"));
    }

    #[test]
    fn script_capability_has_correct_shape() {
        let dir = setup_temp_project();
        let caps = scan_scripts(dir.path());
        let cap = caps.iter().find(|c| c.id == "script:verify.sh").unwrap();
        assert_eq!(cap.kind, CapabilityKind::Script);
        assert_eq!(cap.path, "scripts/verify.sh");
        assert_eq!(cap.trust_level, TrustLevel::Observed);
        assert_eq!(cap.detector, "script-scanner");
        assert!(cap.evidence.contains("scripts/verify.sh"));
    }

    #[test]
    fn discovers_rust_crates() {
        let dir = setup_temp_project();
        let caps = scan_crates(dir.path());
        let ids: Vec<&str> = caps.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"rust:my-tool"));
        assert!(ids.contains(&"rust:other-lib"));
        // not-a-crate should NOT appear (no Cargo.toml)
        assert!(!ids.contains(&"rust:not-a-crate"));
    }

    #[test]
    fn rust_crate_capability_has_correct_shape() {
        let dir = setup_temp_project();
        let caps = scan_crates(dir.path());
        let cap = caps.iter().find(|c| c.id == "rust:my-tool").unwrap();
        assert_eq!(cap.kind, CapabilityKind::RustTool);
        assert_eq!(cap.path, "crates/my-tool/");
        assert_eq!(cap.trust_level, TrustLevel::Trusted);
        assert_eq!(cap.detector, "crate-scanner");
        assert!(cap.evidence.contains("crates/my-tool/"));
    }

    #[test]
    fn discover_all_merges_and_sorts() {
        let dir = setup_temp_project();
        let registry = discover_all(dir.path());
        assert!(registry.capabilities.len() >= 5);
        assert_eq!(registry.schema_version, "2.0-m5");
        // Verify sorted by id
        for w in registry.capabilities.windows(2) {
            assert!(w[0].id <= w[1].id, "not sorted: {} > {}", w[0].id, w[1].id);
        }
    }

    #[test]
    fn discover_all_does_not_scan_home_or_claude() {
        let dir = setup_temp_project();
        let registry = discover_all(dir.path());
        for cap in &registry.capabilities {
            assert!(
                !cap.path.contains("$HOME"),
                "should not contain $HOME: {}",
                cap.path
            );
            assert!(
                !cap.path.contains(".claude"),
                "should not contain .claude: {}",
                cap.path
            );
            assert!(
                !cap.evidence.contains("$HOME"),
                "evidence should not reference $HOME"
            );
        }
    }

    #[test]
    fn find_by_id_returns_correct_capability() {
        let dir = setup_temp_project();
        let registry = discover_all(dir.path());
        let cap = find_by_id(&registry, "policy:agent-task-protocol");
        assert!(cap.is_some());
        assert_eq!(cap.unwrap().kind, CapabilityKind::PolicyFile);

        let missing = find_by_id(&registry, "rust:nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn missing_directories_return_empty() {
        let dir = tempfile::tempdir().unwrap();
        // No protocol/, scripts/, or crates/ directories
        let registry = discover_all(dir.path());
        assert!(registry.capabilities.is_empty());
    }

    #[test]
    fn render_json_is_valid() {
        let dir = setup_temp_project();
        let registry = discover_all(dir.path());
        let json = render_json(&registry);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("JSON should be valid");
        assert_eq!(parsed["schema_version"], "2.0-m5");
        assert!(parsed["capabilities"].is_array());
        assert!(parsed["capabilities"].as_array().unwrap().len() >= 5);
    }

    #[test]
    fn render_text_contains_capability_ids() {
        let dir = setup_temp_project();
        let registry = discover_all(dir.path());
        let text = render_text(&registry);
        assert!(text.contains("policy:agent-task-protocol"));
        assert!(text.contains("script:verify.sh"));
        assert!(text.contains("rust:my-tool"));
        assert!(text.contains("2.0-m5"));
    }

    #[test]
    fn render_one_json_is_valid() {
        let cap = Capability {
            id: "rust:test-crate".to_string(),
            kind: CapabilityKind::RustTool,
            path: "crates/test-crate/".to_string(),
            trust_level: TrustLevel::Trusted,
            evidence: "found Cargo.toml".to_string(),
            detector: "crate-scanner".to_string(),
        };
        let json = render_one_json(&cap);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["id"], "rust:test-crate");
    }

    #[test]
    fn serde_roundtrip_trust_level() {
        // Verify trust levels serialize/deserialize correctly
        let json = serde_json::to_string(&TrustLevel::Observed).unwrap();
        assert_eq!(json, r#""observed""#);
        let parsed: TrustLevel = serde_json::from_str(r#""trusted""#).unwrap();
        assert_eq!(parsed, TrustLevel::Trusted);
    }
}
