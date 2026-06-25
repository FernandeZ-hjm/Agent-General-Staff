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

    // Exit code: stop / validation failure → 1 (both check-only and full run).
    let should_exit_1 = plan.gate_decision == "stop" || !plan.validation_passed;
    if should_exit_1 {
        std::process::exit(1);
    }
    // `--check-only` is a pure gate query: its exit code is the contract. A
    // confirmation-gated card must exit distinct from `allow` (2, not 0) so that
    // exit-code-based callers cannot treat a Heavy confirmation card as runnable
    // without honoring the confirmation gate. The launch-plan / dry-run paths
    // still exit 0 (they produced a plan; the confirm requirement rides in it).
    if check_only && plan.gate_decision == "confirm" {
        std::process::exit(2);
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
