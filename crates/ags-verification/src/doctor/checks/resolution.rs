use super::*;

/// Credential-shaped key suffixes normalized to lowercase alphanumerics.
///
/// Suffix matching avoids false positives on legitimate keys like `authority`
/// while still catching compound keys such as `client_secret` and `api_key`.
const CRED_KEY_SUFFIXES: &[&str] = &[
    "token",
    "secret",
    "secretkey",
    "password",
    "passwd",
    "passphrase",
    "apikey",
    "credential",
    "credentials",
    "authorization",
    "bearer",
    "privatekey",
    "accesskey",
    "clientsecret",
];

/// Normalize a YAML key to lowercase alphanumerics, so `api_key`, `API-KEY`,
/// `apiKey`, and `api key` all collapse to `apikey`.
pub(super) fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub(super) fn is_credential_key(normalized: &str) -> bool {
    CRED_KEY_SUFFIXES.iter().any(|t| normalized.ends_with(t))
}

/// Recursively collect credential-evidence violations from a parsed YAML value.
/// Flags (a) any mapping KEY shaped like a credential and (b) an `auth_status`
/// key whose scalar value asserts `configured` (case-insensitive). Inspects KEYS,
/// not arbitrary prose values — a `denied:` note mentioning "tokens" is not a hit.
pub(super) fn scan_yaml_credentials(value: &YamlValue, path: &str, out: &mut Vec<String>) {
    match value {
        YamlValue::Mapping(map) => {
            for (k, v) in map.iter() {
                let key = k.as_str().unwrap_or("");
                let here = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                let norm = normalize_key(key);
                if is_credential_key(&norm) {
                    out.push(here.clone());
                } else if norm == "authstatus"
                    && v.as_str()
                        .map(|s| s.to_ascii_lowercase().contains("configured"))
                        .unwrap_or(false)
                {
                    out.push(format!("{here}=configured"));
                }
                scan_yaml_credentials(v, &here, out);
            }
        }
        YamlValue::Sequence(seq) => {
            for (i, item) in seq.iter().enumerate() {
                scan_yaml_credentials(item, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// Read-only skill-resolution drift check. Two boundaries, never writes, never
/// probes a host CLI:
///  1. **auth-evidence boundary** — NO tracked manifest may carry a credential
///     key or assert a configured auth status. A violation is the one blocking
///     skill-resolution FAIL: runtime auth posture is runtime-derived only and
///     must never be tracked. Mirrors the credential grep in the verification gate.
///  2. **Capability snapshot** — machine-local Skill/MCP index presence and freshness.
pub fn skill_resolution_drift_check(repo_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // 1. Auth-evidence boundary — tracked manifests must carry no credential key
    // and assert no configured auth status. Parses each manifest and walks it
    // recursively, normalizing KEYS case-insensitively, so credential-shaped
    // fields (`api_key`, `Authorization`, `client_secret`, spaced/cased/nested
    // variants) are caught — not just the three lowercase substrings the older
    // line scan looked for.
    let mut violations: Vec<String> = Vec::new();
    for rel in [
        "manifests/skills-registry.yaml",
        "manifests/mcp-registry.yaml",
        "manifests/suite.yaml",
    ] {
        if let Ok(content) = std::fs::read_to_string(repo_root.join(rel)) {
            if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&content) {
                let mut hits = Vec::new();
                scan_yaml_credentials(&doc, "", &mut hits);
                for h in hits {
                    violations.push(format!("{rel}:{h}"));
                }
            }
        }
    }
    if violations.is_empty() {
        findings.push(Finding::pass(
            "skill-resolution-auth-boundary",
            "no credential key or configured auth status in tracked manifests",
        ));
    } else {
        findings.push(Finding::fail(
            "skill-resolution-auth-boundary",
            "tracked manifest carries a credential key or configured auth status",
            format!(
                "auth_status is runtime-derived and must never be tracked. Offending line(s): {}",
                violations.join(", ")
            ),
        ));
    }

    // 2. Machine-local Skill/MCP capability snapshot. Missing or stale state is a
    // governance precondition failure for Skill targets, never an advisory
    // deterministic active-skill snapshot.
    let runtime_home = ags_capability_governance::locate_runtime_home();
    let evidence = ags_capability_governance::snapshot_path(&runtime_home, "codex");
    match ags_capability_governance::load_static_snapshot(&runtime_home, "codex") {
        Ok(_) => findings.push(Finding::info(
            "skill-active-table-snapshot",
            format!("Codex Skill/MCP capability snapshot is current ({})", evidence.display()),
        )),
        Err(_) if !evidence.is_file() => findings.push(Finding::warn(
            "skill-active-table-snapshot",
            "machine-local Codex Skill/MCP capability snapshot is missing",
            format!(
                "Run `ags capability snapshot --host codex --write` (expected at {}). DirectResponse and pure MachineCli routes remain available.",
                evidence.display()
            ),
        )),
        Err(_) => findings.push(Finding::fail(
            "skill-active-table-snapshot",
            "machine-local Codex skill snapshot is stale",
            "Run `ags capability snapshot --host codex --write` before routing a Skill target.",
        )),
    }

    findings
}

/// Read-only routing-COVERAGE gate (manifest hygiene). Every adopted capability
/// — suite.yaml required/optional/personal skills and governed MCPs — must carry
/// an explicit `routing.route_state` (routable / not-routable / retired) in the
/// routing-source manifests. A missing route_state is exactly the
/// indistinguishable "forgot to annotate" gap the 0.2.7 closure removes, so it is a
/// FAIL — but it gates the MANIFEST AUTHOR (CI / doctor), never a live route:
/// This gates manifest authorship, not request routing. Hermetic: reads manifests
/// only, never probes a host.
pub fn skill_resolution_coverage_check(repo_root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Classify one registry entry's `routing` block EXACTLY as production
    // `collect_routing` does: a typed `RoutingMetadata` parse must succeed AND
    // `route_state` must be declared explicitly. A present-but-malformed block
    // (typo'd enum like `routeable`, non-mapping, invalid field) passes a naive
    // key-presence check yet is silently dropped from the production routing map,
    // so it must FAIL coverage rather than pass it.
    #[derive(PartialEq)]
    enum RouteCoverage {
        Covered,
        Malformed,
        Missing,
    }
    fn classify_routing(item: &YamlValue) -> RouteCoverage {
        let Some(block) = item.get("routing") else {
            return RouteCoverage::Missing;
        };
        if serde_yaml::from_value::<ags_capability_governance::skill_body::console::RoutingMetadata>(block.clone())
            .is_err()
        {
            return RouteCoverage::Malformed;
        }
        if block.get("route_state").is_none() {
            return RouteCoverage::Missing;
        }
        RouteCoverage::Covered
    }
    fn routable_metadata_complete(item: &YamlValue) -> bool {
        let Some(routing) = item.get("routing") else {
            return true;
        };
        if routing.get("route_state").and_then(YamlValue::as_str) != Some("routable") {
            return true;
        }
        let nonempty_string = |key: &str| {
            routing
                .get(key)
                .and_then(YamlValue::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        };
        let nonempty_sequence = |value: Option<&YamlValue>| {
            value
                .and_then(YamlValue::as_sequence)
                .is_some_and(|items| !items.is_empty())
        };
        nonempty_string("invoke_hint")
            && nonempty_sequence(routing.get("intent_tags"))
            && nonempty_sequence(routing.get("examples").and_then(|v| v.get("positive")))
            && nonempty_sequence(routing.get("examples").and_then(|v| v.get("negative")))
    }

    // name → coverage verdict from skills-registry (typed parse, not key presence).
    let sr = repo_root.join("manifests/skills-registry.yaml");
    let skill_cov: std::collections::HashMap<String, RouteCoverage> =
        match std::fs::read_to_string(&sr) {
            Ok(c) => match serde_yaml::from_str::<YamlValue>(&c) {
                Ok(doc) => doc
                    .get("skills")
                    .and_then(|v| v.as_sequence())
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|it| {
                                it.get("name")
                                    .and_then(|n| n.as_str())
                                    .map(|n| (n.to_string(), classify_routing(it)))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(e) => {
                    return vec![Finding::skip(
                        "skill-resolution-coverage",
                        format!("skills-registry.yaml unparseable: {e}"),
                    )]
                }
            },
            Err(_) => {
                return vec![Finding::skip(
                    "skill-resolution-coverage",
                    "skills-registry.yaml not present (non-suite edition)",
                )]
            }
        };

    // Suite skill names from suite.yaml (required + optional lists, personal map).
    let mut suite_skills: Vec<String> = Vec::new();
    if let Ok(c) = std::fs::read_to_string(repo_root.join("manifests/suite.yaml")) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&c) {
            let suite = doc.get("suite");
            for sect in ["required", "optional"] {
                if let Some(seq) = suite
                    .and_then(|s| s.get(sect))
                    .and_then(|v| v.as_sequence())
                {
                    suite_skills.extend(
                        seq.iter()
                            .filter_map(|it| it.get("name").and_then(|n| n.as_str()))
                            .map(String::from),
                    );
                }
            }
            if let Some(map) = suite
                .and_then(|s| s.get("personal"))
                .and_then(|v| v.as_mapping())
            {
                suite_skills.extend(map.keys().filter_map(|k| k.as_str()).map(String::from));
            }
        }
    }

    let mut malformed: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for name in suite_skills {
        match skill_cov.get(&name) {
            Some(RouteCoverage::Covered) => {}
            Some(RouteCoverage::Malformed) => malformed.push(name),
            _ => missing.push(name),
        }
    }
    if malformed.is_empty() && missing.is_empty() {
        findings.push(Finding::pass(
            "skill-resolution-coverage",
            "every suite skill declares a valid, explicit routing.route_state (typed parse)",
        ));
    } else {
        let mut detail = String::new();
        if !malformed.is_empty() {
            detail.push_str(&format!(
                "malformed routing block (fails typed parse, dropped from routing): {}. ",
                malformed.join(", ")
            ));
        }
        if !missing.is_empty() {
            detail.push_str(&format!(
                "missing explicit route_state (routable | not-routable | retired, never defaulted): {}.",
                missing.join(", ")
            ));
        }
        findings.push(Finding::fail(
            "skill-resolution-coverage",
            "suite skills with invalid or missing routing.route_state",
            detail,
        ));
    }

    // Governed MCPs: same typed-parse coverage (key presence is not enough).
    if let Ok(c) = std::fs::read_to_string(repo_root.join("manifests/mcp-registry.yaml")) {
        if let Ok(doc) = serde_yaml::from_str::<YamlValue>(&c) {
            if let Some(seq) = doc.get("mcps").and_then(|v| v.as_sequence()) {
                let mcp_bad: Vec<String> = seq
                    .iter()
                    .filter(|it| classify_routing(it) != RouteCoverage::Covered)
                    .filter_map(|it| it.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect();
                if mcp_bad.is_empty() {
                    findings.push(Finding::pass(
                        "skill-resolution-coverage-mcp",
                        "every governed MCP declares a valid, explicit routing.route_state",
                    ));
                } else {
                    findings.push(Finding::fail(
                        "skill-resolution-coverage-mcp",
                        "governed MCPs with invalid or missing routing.route_state",
                        format!(
                            "Fix routing.route_state (valid + explicit) in mcp-registry.yaml for: {}.",
                            mcp_bad.join(", ")
                        ),
                    ));
                }
            }
        }
    }

    let mut incomplete = Vec::new();
    for (path, sections) in [
        (
            "manifests/skills-registry.yaml",
            &["skills", "route_targets"][..],
        ),
        (
            "manifests/mcp-registry.yaml",
            &["mcps", "route_targets"][..],
        ),
    ] {
        let Ok(content) = std::fs::read_to_string(repo_root.join(path)) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<YamlValue>(&content) else {
            continue;
        };
        for section in sections {
            let Some(items) = doc.get(*section).and_then(YamlValue::as_sequence) else {
                continue;
            };
            incomplete.extend(
                items
                    .iter()
                    .filter(|item| !routable_metadata_complete(item))
                    .filter_map(|item| item.get("name").and_then(YamlValue::as_str))
                    .map(|name| format!("{section}:{name}")),
            );
        }
    }
    if incomplete.is_empty() {
        findings.push(Finding::pass(
            "routing-metadata-completeness",
            "every routable Skill, MCP, and CLI-backed target declares invoke_hint, intent_tags, and positive/negative examples",
        ));
    } else {
        findings.push(Finding::fail(
            "routing-metadata-completeness",
            "routable capabilities have incomplete host semantic-selection metadata",
            format!(
                "Add invoke_hint, intent_tags, and non-empty positive/negative examples for: {}. AGS still does not interpret natural language; these fields populate the host catalog.",
                incomplete.join(", ")
            ),
        ));
    }

    findings
}
