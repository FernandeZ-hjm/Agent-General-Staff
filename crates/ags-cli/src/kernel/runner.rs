/// Shared dispatch: `run`
pub(crate) fn run(
    path: &str,
    check_only: bool,
    dry_run: bool,
    approve_writes: bool,
    current_task_approval: bool,
    format: &str,
) {
    let plan = ags_task_contract::runner::run_task_card(
        path,
        check_only,
        dry_run,
        approve_writes,
        current_task_approval,
    );

    match format {
        "json" => println!("{}", ags_task_contract::runner::render_json(&plan)),
        _ => println!("{}", ags_task_contract::runner::render_text(&plan)),
    }

    // Exit code: stop / validation failure → 1 (both check-only and full run).
    let should_exit_1 = plan.gate_decision == "stop" || !plan.validation_passed;
    if should_exit_1 {
        std::process::exit(1);
    }
}
