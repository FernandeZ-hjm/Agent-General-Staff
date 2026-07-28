//! Canonical task-card field specification shared by every adapter.

#[derive(Debug, Clone, Copy)]
pub(crate) struct TaskField {
    pub header: &'static str,
    pub inline: bool,
    pub required_marker: Option<&'static str>,
    required_order: Option<u8>,
    render_order: Option<u8>,
}

const fn field(
    header: &'static str,
    inline: bool,
    required_marker: Option<&'static str>,
    required_order: Option<u8>,
    render_order: Option<u8>,
) -> TaskField {
    TaskField {
        header,
        inline,
        required_marker,
        required_order,
        render_order,
    }
}

/// Canonical fields remain in the historical compiler order so machine-facing
/// slot provenance stays deterministic. Rendering and validation order are
/// explicit properties of the same field specification.
pub(crate) const TASK_FIELDS: &[TaskField] = &[
    field("Contract ID:", true, Some("Contract ID:"), Some(1), Some(1)),
    field(
        "Handoff source:",
        true,
        Some("Handoff source:"),
        Some(2),
        Some(2),
    ),
    field("Executor:", true, Some("Executor:"), Some(3), Some(3)),
    field(
        "Runtime adapter:",
        true,
        Some("Runtime adapter:"),
        Some(4),
        Some(4),
    ),
    field(
        "Execution surface:",
        true,
        Some("Execution surface:"),
        Some(5),
        Some(5),
    ),
    field(
        "Execution mode:",
        true,
        Some("Execution mode:"),
        Some(6),
        Some(6),
    ),
    field(
        "Execution topology:",
        true,
        Some("Execution topology:"),
        Some(7),
        Some(7),
    ),
    field("任务级别：", true, Some("任务级别"), Some(8), Some(10)),
    field("Execution effort:", true, None, None, Some(8)),
    field(
        "Delegation planning:",
        true,
        Some("Delegation planning:"),
        Some(9),
        Some(9),
    ),
    field(
        "读取并遵守：",
        false,
        Some("读取并遵守："),
        Some(0),
        Some(0),
    ),
    field(
        "Review gate:",
        false,
        Some("Review gate:"),
        Some(10),
        Some(11),
    ),
    field("路径：", false, None, None, None),
    field("读取：", false, None, None, None),
    field("任务：", false, Some("任务："), Some(11), Some(12)),
    field("背景：", false, Some("背景："), Some(12), Some(13)),
    field("项目画像：", false, Some("项目画像："), Some(13), Some(14)),
    field("记忆胶囊：", false, Some("记忆胶囊："), Some(14), Some(15)),
    field("任务存档：", false, Some("任务存档："), Some(15), Some(16)),
    field("适用治理文档：", false, None, None, Some(17)),
    field(
        "目标文件夹路径：",
        false,
        Some("目标文件夹路径："),
        Some(16),
        Some(18),
    ),
    field("相关路径：", false, Some("相关路径："), Some(17), Some(19)),
    field(
        "本次任务相关文件：",
        false,
        Some("本次任务相关文件："),
        Some(18),
        Some(20),
    ),
    field("目标：", false, Some("目标："), Some(19), Some(21)),
    field("验收标准：", false, Some("验收标准："), Some(20), Some(22)),
    field("非目标：", false, Some("非目标："), Some(21), Some(23)),
    field("子任务编排：", false, None, None, Some(24)),
    field("实施要求：", false, None, None, Some(25)),
    field("关键路径：", false, None, None, None),
    field("验证：", false, Some("验证："), Some(22), Some(26)),
    field(
        "Verification gate:",
        false,
        Some("Verification gate:"),
        Some(23),
        Some(27),
    ),
    field("停止条件：", false, None, None, None),
    field("交付：", false, Some("交付："), Some(24), Some(28)),
];

pub(crate) fn task_field(header: &str) -> Option<&'static TaskField> {
    TASK_FIELDS.iter().find(|field| field.header == header)
}

pub(crate) fn rendered_fields() -> impl Iterator<Item = &'static TaskField> {
    let mut fields = TASK_FIELDS
        .iter()
        .filter(|field| field.render_order.is_some())
        .collect::<Vec<_>>();
    fields.sort_by_key(|field| field.render_order);
    fields.into_iter()
}

pub(crate) fn required_fields() -> impl Iterator<Item = &'static TaskField> {
    let mut fields = TASK_FIELDS
        .iter()
        .filter(|field| field.required_marker.is_some())
        .collect::<Vec<_>>();
    fields.sort_by_key(|field| field.required_order);
    fields.into_iter()
}
