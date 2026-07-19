//! Protocol safety assertion checks.
//!
//! This module verifies that critical protocol safety rules (assertions)
//! are present and not contradicted in target suite protocol files.
//! Unlike content drift, which compares section-by-section against a
//! source, assertion checks scan for specific invariant signatures.
//!
//! Missing or contradicted safety assertions are ALWAYS `Severity::Fail`,
//! even for public-full sanitized targets where normal content drift may be
//! allowlisted.
//!
//! # Design
//!
//! Each assertion checks a set of "required phrases" (all must be present
//! in the target file) and "contradiction phrases" (none may be present).
//! This is keyword-based, not semantic — it catches the most common
//! cases of missing or inverted rules without needing NLP.

use crate::types::{self, Drift, DriftKind, ProjectKind, Severity};
use std::fs;
use std::path::Path;

// ── Assertion definition ─────────────────────────────────────────────────

struct Assertion {
    /// Stable identifier for this assertion (used in error codes).
    id: &'static str,
    /// Human-readable description of the safety rule.
    description: &'static str,
    /// Protocol file that should contain this rule.
    file: &'static str,
    /// Phrases that MUST appear in the target file.
    required_phrases: &'static [&'static str],
    /// Phrases that MUST NOT appear (indicate contradiction).
    contradiction_phrases: &'static [&'static str],
}

/// All protocol safety assertions that must be present in every target.
fn all_assertions() -> Vec<Assertion> {
    vec![
        // ── A1: ultracode is thinking intensity only ─────────────────
        Assertion {
            id: "ultracode-thinking-intensity-only",
            description: "\"ultracode\"/\"Execution effort: ultracode\" does NOT change permission mode, parallelism, or launch args",
            file: "protocol/runtime-adapters.md",
            required_phrases: &[
                "thinking intensity only",
                "does not change permission",
            ],
            contradiction_phrases: &[
                "ultracode grants write",
                "ultracode enables parallelism",
                "ultracode mode allows editing",
            ],
        },
        // ── A2: task level ≠ permission; executable cards run directly ─
        Assertion {
            id: "heavy-level-not-permission-downgrade",
            description: "Task level is a risk/review tier, not the execution authority: task level keeps the card's declared permission mode and never rewrites it. Permission mode has exactly two states; execute-and-verify runs directly.",
            file: "protocol/runtime-adapters.md",
            required_phrases: &[
                "task level does not change the permission mode",
                "plan-only and execute-and-verify",
                "execute-and-verify runs directly",
            ],
            contradiction_phrases: &[
                "downgraded to plan-only",
                "unlocks heavy",
            ],
        },
        // ── A3: plan-only must not produce write-type launch args ─────
        Assertion {
            id: "planonly-no-write-launch-args",
            description: "plan-only must never produce write-type launch args; active parallelism and headless/background-agent must be stripped",
            file: "protocol/runtime-adapters.md",
            required_phrases: &["write-type launch args", "strip", "plan-only"],
            contradiction_phrases: &["plan-only may produce --worktree"],
        },
        // ── A4: runner must consume resolve-policy JSON, not raw fields ─
        Assertion {
            id: "runner-must-consume-resolved-policy",
            description: "Runner must consume resolve-policy JSON (effective_*, allowed_launch_args) and must NOT derive launch flags from raw task-card fields",
            file: "protocol/runtime-adapters.md",
            required_phrases: &[
                "allowed_launch_args",
                "effective_permission_mode",
                "must not",
            ],
            contradiction_phrases: &[
                "read raw task-card fields",
                "may derive flags directly from",
                "should derive flags directly from",
            ],
        },
        // ── A5: Runner prepares a plan; the host executes ───────────
        Assertion {
            id: "runner-is-non-executing-plan-preparer",
            description: "AGS Runner never launches or executes an executor; an allowed plan returns HOST_EXECUTION_REQUIRED",
            file: "protocol/runtime-adapters.md",
            required_phrases: &["runner never launches", "host_execution_required"],
            contradiction_phrases: &[
                "runner launches the executor",
                "ags run executes the task",
                "runner writes the final receipt",
            ],
        },
    ]
}

// ── Public entry point ───────────────────────────────────────────────────

