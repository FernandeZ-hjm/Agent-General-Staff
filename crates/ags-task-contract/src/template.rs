//! Canonical task-card skeleton (contract v3 §7.6).
//!
//! The card has exactly thirteen fields plus one fixed preamble line. Every
//! other policy fact lives in `ags.toml` or the protocol docs — cards never
//! re-paste fixed rules. Field order is stable for parser and cache
//! friendliness.

pub const CARD_HEADING: &str = "## 任务卡";

/// Field names in canonical order (the preamble `读取并遵守` precedes them).
pub const CARD_FIELDS: [&str; 13] = [
    "Contract ID:",
    "Executor:",
    "任务级别：",
    "任务：",
    "目标：",
    "验收标准：",
    "验证：",
    "能力上限：",
    "写边界：",
    "拓扑：",
    "停止条件：",
    "相关路径：",
    "交付方式：",
];

pub const VALID_EXECUTORS: [&str; 5] = ["Codex", "Claude Code", "Cursor", "OMP", "Other"];
pub const VALID_LEVELS: [&str; 3] = ["Light", "Medium", "Heavy"];
pub const VALID_TOPOLOGIES: [&str; 3] = ["single", "worktree", "parallel"];
pub const VALID_DELIVERY: [&str; 2] = ["ags-run", "manual"];

/// The canonical skeleton, as generated for handoff. Placeholders remain
/// only in free-text fields; every enumerated field carries a valid value so
/// the generated scaffold always passes the validator.
pub fn skeleton() -> String {
    let mut out = String::new();
    out.push_str(CARD_HEADING);
    out.push('\n');
    out.push_str("\n读取并遵守：\n- AGENTS.md\n");
    out.push_str("\nContract ID: tc-0123456789abcdef\n");
    out.push_str("\nExecutor: Other\n");
    out.push_str("\n任务级别：Light\n");
    out.push_str("\n任务：\n<一句话任务描述>\n");
    out.push_str("\n目标：\n- G-01: goal_1\n");
    out.push_str("\n验收标准：\n- AC-01 -> G-01: observable_result_1\n");
    out.push_str("\n验证：\n- V-01 -> AC-01: <verification command or explicit manual check>\n- EV-01 -> AC-01: <test result / diff summary / report path>\n");
    out.push_str("\n能力上限：-\n");
    out.push_str("\n写边界：仓库内\n");
    out.push_str("\n拓扑：single\n");
    out.push_str("\n停止条件：\n- <when to pause and report instead of continuing>\n");
    out.push_str("\n相关路径：\n- path_1\n");
    out.push_str("\n交付方式：ags-run\n");
    out
}
