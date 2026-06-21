use crate::context::AGS_VERSION;

pub(in crate::setup) fn claude_ags_command_content() -> String {
    format!(
        r#"---
description: AGS one-command setup, project onboarding, and governance
argument-hint: [setup|init|preflight|doctor|verify|request...]
---

# AGS

This is the post-install AGS operator surface. Route by the first token in `$ARGUMENTS`.

## `/ags setup`

Initialize this machine into AGS with one user command. Run these steps without asking for another confirmation unless credentials, sudo, or destructive replacement is required:

```bash
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v ags >/dev/null 2>&1; then
  echo "AGS CLI is not on PATH. Run the AGS one-line installer first, then retry /ags setup." >&2
  exit 127
fi

ags setup --yes --force --register-claude
ags verify --profile private

claude mcp list
```

Expected result: `ags`, `/ags`, and Claude Code MCP registration are ready on this machine.

## `/ags init`

Onboard the current repository into AGS governance with one user command:

```bash
ags init --target .
ags session preflight --for claude-code --target .
```

Aliases: `/ags onboard`, `/ags manage`, `/ags 纳管`.

## Other routes

- Empty or `preflight`: report the AGS preflight result and next allowed actions.
- `doctor`: run `ags doctor --target .` and summarize the findings.
- `verify`: run `ags verify --scope local --target .` and summarize the check results.
- Any other text: treat it as the user request. Prefer MCP `ags_preflight` first; if MCP is unavailable, run `ags session preflight --for claude-code --target .`. Complete AGS solution formation and do not generate an executable task card until the user explicitly asks for one.

Current AGS version expected by this command: {AGS_VERSION}.
"#
    )
}
pub(in crate::setup) fn codex_ags_command_skill_specs() -> &'static [(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)] {
    // Standard Codex front-stage AGS command skills: exactly setup → agents →
    // skill → init → doctor. `ags-capability` is intentionally NOT here — it is
    // the underlying cross-Agent visibility/sync layer (`ags capability ...` CLI
    // remains) and is retired from the front-stage set (see
    // `retired_codex_ags_skill_dirs`).
    &[
        (
            "ags-setup",
            "AGS Setup",
            "初始化本机 AGS 环境",
            "用 $ags-setup 初始化本机 AGS 环境。",
            "初始化本机 AGS 环境：运行 `ags setup --yes --force --register-claude`，然后用 `ags verify --profile private` 校验",
        ),
        (
            "ags-agents",
            "AGS Agents",
            "纳管本机 Agent 宿主",
            "用 $ags-agents 纳管本机 Agent 宿主。",
            "纳管本机 Agent 宿主：运行 `ags agents scan` 盘点宿主与 AGS MCP 注册，`ags agents govern` 生成 advise-only 接入方案，`ags agents verify --host <host>` 复核可见性",
        ),
        (
            "ags-skill",
            "AGS Skill",
            "管理第三方技能",
            "用 $ags-skill 管理第三方技能。",
            "管理第三方技能：运行 `ags skill` 查看概览，或运行 `ags skill --fix`、`ags skill scan`、`ags skill check`、`ags skill propose --action adopt --skill <name>` 生成纳管建议",
        ),
        (
            "ags-init",
            "AGS Init",
            "纳管当前项目",
            "用 $ags-init 纳管当前项目。",
            "纳管当前仓库：运行 `ags init --target .`，然后运行 `ags session preflight --for codex --target .`",
        ),
        (
            "ags-doctor",
            "AGS Doctor",
            "诊断 AGS 状态",
            "用 $ags-doctor 诊断 AGS 状态。",
            "诊断 AGS 安装和项目状态：运行 `ags doctor --target .` 并优先汇总失败项",
        ),
    ]
}
pub(in crate::setup) fn codex_ags_command_skill_content(
    name: &str,
    display_name: &str,
    summary: &str,
) -> String {
    let route = name.strip_prefix("ags-").unwrap_or(name);
    format!(
        r#"---
name: "{name}"
description: "当用户提到 /ags {route}、{display_name}、AGS {route}，或需要{summary}时使用。"
---

# {display_name}

这是 Codex 顶层 AGS 命令技能，用来把明确的 AGS 操作路由到已安装的 `ags` CLI 和 AGS 初始化门禁。

## 必须先执行

对目标仓库先运行 AGS preflight：

```bash
ags session preflight --for codex --target .
```

如果目标项目不明确，先询问仓库路径，不要误把桌面工作区当成项目。

## 路由

{summary}.

## 安全边界

不要绕过 AGS 做临时初始化。除非用户明确要求生成任务卡，否则不要生成可执行任务卡。

此技能期望的 AGS 版本：{AGS_VERSION}。
"#
    )
}
pub(in crate::setup) fn codex_ags_command_skill_agent_metadata_content(
    display_name: &str,
    short_description: &str,
    default_prompt: &str,
) -> String {
    format!(
        r#"interface:
  display_name: "{display_name}"
  short_description: "{short_description}"
  default_prompt: "{default_prompt}"

policy:
  allow_implicit_invocation: true
"#
    )
}