/// Run all protocol safety assertion checks for a target.
///
/// Returns a list of drift findings for missing or contradicted assertions.
/// These are always `Severity::Fail` regardless of target kind.
pub fn check_assertions(
    target_root: &Path,
    target_name: &str,
    _target_kind: &ProjectKind,
) -> Vec<Drift> {
    let mut drifts = Vec::new();

    // Group assertions by file so we only read each file once
    let assertions = all_assertions();
    let mut files: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for a in &assertions {
        files.insert(a.file);
    }

    for file in &files {
        let file_path = target_root.join(file);
        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => {
                // File doesn't exist in target — report ALL assertions
                // for this file as missing
                for a in assertions.iter().filter(|a| a.file == *file) {
                    drifts.push(Drift::new(
                        types::error_code::INVARIANT_MISSING,
                        DriftKind::InvariantMissing,
                        Severity::Fail,
                        a.file,
                        vec![],
                        format!(
                            "{}: safety assertion '{}' cannot be verified — file missing in target '{}'",
                            target_name, a.id, target_name
                        ),
                        &format!(
                            "restore {} in {} and ensure it contains the rule: {}",
                            a.file, target_name, a.description
                        ),
                    ));
                }
                continue;
            }
        };

        let normalized_content = normalize_assertion_text(&content);

        for a in assertions.iter().filter(|a| a.file == *file) {
            // Check for required phrases
            let mut missing_required: Vec<&str> = Vec::new();
            for phrase in a.required_phrases {
                if !normalized_content.contains(&normalize_assertion_text(phrase)) {
                    missing_required.push(phrase);
                }
            }

            if !missing_required.is_empty() {
                drifts.push(Drift::new(
                    types::error_code::INVARIANT_MISSING,
                    DriftKind::InvariantMissing,
                    Severity::Fail,
                    a.file,
                    vec![],
                    format!(
                        "{}: safety assertion '{}' appears to be missing or incomplete. Missing signatures: {}",
                        target_name,
                        a.id,
                        missing_required.join(", ")
                    ),
                    &format!(
                        "ensure {} in {} contains the rule: {}",
                        a.file, target_name, a.description
                    ),
                ));
                continue; // Skip contradiction check if missing
            }

            // Check for contradiction phrases
            let mut found_contradictions: Vec<&str> = Vec::new();
            for phrase in a.contradiction_phrases {
                if normalized_content.contains(&normalize_assertion_text(phrase)) {
                    found_contradictions.push(phrase);
                }
            }

            if !found_contradictions.is_empty() {
                drifts.push(Drift::new(
                    types::error_code::INVARIANT_CONTRADICTED,
                    DriftKind::InvariantContradicted,
                    Severity::Fail,
                    a.file,
                    vec![],
                    format!(
                        "{}: safety assertion '{}' appears to be contradicted. Found contradictory signatures: {}",
                        target_name,
                        a.id,
                        found_contradictions.join(", ")
                    ),
                    &format!(
                        "review {} in {} and ensure it correctly states: {}",
                        a.file, target_name, a.description
                    ),
                ));
            }
        }
    }

    drifts
}

