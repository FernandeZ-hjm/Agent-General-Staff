#[cfg(test)]
mod tests {
    use super::super::merge::{
        describe_order, merge_codex_memory_lifecycle, merge_memory_capture, merge_memory_start,
        MergeOutcome,
    };

    #[test]
    fn memory_start_merge_is_idempotent() {
        let mut settings = serde_json::json!({});
        assert_eq!(
            merge_memory_start(&mut settings, "python3 context-memory-start.py"),
            MergeOutcome::Wired
        );
        assert_eq!(
            merge_memory_start(&mut settings, "python3 context-memory-start.py"),
            MergeOutcome::AlreadyPresent
        );
    }

    #[test]
    fn memory_capture_restores_guard_and_is_idempotent() {
        let mut settings = serde_json::json!({});
        assert_eq!(
            merge_memory_capture(&mut settings, "python3 claude-stop-memory-capture.py"),
            MergeOutcome::Wired
        );
        assert_eq!(describe_order(&settings), "raw-guard → memory-capture");
        assert_eq!(
            merge_memory_capture(&mut settings, "python3 claude-stop-memory-capture.py"),
            MergeOutcome::AlreadyPresent
        );
    }

    #[test]
    fn codex_lifecycle_uses_native_start_and_close_events() {
        let mut settings = serde_json::json!({});
        assert_eq!(
            merge_codex_memory_lifecycle(
                &mut settings,
                "python3 context-memory-start.py",
                "python3 claude-stop-memory-capture.py",
            ),
            MergeOutcome::Wired
        );
        assert!(settings["hooks"]["SessionStart"].is_array());
        assert!(settings["hooks"]["SessionEnd"].is_array());
    }
}
