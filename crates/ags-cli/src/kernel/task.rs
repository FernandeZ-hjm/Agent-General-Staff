use crate::cli::TaskAction;
use std::path::PathBuf;

/// Shared dispatch: `task validate` / `task-card-validator`
pub(crate) fn cmd_task_validate(paths: &[String]) {
    let paths: Vec<String> = if paths.is_empty() {
        vec!["-".to_string()]
    } else {
        paths.to_vec()
    };
    let ok = task_card_validator::validate_files(&paths);
    if !ok {
        std::process::exit(1);
    }
}
/// Shared dispatch: `task compile` (M4)
fn cmd_task_compile(
    path: &str,
    format: &str,
    output: &str,
    check_only: bool,
    task_card_requested: bool,
    confirmed_handoff_contract: bool,
) {
    use std::io::Read;

    if check_only && output == "card" {
        eprintln!("task compile: --check-only cannot be combined with --output card");
        std::process::exit(2);
    }

    if !task_card_requested && output == "card" {
        eprintln!("task compile: --task-card-requested is required for --output card");
        eprintln!(
            "  The user must explicitly issue a task-card instruction before an executable card can be generated."
        );
        eprintln!("  Use --task-card-requested after receiving: \"生成任务卡\", \"按这个方案出任务卡\", \"交给 Claude Code 执行\", etc.");
        std::process::exit(1);
    }

    if !confirmed_handoff_contract && output == "card" {
        eprintln!("task compile: --confirmed-handoff-contract is required for --output card");
        eprintln!(
            "  Confirm the solution/diagnosis, scope, verification, and handoff contract before compiling."
        );
        std::process::exit(1);
    }

    let display_path = if path == "-" {
        "(stdin)".to_string()
    } else {
        path.to_string()
    };

    // Read input
    let content = if path == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("{}: 读取失败 — {}", display_path, e);
            std::process::exit(1);
        }
        buf
    } else {
        match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 读取失败 — {}", display_path, e);
                std::process::exit(1);
            }
        }
    };

    // Determine project root (current directory)
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // Compile
    let (compiled_card, report) = task_compiler::compile_with_contract(
        &content,
        &project_root,
        check_only,
        task_card_requested,
        confirmed_handoff_contract,
    );

    // Validate the compiled card using the canonical validator
    let (validation_passed, validation_errors) = if !report.missing_slots.is_empty() {
        // Can't validate — missing slots
        (
            false,
            vec![format!(
                "Missing required slots: {}",
                report.missing_slots.join(", ")
            )],
        )
    } else {
        let errors = task_card_validator::validate(&compiled_card);
        (errors.is_empty(), errors)
    };

    // Build final report with actual validation results
    // Preserve gate fields from the compiler; override validation from the
    // canonical validator (which only runs meaningfully when executable_allowed).
    let final_report = task_compiler::CompileReport {
        schema_version: report.schema_version,
        compiled_task_card: report.compiled_task_card,
        slot_sources: report.slot_sources,
        missing_slots: report.missing_slots,
        assumptions: report.assumptions,
        validation_passed: if report.executable_allowed {
            validation_passed
        } else {
            report.validation_passed
        },
        validation_errors: if report.executable_allowed {
            validation_errors
        } else {
            report.validation_errors
        },
        check_only,
        task_card_requested: report.task_card_requested,
        confirmed_handoff_contract: report.confirmed_handoff_contract,
        executable_allowed: report.executable_allowed,
        block_reason: report.block_reason,
    };

    // check_only mode is inherently diagnostic — succeed if slots filled
    // regular mode requires executable_allowed AND validation_passed
    let success = if final_report.check_only {
        final_report.missing_slots.is_empty()
    } else {
        final_report.executable_allowed && final_report.validation_passed
    };

    // Card output is intended for direct piping into `ags task validate -`.
    // Never write a partial or invalid card to stdout.
    if output == "card" && !success {
        if !final_report.missing_slots.is_empty() {
            eprintln!(
                "{}: COMPILATION INCOMPLETE — {} missing slot(s)",
                display_path,
                final_report.missing_slots.len()
            );
            for slot in &final_report.missing_slots {
                eprintln!("  - {}", slot);
            }
        } else {
            eprintln!("{}: VALIDATION FAILED", display_path);
            for err in &final_report.validation_errors {
                eprintln!("  - {}", err);
            }
        }
        std::process::exit(1);
    }

    // Output
    if output == "card" {
        // Plain card output — directly pipeable to `ags task validate -`
        match format {
            "json" => {
                // JSON card-only: wrap in a minimal object for machine consumers
                let card_json = serde_json::json!({
                    "compiled_task_card": final_report.compiled_task_card,
                });
                if let Ok(json) = serde_json::to_string_pretty(&card_json) {
                    println!("{}", json);
                }
            }
            _ => {
                // Plain text card output — first line is ## 任务卡
                print!("{}", task_compiler::render_card_text(&final_report));
            }
        }
    } else {
        // Full report output
        match format {
            "json" => {
                println!("{}", task_compiler::render_report_json(&final_report));
            }
            _ => {
                println!("{}", task_compiler::render_report_text(&final_report));
            }
        }
    }

    // Exit code
    if success {
        // Success — exit 0
    } else if !final_report.missing_slots.is_empty() {
        eprintln!(
            "{}: COMPILATION INCOMPLETE — {} missing slot(s)",
            display_path,
            final_report.missing_slots.len()
        );
        for slot in &final_report.missing_slots {
            eprintln!("  - {}", slot);
        }
        std::process::exit(1);
    } else {
        eprintln!("{}: VALIDATION FAILED", display_path);
        for err in &final_report.validation_errors {
            eprintln!("  - {}", err);
        }
        std::process::exit(1);
    }
}

pub(crate) fn run(action: TaskAction) {
    match action {
        TaskAction::Validate { paths } => cmd_task_validate(&paths),
        TaskAction::Compile {
            path,
            format,
            output,
            check_only,
            task_card_requested,
            confirmed_handoff_contract,
        } => cmd_task_compile(
            &path,
            &format,
            &output,
            check_only,
            task_card_requested,
            confirmed_handoff_contract,
        ),
    }
}
