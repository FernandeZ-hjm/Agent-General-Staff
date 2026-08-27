//! Permission matrix evaluation (contract v3 §7.1 / §7.2).
//!
//! Patterns are `surface:action` pairs where either side may be `*`.
//! Evaluation: the most specific matching pattern wins; on a specificity tie
//! deny beats ask beats allow. No match is a deny (fail closed). Sealed
//! operations are matched against `[sealed].ops` and short-circuit to
//! `Decision::Sealed` — they never fall through to the tool matrix.

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
    Sealed,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Ask => "ask",
            Decision::Deny => "deny",
            Decision::Sealed => "sealed",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Decision::Allow => 1,
            Decision::Ask => 2,
            Decision::Deny => 3,
            Decision::Sealed => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    pub surface: String,
    pub action: String,
}

fn valid_literal(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Parse `surface:action`; `*` allowed on either side. Returns `None` for
/// malformed patterns (lint reports them; evaluation skips them).
pub fn parse_pattern(text: &str) -> Option<Pattern> {
    let (surface, action) = text.split_once(':')?;
    let surface = surface.trim().to_ascii_lowercase();
    let action = action.trim().to_ascii_lowercase();
    if (surface != "*" && !valid_literal(&surface)) || (action != "*" && !valid_literal(&action)) {
        return None;
    }
    Some(Pattern { surface, action })
}

pub fn pattern_matches(pattern: &Pattern, surface: &str, action: &str) -> bool {
    let s = surface.to_ascii_lowercase();
    let a = action.to_ascii_lowercase();
    (pattern.surface == "*" || pattern.surface == s)
        && (pattern.action == "*" || pattern.action == a)
}

fn specificity(pattern: &Pattern) -> u8 {
    u8::from(pattern.surface != "*") + u8::from(pattern.action != "*")
}

/// Evaluate a tool call (`surface` + `action`) against the permission matrix.
pub fn evaluate(config: &Config, surface: &str, action: &str) -> Decision {
    let mut best: Option<(u8, u8)> = None; // (specificity, rank)
    for (entries, decision) in [
        (config.permissions.allow.as_slice(), Decision::Allow),
        (config.permissions.ask.as_slice(), Decision::Ask),
        (config.permissions.deny.as_slice(), Decision::Deny),
    ] {
        for entry in entries {
            let Some(pattern) = parse_pattern(entry) else {
                continue;
            };
            if pattern_matches(&pattern, surface, action) {
                let candidate = (specificity(&pattern), decision.rank());
                match best {
                    Some(current)
                        if current.0 > candidate.0
                            || (current.0 == candidate.0 && current.1 >= candidate.1) => {}
                    _ => best = Some(candidate),
                }
            }
        }
    }
    match best {
        Some((_, 1)) => Decision::Allow,
        Some((_, 2)) => Decision::Ask,
        Some((_, 3)) => Decision::Deny,
        _ => Decision::Deny, // no match → fail closed
    }
}

/// Does `op` match the `[sealed].ops` list? Supports `prefix:*` entries.
pub fn op_is_sealed(config: &Config, op: &str) -> bool {
    let op = op.trim().to_ascii_lowercase();
    config.sealed.ops.iter().any(|entry| {
        let entry = entry.trim().to_ascii_lowercase();
        if let Some(prefix) = entry.strip_suffix(":*") {
            op.starts_with(prefix)
        } else {
            entry == op
        }
    })
}

/// Decision for a typed operation name (MCP `ags_decide` path). Sealed ops go
/// to the sealed transaction; anything else is blocked in contract v3 — the
/// registry only contains the sealed subset (G-06).
pub fn evaluate_op(config: &Config, op: &str) -> Decision {
    if op_is_sealed(config, op) {
        Decision::Sealed
    } else {
        Decision::Deny
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PermissionsSection};

    fn config_with(allow: &[&str], ask: &[&str], deny: &[&str]) -> Config {
        Config {
            permissions: PermissionsSection {
                allow: allow.iter().map(|s| s.to_string()).collect(),
                ask: ask.iter().map(|s| s.to_string()).collect(),
                deny: deny.iter().map(|s| s.to_string()).collect(),
            },
            ..Config::default()
        }
    }

    #[test]
    fn exact_match_wins_over_wildcard() {
        let c = config_with(&["bash:*", "bash:mutate"], &[], &[]);
        assert_eq!(evaluate(&c, "bash", "mutate"), Decision::Allow);
        assert_eq!(evaluate(&c, "bash", "readonly"), Decision::Allow);
    }

    #[test]
    fn deny_beats_ask_and_allow_on_tie() {
        let c = config_with(&["git:*"], &["git:commit"], &["git:commit"]);
        assert_eq!(evaluate(&c, "git", "commit"), Decision::Deny);
    }

    #[test]
    fn more_specific_allow_beats_less_specific_deny() {
        // deny wins only on ties; an exact allow beats a wildcard deny.
        let c = config_with(&["bash:readonly"], &[], &["bash:*"]);
        assert_eq!(evaluate(&c, "bash", "readonly"), Decision::Allow);
        assert_eq!(evaluate(&c, "bash", "mutate"), Decision::Deny);
    }

    #[test]
    fn no_match_fails_closed() {
        let c = config_with(&[], &[], &[]);
        assert_eq!(evaluate(&c, "bash", "anything"), Decision::Deny);
    }

    #[test]
    fn remote_wildcard_denies_every_remote_action() {
        let c = config_with(&[], &[], &["remote:*"]);
        assert_eq!(evaluate(&c, "remote", "ssh"), Decision::Deny);
        assert_eq!(evaluate(&c, "remote", "push"), Decision::Deny);
        assert_eq!(evaluate(&c, "remote", ""), Decision::Deny);
    }

    #[test]
    fn parse_rejects_malformed_patterns() {
        assert!(parse_pattern("bash").is_none());
        assert!(parse_pattern(":x").is_none());
        assert!(parse_pattern("bash:").is_none());
        assert!(parse_pattern("ba sh:x").is_none());
        assert!(parse_pattern("mcp.lark:*").is_some());
        assert!(parse_pattern("bash:mutate").is_some());
    }

    #[test]
    fn sealed_ops_match_exact_and_prefix() {
        let mut c = Config::default();
        c.sealed.ops = vec!["update".to_string(), "release:*".to_string()];
        assert!(op_is_sealed(&c, "update"));
        assert!(op_is_sealed(&c, "release:project-public"));
        assert!(!op_is_sealed(&c, "govern.skill.install"));
        assert_eq!(evaluate_op(&c, "update"), Decision::Sealed);
        assert_eq!(evaluate_op(&c, "doctor"), Decision::Deny);
    }

    /// Exhaustive wildcard table (AC-02 / design §9.1): every row is a
    /// (matrix, surface:action) pair with the required decision, including
    /// the "looks denied but must allow" and "looks allowed but must deny"
    /// negative cases. Semantics: most-specific match wins; on a true
    /// specificity tie, deny beats ask beats allow.
    #[test]
    fn wildcard_exhaustive_table() {
        let c = config_with(
            &["bash:readonly", "mcp:*", "git:diff", "git:push"],
            &["git:commit", "mcp:network", "bash:mutate"],
            &["remote:*", "git:*", "bash:mutate"],
        );
        // (surface, action, expected)
        let cases: &[(&str, &str, Decision)] = &[
            ("bash", "readonly", Decision::Allow),  // exact allow
            ("bash", "mutate", Decision::Deny),     // true tie (ask+deny same pattern) → deny
            ("git", "diff", Decision::Allow),       // exact allow beats wildcard deny
            ("git", "push", Decision::Allow),       // exact allow beats wildcard deny
            ("git", "commit", Decision::Ask),       // exact ask beats wildcard deny
            ("git", "rebase", Decision::Deny),      // wildcard deny, nothing more specific
            ("remote", "ssh", Decision::Deny),      // remote:* deny
            ("remote", "status", Decision::Deny),   // looks read-only, still deny
            ("mcp", "lark-doc", Decision::Allow),   // mcp:* allow, unknown server
            ("mcp", "network", Decision::Ask),      // exact ask beats wildcard allow
            ("credential", "read", Decision::Deny), // unscaffolded surface → deny
            ("read", "file", Decision::Deny),       // absent from matrix → deny
        ];
        for (surface, action, expected) in cases {
            let got = evaluate(&c, surface, action);
            assert_eq!(
                got, *expected,
                "wildcard table violation for {surface}:{action} (expected {:?}, got {:?})",
                expected, got
            );
        }
    }
}