fn normalize_assertion_text(text: &str) -> String {
    text.to_lowercase()
        .replace(['`', '*', '_'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_target(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "assert-test-{}-{}-{}",
            name,
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(dir.join("protocol")).unwrap();
        dir
    }

    fn write_protocol(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn good_runtime_adapters() -> &'static str {
        "\
# Runtime Adapters

## Execution-Policy Resolver

The execution-policy crate is the resolver that reads a validated task card
and produces a structured resolution of how the task should actually execute.
Ultracode is thinking intensity only — it does not change permission mode,
does not enable parallelism, and does not add launch args.

## Key resolution rules

Task level does not change the permission mode; the task level is a risk/review
tier and the permission mode is the execution authority. Permission modes are
plan-only and execute-and-verify; execute-and-verify runs directly. The
runner must consume the resolved execution policy JSON and copy allowed_launch_args
and effective_permission_mode into a non-executing LaunchPlan. Runners must not
derive launch flags directly from raw task-card fields or launch a process.

Runner never launches. An allowed LaunchPlan returns HOST_EXECUTION_REQUIRED.

plan-only must never produce
write-type launch args. Active parallelism flags and headless/background-agent
must be stripped or stopped when the effective permission mode forbids writes.
"
    }

    // ── positive cases ───────────────────────────────────────────────

    #[test]
    fn all_assertions_pass_for_good_content() {
        let target = temp_target("all_pass");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            good_runtime_adapters(),
        );

        let drifts = check_assertions(&target, "test-target", &ProjectKind::Stable);

        assert!(
            drifts.is_empty(),
            "expected no assertion failures, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn stable_target_with_all_assertions_passes() {
        let target = temp_target("stable_ok");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            good_runtime_adapters(),
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);
        assert!(drifts.is_empty());

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn public_target_with_all_assertions_passes() {
        let target = temp_target("public_ok");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            good_runtime_adapters(),
        );

        let drifts = check_assertions(&target, "public", &ProjectKind::PublicCoreOnly);
        assert!(drifts.is_empty());

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn markdown_emphasis_does_not_hide_required_phrases() {
        let target = temp_target("markdown_emphasis");
        let content = "\
# Runtime Adapters

`ultracode` is thinking intensity only. It does **not** change permission mode,
does not enable parallelism, and does not add launch args.
Task level does **not** change the permission mode. Permission modes are **plan-only and execute-and-verify**; **execute-and-verify runs directly**.
plan-only must never produce write-type launch args and must strip active parallelism.
The runner must consume allowed_launch_args and effective_permission_mode into
a non-executing LaunchPlan. It must NOT derive flags directly from raw task-card
fields or launch a process.
Runner never launches; an allowed plan returns HOST_EXECUTION_REQUIRED.
";
        write_protocol(&target, "protocol/runtime-adapters.md", content);

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        assert!(
            drifts.is_empty(),
            "markdown emphasis and negated derive text should pass: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        let _ = fs::remove_dir_all(&target);
    }

    // ── negative: missing safety assertion → FAIL ───────────────────

    #[test]
    fn missing_ultracode_assertion_produces_fail() {
        let target = temp_target("missing_ultracode");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nTask level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\nplan-only must not produce write-type launch args and must strip parallelism.\nRunners must consume allowed_launch_args and effective_permission_mode from the resolved policy.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        let ultracode_drift = drifts.iter().find(|d| {
            d.code == types::error_code::INVARIANT_MISSING && d.message.contains("ultracode")
        });
        assert!(
            ultracode_drift.is_some(),
            "expected ultracode invariant missing, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(ultracode_drift.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn missing_heavy_assertion_produces_fail() {
        let target = temp_target("missing_heavy");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nUltracode is thinking intensity only — it does not change permission mode.\nplan-only must not produce write-type launch args and must strip parallelism.\nRunners must consume allowed_launch_args and effective_permission_mode from the resolved policy.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        // The A2 assertion requires the task-level boundary, the exact two-mode
        // vocabulary, and direct execution semantics. This fixture only contains
        // the plan-only writability rule, so A2 is reported missing.
        let heavy_drift = drifts.iter().find(|d| d.message.contains("heavy"));
        assert!(
            heavy_drift.is_some(),
            "expected heavy assertion missing, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(heavy_drift.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn missing_planonly_assertion_produces_fail() {
        let target = temp_target("missing_planonly");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nUltracode is thinking intensity only — it does not change permission mode.\nTask level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\nRunners must consume allowed_launch_args and effective_permission_mode from the resolved policy.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        // The plan-only assertion requires write-arg and strip language; this
        // fixture omits it, so the assertion is reported missing.
        let ro_drift = drifts.iter().find(|d| d.message.contains("planonly"));
        assert!(
            ro_drift.is_some(),
            "expected plan-only assertion missing, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(ro_drift.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn missing_resolved_policy_assertion_produces_fail() {
        let target = temp_target("missing_runner");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nUltracode is thinking intensity only — it does not change permission mode.\nTask level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\nplan-only must not produce write-type launch args and strip parallelism.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        let runner_drift = drifts
            .iter()
            .find(|d| d.message.contains("runner-must-consume"));
        assert!(
            runner_drift.is_some(),
            "expected runner assertion missing, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(runner_drift.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    // ── negative: missing file → FAIL ────────────────────────────────

    #[test]
    fn missing_runtime_adapters_file_produces_fails_for_all_assertions() {
        let target = temp_target("missing_file");
        // Don't write runtime-adapters.md at all

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        // All 5 assertions live in runtime-adapters.md, so all 5 should fail
        assert_eq!(drifts.len(), 5);
        for d in &drifts {
            assert_eq!(d.severity, Severity::Fail);
            assert_eq!(d.kind, DriftKind::InvariantMissing);
        }

        let _ = fs::remove_dir_all(&target);
    }

    // ── negative: contradiction → FAIL ───────────────────────────────

    #[test]
    fn contradicted_assertion_produces_invariant_contradicted() {
        let target = temp_target("contradicted");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nUltracode is thinking intensity only — it does not change permission mode.\nUltracode grants write access and enables parallelism for all tasks.\nTask level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\nplan-only must not produce write-type launch args and must strip parallelism.\nRunners must consume allowed_launch_args and effective_permission_mode from the resolved policy JSON.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        let contradicted = drifts
            .iter()
            .find(|d| d.kind == DriftKind::InvariantContradicted);
        assert!(
            contradicted.is_some(),
            "expected invariant contradicted for ultracode-grants-write, got: {:?}",
            drifts
                .iter()
                .map(|d| (&d.kind, &d.message))
                .collect::<Vec<_>>()
        );
        assert_eq!(contradicted.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn contradicted_heavy_downgrade_invariant_fails() {
        // Even with the A2 required phrases present, a level-driven downgrade
        // must be caught as an A2 contradiction.
        let target = temp_target("contradicted_heavy_downgrade");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\n\
             Ultracode is thinking intensity only — it does not change permission mode and does not enable parallelism.\n\
             Task level does not change the permission mode; permission modes are plan-only and execute-and-verify; execute-and-verify runs directly.\n\
             Heavy execute tasks are downgraded to plan-only.\n\
             plan-only must never produce write-type launch args and must strip parallelism.\n\
             Runners must consume allowed_launch_args and effective_permission_mode; they must not derive flags directly from raw task-card fields.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        let contradicted = drifts.iter().find(|d| {
            d.kind == DriftKind::InvariantContradicted
                && d.message.contains("heavy-level-not-permission-downgrade")
        });
        assert!(
            contradicted.is_some(),
            "expected A2 contradiction for the Heavy downgrade invariant, got: {:?}",
            drifts
                .iter()
                .map(|d| (&d.kind, &d.message))
                .collect::<Vec<_>>()
        );
        assert_eq!(contradicted.unwrap().severity, Severity::Fail);

        let _ = fs::remove_dir_all(&target);
    }

    // ── public target boundary ───────────────────────────────────────

    #[test]
    fn public_target_with_legal_redaction_but_safety_assertions_passes() {
        // Public can redact internal sections but MUST keep safety assertions
        let target = temp_target("public_legal");
        // Write a good runtime-adapters.md with safety assertions intact
        // plus some public-appropriate redactions (no internal paths)
        let content = "\
# Runtime Adapters

## Execution-Policy Resolver

The execution-policy resolver reads validated task cards.
Ultracode is thinking intensity only — it does not change permission mode,
does not enable parallelism, and does not add launch args.
Task level does not change the permission mode. Permission modes are plan-only
and execute-and-verify; execute-and-verify runs directly.
plan-only must never produce write-type launch args.
Active parallelism and headless must be stripped.
Runners must consume allowed_launch_args and effective_permission_mode
from the resolved policy JSON. Runners must not derive flags directly
from raw task-card fields.
Runner never launches. An allowed LaunchPlan returns HOST_EXECUTION_REQUIRED.
";
        write_protocol(&target, "protocol/runtime-adapters.md", content);

        let drifts = check_assertions(&target, "public", &ProjectKind::PublicCoreOnly);
        assert!(
            drifts.is_empty(),
            "public target with safety assertions intact should pass, got: {:?}",
            drifts.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn public_target_missing_safety_assertion_still_fails() {
        // Even for public, missing safety assertion = FAIL
        let target = temp_target("public_missing");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\n## Public Distribution\n\nThis is a public distribution with internal details redacted.\n",
        );

        let drifts = check_assertions(&target, "public", &ProjectKind::PublicCoreOnly);

        assert!(!drifts.is_empty());
        for d in &drifts {
            assert_eq!(
                d.severity,
                Severity::Fail,
                "public target must FAIL on missing safety assertions"
            );
        }

        let _ = fs::remove_dir_all(&target);
    }

    // ── no false positive for unrelated content ─────────────────────

    #[test]
    fn unrelated_file_changes_do_not_trigger_assertion_failures() {
        let target = temp_target("unrelated");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            good_runtime_adapters(),
        );
        // Write another protocol file that has nothing to do with assertions
        write_protocol(
            &target,
            "protocol/task-card-template.md",
            "# Task Card Template\n\n## Usage\n\nSome template content.\n",
        );

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);
        assert!(drifts.is_empty());

        let _ = fs::remove_dir_all(&target);
    }

    // ── assertion check does not produce resolver decision fields ────

    #[test]
    fn assertion_drifts_do_not_contain_resolver_decision_fields() {
        let target = temp_target("no_resolver_leak");
        write_protocol(
            &target,
            "protocol/runtime-adapters.md",
            "# Runtime Adapters\n\nUltracode is thinking intensity only — it does not change permission mode.\nplan-only must not produce write-type launch args and must strip parallelism.\nRunners must consume allowed_launch_args and effective_permission_mode.\n",
        );
        // ^^ Missing heavy assertion — triggers INVARIANT_MISSING

        let drifts = check_assertions(&target, "stable", &ProjectKind::Stable);

        // Verify the drifts are about INVARIANT_MISSING (assertions),
        // not about resolver decisions. Assertion IDs may contain protocol
        // terms (plan-only, downgrade) as identifiers — that's normal.
        // The key invariant: workflow-sync-check never produces
        // ResolvedExecutionPolicy, effective_permission_mode decisions,
        // or launch args.
        for d in &drifts {
            assert_eq!(d.kind, DriftKind::InvariantMissing);
            // Must NOT contain resolver output field names
            for resolver_term in &[
                "ResolvedExecutionPolicy",
                "stop_before_launch",
                "was_downgraded",
            ] {
                assert!(
                    !d.message.contains(resolver_term),
                    "assertion drift must not contain resolver decision field: {}",
                    resolver_term
                );
            }
        }

        let _ = fs::remove_dir_all(&target);
    }
}
