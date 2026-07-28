# AGS Task Handoff Rules

## 生成门槛

仅在用户明确要求交接/任务卡，且已存在确认、封闭的 handoff contract 时生成任务卡。
调用 compiler 时使用 `--task-card-requested --confirmed-handoff-contract`；宿主 Plan mode
最终产物使用 `--host-plan-mode-final --confirmed-handoff-contract`。方案确认不等于修改授权。

任务卡必须采用 `protocol/task-card-template.md` 的唯一 canonical 骨架，第一条非空行是
`## 任务卡`。已有卡先 validate-first；合法卡直接进入 policy/gate/LaunchPlan，非法卡停止，
不得回落到 solution formation 或重新编译。

任务卡权限由 `Execution mode`、`Execution topology`、`Delegation planning`
共同定义。Heavy 仅增加独立 review gate；
发布、外部写入、凭据、迁移、破坏性操作和受保护路径仍单独判断。

任务卡至少包含 Executor、Runtime adapter、Execution surface、Execution mode、
Execution topology、目标、背景、非目标、硬性要求、相关路径、Verification gate 和交付报告格式。
`[skill: xxx]` 仅来自已确认的精确 `skill_id`，并须通过 registry、invoke hint 和当前机器
snapshot 三闸。交付报告须遵循 `claude-delivery-report` 或项目等价协议。

## OMP Plan 单卡出口

1. 先 `ags_preflight(agent=omp)`，读取 current-host，再以 `solution_formation` 提交 typed
   `HostRouteProposal`。
2. 关键项未决时保持 `solution_state=open`，不得调用 TaskCompile。
3. 方案封闭后，最终计划直接输出唯一 canonical `## 任务卡`，不再附加第二份 prose plan。
4. 任务卡只是 Plan UI 的待激活 artifact；生成本身不授权修改。
5. 用户选择执行后先退出 Plan mode，再原样派发任务卡正文与 `task_card_hash`；不得重写。
6. 执行 Agent validate-first；hash、target 或用户激活证据不一致时停止。

## 输出门禁

只要最终输出包含 `Executor: Claude Code`，就必须交付可执行任务卡块。含内嵌代码块时，
外层使用四反引号或四波浪线。用 `ags task validate <task-card>` 校验。
