/// Shared dispatch: `run`
fn cmd_run(
    path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
    current_task_approval: bool,
    format: &str,
) {
    let plan = runner::run_task_card(
        path,
        check_only,
        dry_run,
        approve_writes,
        current_task_approval,
    );

    match format {
        "json" => println!("{}", runner::render_json(&plan)),
        _ => println!("{}", runner::render_text(&plan)),
    }

    // Exit code: 0 if gate allows/confirms, 1 if stop or validation failed
    let should_exit_1 = plan.gate_decision == "stop" || !plan.validation_passed;
    if check_only && should_exit_1 {
        std::process::exit(1);
    }
    if !check_only && should_exit_1 {
        std::process::exit(1);
    }
}

// ── Verify dispatch ────────────────────────────────────────────────────────

pub(crate) fn run(
    path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
    current_task_approval: bool,
    format: &str,
) {
    cmd_run(
        path,
        check_only,
        dry_run,
        approve_writes,
        current_task_approval,
        format,
    )
}
