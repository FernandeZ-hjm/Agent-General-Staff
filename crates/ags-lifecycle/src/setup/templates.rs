use super::AGS_VERSION;

pub(in crate::setup) fn host_entry_policy_content() -> String {
    format!(
        r#"# AGS Host Entry Policy

Current protocol version: {AGS_VERSION}.

- In an AGS-governed project, call MCP `ags_preflight` before any other AGS tool; use `ags session preflight --for <agent> --target <repo>` only when MCP is unavailable.
- After preflight, read `ags://capabilities/current-host`. The host keeps the complete conversation context, performs the only natural-language interpretation, builds a typed `HostRouteProposal`, and submits it to the strictly read-only `ags_route_request`.
- Never send raw request text to AGS. Consume `RouteResolution`; load an exact admitted `SkillTarget`, invoke an exact admitted `McpTarget` through the host's connected MCP surface, or consume a closed `MachineCliTarget`. AGS never proxies a third-party MCP. Only `ags_apply_action` may consume a returned connection-held action.
- Existing canonical `## 任务卡` input validates first. A valid card proceeds to policy, gate, and LaunchPlan; an invalid card stops and never falls back to task-card generation.
- Task-card compilation requires a confirmed closed handoff contract plus either an explicit handoff request or the final host Plan-mode artifact. Authorized same-session direct edits remain host-native.
- In host Plan mode, keep `solution_state=open` while decisions remain unresolved. When the contract closes, run `ags task compile --host-plan-mode-final --confirmed-handoff-contract`; the final artifact is the single canonical `## 任务卡`, not a separate final-plan document.
- The Plan UI keeps that card pending user activation. When the user selects Execute, switch to Default/execution mode and dispatch the exact same card and `task_card_hash` to the execution Agent; do not regenerate or rewrite it. The execution Agent validates the existing card first.
"#
    )
}

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

ags setup --yes --force --lifecycle-hosts detected
ags doctor
```

Expected result: the AGS runtime, `/ags`, and required suite Skills are verified only across the detected, approved Host set. Host MCP registration is owned by the selected `@agent-governance-suite/mcp` package integration, not by setup.

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
- Any other text: treat it as the user request. Prefer MCP `ags_preflight` first; if MCP is unavailable, run `ags session preflight --for claude-code --target .`. Read the preflight-bound current-host capability resource, use complete conversation context to create a typed `HostRouteProposal`, and submit it to strictly read-only `ags_route_request`. Never send raw request text or reclassify it in Compiler, Policy, Gate, Runner, or Capability Resolver. Load admitted Skills through the host and invoke admitted MCP targets through the host's connected MCP surface; AGS never proxies third-party MCPs. Only `ags_apply_action` may consume a returned connection-held action. A confirmed same-session direct edit stays host-native and does not regenerate a plan or task card; an existing canonical task card validates first. Explicit handoff uses `--task-card-requested --confirmed-handoff-contract`. In host Plan mode, the decision-complete final artifact uses `--host-plan-mode-final --confirmed-handoff-contract` and is the canonical task card itself. If solution work is unresolved or reopened, remain in solution formation and do not compile.

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
    // skill → init → doctor. `ags-capability` is intentionally NOT here — the
    // explicit static-snapshot CLI (`ags capability ...`) remains available
    // without a second front-stage command skill (see
    // `retired_codex_ags_skill_dirs`). Private-only command skills (e.g. the
    // public-edition sync skill) are machine-local and never generated here.
    // Codex reads these bodies from the shared `.agents/skills` projection;
    // setup must not restore the retired `.codex/skills` duplicate.
    &[
        (
            "ags-setup",
            "AGS Setup",
            "初始化本机 AGS 环境",
            "用 $ags-setup 初始化本机 AGS 环境。",
            "初始化本机 AGS 环境：先运行 `ags setup` 查看宿主，再运行 `ags setup --yes --force --lifecycle-hosts <ids|detected>` 至少选择当前宿主，然后用 `ags doctor` 校验",
        ),
        (
            "ags-agents",
            "AGS Agents",
            "纳管本机 Agent 宿主",
            "用 $ags-agents 纳管本机 Agent 宿主。",
            "纳管本机 Agent 宿主：运行 `ags agents scan` 盘点宿主与 AGS MCP 注册，先用 `ags agents govern --agent <host>` 预览，再经用户确认运行 `--apply` 安装该宿主的 AGS 原生记忆生命周期适配器；MCP 注册仍为 advise-only；最后用 `ags agents verify --host <host>` 复核",
        ),
        (
            "ags-skill",
            "AGS Skill",
            "管理第三方技能",
            "用 $ags-skill 管理第三方技能。",
            "管理第三方技能：用 `ags skill recommend` 浏览推荐目录，或向 `ags skill inspect/install` 提供任意 GitHub 来源；审阅 Plan 后用精确 plan hash 确认安装或更新，AGS 会事务化刷新受影响宿主快照并验证路由，最后用 `ags skill status` 或 `ags skill verify <skill-id>` 复核",
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
