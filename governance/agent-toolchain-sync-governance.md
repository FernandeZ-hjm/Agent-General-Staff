# Agent Toolchain Sync Governance

Generated: 2026-05-23

这份文档描述我这套 Agent 开发环境的更新与同步治理方式。核心原则是：本地已经在使用的技能、插件和配置文件是主清单；远端 GitHub、插件缓存、CLI 官方源只作为候选更新来源。任何更新都必须先做只读检查，再经过人工 diff 和接纳决策，不能直接覆盖本地文件。

## 1. 治理目标

这套同步机制解决三个问题：

1. 我本地已经有大量技能、插件和快捷封装，其中不少做过本地化改造，不能被上游更新直接覆盖。
2. 一部分技能来自 GitHub、系统内置插件或 CLI 安装源，名字可能变过，安装方式也不统一，需要先建立可追踪的来源关系。
3. 后续要能低成本接收上游变化，但只把有价值、低风险、可解释的更新合并进本地开发套件。

因此，更新流程被拆成两层：

- 观察层：只读检查远端是否变化，生成报告，不改本地技能和插件。
- 接纳层：对具体条目做 diff、评估价值与风险，再由人工决定是否改本地版本。

## 1.1 三方 Agent 接入方式

这套治理 kit 已纳入 Multi-Agent Engineering Kit 的统一三方代理协议：

- Cursor：作为主开发代理，负责判断任务风险、生成任务卡、复核 proposal / patch / log。
- Codex：可作为诊断和方案收敛代理，同样只生成任务卡或审查结果，不绕过本协议。
- Claude Code：作为执行代理，只按任务卡运行 `tools/agent-toolchain/` 下的只读检查、diff 提案或接纳门禁。

三方都必须引用同一个 canonical 协议：`docs/agent-workflow/agent-task-protocol.md`。涉及本地技能/插件同步时，最终任务卡仍使用 `docs/agent-workflow/task-card-template.md` 的固定骨架，同时引用本文件；`docs/agent-workflow/task-cards/agent-toolchain-sync-task-card.md` 只作为 Agent Toolchain 动态填槽模块。

禁止为 Cursor、Codex、Claude Code 分别维护三套独立同步规范；规则进协议和治理文档，单次差异进任务卡。

开发套件级文档集中在 `docs/agent-workflow/agent-toolchain/`：

- `README.md`：治理 kit 的文档/脚本分层入口。
- `migration-mac-mini.md`：Mac mini 首次迁移与本地关联 runbook。
- `inventory-schema.md`：inventory、canonical map、source resolution、update plan 的字段约定。

## 2. 当前清单规模

当前主清单覆盖 90 个条目：

| 类别 | 数量 | 同步策略 |
|---|---:|---|
| 插件本体 | 8 | 只检查插件 manifest 或父插件来源，通过 Codex 插件机制更新 |
| 插件缓存技能 | 18 | 不单独同步，由父插件控制 |
| 飞书 / Lark 技能 | 26 | 本地为主，观察 `larksuite/cli` 和 well-known 地址 |
| Git lock 可追踪技能 | 8 | 可做 Git 远端比对，但更新前必须 diff |
| Git 直接本地远端技能 | 1 | `guizang-ppt-skill` 以 Git 版作为本地版 |
| 可能来自公开 GitHub 的技能 | 7 | 先验证路径和内容相似度，再决定能否同步 |
| 系统内置技能 | 5 | 记录系统版本，不手动修改 |
| 本地 overlay 快捷技能 | 6 | 只记录，不追上游 |
| 本地自定义 / 可选发布技能 | 7 | 本地长期保留，可选择发布到个人仓库 |

## 3. 分层同步模型

### A. 可自动观察，但不能自动改

这些条目有明确远端或父级来源，可以自动检查 remote head、版本号或 HTTP 状态：

- `plugin_manifest_version_check`
- `plugin_parent_version_check`
- `git_compare_from_skill_lock`
- `git_compare_local_remote`
- `codex_vendor_git_check`
- `lark_cli_version_and_repo_check`

允许动作：

- 读取本地 manifest / lock / metadata
- `git ls-remote` 查看远端 HEAD
- HTTP HEAD 检查公开源是否可达
- 生成报告和本地指纹

禁止动作：

- 直接写入 `$HOME/.agents/skills`
- 直接写入 `$HOME/.codex/skills`
- 直接写入 `$HOME/.codex/plugins/cache`
- 自动运行 `lark-cli update`
- 自动运行 `npx skills add/remove/update`
- 自动安装依赖或覆盖本地目录

### B. 需要人工 diff 才能接纳

这些条目有较高概率来自公开仓库，但本地内容可能做过封装、裁剪或迁移：

- `diagnose`
- `grill-with-docs`
- `improve-codebase-architecture`
- `prototype`
- `zoom-out`
- `systematic-debugging`
- `verification-before-completion`

处理方式：

1. 临时 clone 上游仓库到工作区外的临时目录。
2. 找到上游对应 skill 路径。
3. 对比本地 `SKILL.md`、脚本、引用文件和触发规则。
4. 将变化拆成三类：可吸收、需改写、拒绝。
5. 生成更新提案，由人工确认后再改本地。

### C. 本地 wrapper，只观察上游工具

`graphify-project-map` 属于这一类。它不是上游 skill 的直接拷贝，而是围绕 `safishamsi/graphify` 做的本地化封装。

同步策略：

- 观察 `https://github.com/safishamsi/graphify` 的变化。
- 只在 Graphify CLI 行为、参数、输出结构发生变化时评估 wrapper 是否要调整。
- 不把上游项目文件直接同步到本地 skill。

### D. 系统版或插件缓存，只记录不手改

这些条目由 Codex 系统或插件机制管理：

- `skill-creator`
- `imagegen`
- `openai-docs`
- `plugin-creator`
- `skill-installer`
- Browser / Chrome / GitHub / Figma / Documents / Presentations / Spreadsheets 等插件技能

同步策略：

- 本地仅记录版本、路径、指纹和父插件来源。
- 更新只能通过系统或插件机制完成。
- 插件缓存目录不作为人工维护目录。

### E. 本地自定义，保留本地权威

这些技能目前没有可靠公开来源，或明显属于本地创作：

- `claude-delivery-report`
- `claude-execution-prompt-maker`
- `产经破壁机-撰稿`
- `产经破壁机-选题策划`
- `六爻`
- `深度科技评论`
- `辐射塔罗牌`

同步策略：

- 不等待上游。
- 不做自动搜索式覆盖。
- 如需多设备同步或版本管理，可以发布到个人私有仓库，再把该仓库记录为本地权威远端。

## 4. 当前只读检查器

检查脚本：

```bash
tools/agent-toolchain/check-agent-toolchain-updates.sh
```

常用命令：

```bash
# 完全离线检查清单结构、路径和本地指纹
tools/agent-toolchain/check-agent-toolchain-updates.sh --no-network

# 联网检查远端 Git / HTTP 状态
tools/agent-toolchain/check-agent-toolchain-updates.sh

# 输出到指定报告
tools/agent-toolchain/check-agent-toolchain-updates.sh --report /tmp/agent-toolchain-update-report.md
```

报告文件：

```text
./tools/agent-toolchain/reports/agent-toolchain-update-report.md
```

最近一次联网检查结果：

| 指标 | 数量 |
|---|---:|
| 总条目 | 90 |
| Git 远端检查 OK | 60 |
| Git 远端检查错误 | 10 |
| HTTP 检查错误 | 1 |

剩余错误主要来自不可匿名访问或非公开的 OpenAI 插件来源，以及个别飞书 well-known URL 的不可达状态。它们不代表本地技能损坏，只表示这些来源不能直接作为自动同步源。

## 5. 更新接纳流程

每次想接收更新时，按这个顺序执行：

1. 运行只读检查器，刷新 `agent-toolchain-update-report.md`。
2. 查看 report 中 remote head、版本号或状态变化。
3. 只挑选有实际收益的条目进入 diff。
4. 用 `propose-agent-toolchain-update.sh <canonical_id>` 生成 diff 提案，写入 `tools/agent-toolchain/proposals/`，提案要说明：
   - 上游变化是什么
   - 本地改动是什么
   - 是否会破坏现有触发规则
   - 是否影响成本、模型调度或插件调用
   - 是否需要迁移名称、路径或 README
5. 用 `accept-agent-toolchain-update.sh <proposal_md> --dry-run` 跑接纳门禁：
   - 它会校验本地指纹是否仍与提案一致（不一致直接拒绝）。
   - 输出 local-only / upstream-only / 需要人工判断的文件清单。
   - 对 `plugin/*` 与 `skill/plugin-cache/*` 硬性拒绝，保护父插件机制不被绕过。
   - 全程不写本地技能目录，也不写任何 patch。
6. 在 dry-run 报告复核通过后，再用 `accept-agent-toolchain-update.sh <proposal_md> --confirm-generate-patch` 生成只读 patch，落地到 `tools/agent-toolchain/acceptance-patches/`，方向是 `local -> upstream`，仅供人工 `patch -p0 --dry-run` 预览。脚本本身仍不执行 `patch -p0`。
7. 人工确认后，由人工执行 `patch -p0 < <patch>` 或手工 merge；之后重新运行只读检查器和功能验证。
8. 把接纳结果（commit、版本号、回滚指引）写回清单或变更记录，并复算技能哈希基线。

## 6. 效价比原则

这套开发套件的成本控制不是只看“哪个模型便宜”，而是看一次任务从触发到交付的总成本。

判断一个更新是否值得接纳时，看四个维度：

| 维度 | 接纳倾向高 | 接纳倾向低 |
|---|---|---|
| 任务频率 | 高频开发、调试、交付流程 | 很少触发的边缘技能 |
| 失败成本 | 能减少返工、误提交、误调用 | 只是文字风格变化 |
| 协同收益 | 能改善 agent 分工、工具调用、验证闭环 | 只增加复杂提示词 |
| 维护成本 | 上游清晰、diff 小、回滚容易 | 来源不明、路径漂移、破坏本地封装 |

默认决策：

- 高频基础能力优先稳定，例如调试、验证、提交、浏览器测试。
- 插件类能力优先走官方插件机制，不在缓存里手工修。
- Lark 技能以本地可用为第一目标，上游变化只作为参考。
- 写作、占卜、评论等强本地风格技能，不追求外部同步。
- 对不确定来源，不为了“看起来更新”而引入风险。

## 7. 同步工作流的三段式实现

当前同步工具已经进入三段式更新工作流，每一段都是只读或只生成提案，不会自动应用：

1. `check`：只读检查远端和本地指纹。
2. `diff-propose`：对指定条目拉取临时上游并生成 diff 提案。
3. `accept`：已具备 dry-run / patch 生成，不自动应用。

其中 `accept` 必须保持显式人工触发，不能由定时任务自动执行，并且即便 `--confirm-generate-patch` 也只是写出 patch 文件供人工审阅，不会在脚本内执行 `patch -p0`、`cp`、`mv` 或 `rm` 任何本地技能文件。

已实现脚本：

```bash
# 观察全部条目，不改本地技能/插件
tools/agent-toolchain/check-agent-toolchain-updates.sh
tools/agent-toolchain/check-agent-toolchain-updates.sh --no-network --report /tmp/agent-toolchain-update-report.no-network.md

# 对单个可 Git 比对的条目生成更新提案和 diff
tools/agent-toolchain/propose-agent-toolchain-update.sh skill/caveman-commit
tools/agent-toolchain/propose-agent-toolchain-update.sh skill/diagnose

# 接纳门禁：默认 dry-run，只读分析提案影响
tools/agent-toolchain/accept-agent-toolchain-update.sh tools/agent-toolchain/proposals/skill__diagnose-20260523-152716.md --dry-run

# 人工复核 dry-run 后，生成只读 patch 文件，仍不应用
tools/agent-toolchain/accept-agent-toolchain-update.sh tools/agent-toolchain/proposals/skill__diagnose-20260523-152716.md --confirm-generate-patch
```

`accept-agent-toolchain-update.sh` 的硬性约束：

- 对 `plugin/*` 与 `skill/plugin-cache/*` canonical id 直接 `abort`，要求通过父插件机制处理。
- 启动时校验本地目录指纹是否与提案生成时一致，不一致立刻拒绝，避免接纳一个已经过时的 diff。
- patch 文件方向是 `local -> upstream`，即“如果你接纳上游，本地会变成什么”，便于人工 `patch -p0 --dry-run` 预览。
- 全程不写 `$HOME/.agents/skills`、`$HOME/.codex/skills`、`$HOME/.codex/plugins/cache`。路径由脚本运行时按 `$HOME` 解析，不依赖固定用户名。

`diff-propose` 和 `accept` 当前只处理直接 Git diff 能解释的技能，例如 `git_compare_from_skill_lock`、`git_compare_local_remote`、`git_compare_probable_source`、`codex_vendor_git_check`。插件、Lark 技能和本地 wrapper 仍保持人工 review，因为它们不能安全地把上游目录直接映射成本地 skill。

## 8. 关联文件

| 文件 | 用途 |
|---|---|
| `tools/agent-toolchain/data/agent-toolchain-inventory.yaml` | 当前技能/插件清单 |
| `tools/agent-toolchain/data/agent-toolchain-canonical-map.yaml` | 本地 canonical 映射 |
| `tools/agent-toolchain/data/agent-toolchain-source-resolution.yaml` | 来源反查和可信度 |
| `tools/agent-toolchain/data/agent-toolchain-source-resolution-summary.md` | 来源反查摘要 |
| `tools/agent-toolchain/data/agent-toolchain-update-plan.yaml` | 更新检查策略 |
| `tools/agent-toolchain/check-agent-toolchain-updates.sh` | 只读更新检查器 |
| `tools/agent-toolchain/propose-agent-toolchain-update.sh` | 单条目只读 diff 提案生成器 |
| `tools/agent-toolchain/accept-agent-toolchain-update.sh` | 接纳门禁：dry-run 报告 + 可选只读 patch 生成 |
| `tools/agent-toolchain/proposals/` | 生成的更新提案和 diff 文件 |
| `tools/agent-toolchain/acceptance-patches/` | accept 阶段生成的只读 patch 文件，需人工应用 |
| `tools/agent-toolchain/agent-toolchain-acceptance-log.yaml` | 人工接纳/拒绝/部分接纳记录，含来源、理由、验证和新基线哈希 |
| `tools/agent-toolchain/reports/agent-toolchain-update-report.md` | 最近一次检查报告 |
| `tools/agent-toolchain/fix-crlf.py` | 把 CRLF 文件就地改回 LF 的小工具，配合 Cursor Write 工具使用 |
| `docs/agent-workflow/agent-toolchain/` | 开发套件级说明、迁移 runbook 和 inventory schema |

## 9. 一句话总结

这套系统不是“自动把 GitHub 上的新东西覆盖到本地”，而是“用本地清单守住可用性，用只读检查感知上游变化，用人工 diff 决定是否接纳更新”。这样既能持续吸收外部技能和插件演进，又不会牺牲已经调顺的本地 agent 协作机制。

## 10. 工具与编辑约定

### 10.1 行尾约定

本工作区下所有 `.sh` 脚本必须使用 LF 行尾。原因：bash 不接受 `\r` 出现在 shebang 或 `set -euo pipefail` 行末，会直接报 `set: pipefail\r: invalid option name` 之类语法错。

注意：Cursor 的 Write 工具在这个工作区下偶发会把新写入的文件保存为 CRLF。每次用 Write 新建或大幅改写 `.sh` 后必须确认行尾：

```bash
head -3 <path> | od -c | head -3   # 看是 \n 还是 \r \n
bash -n <path>                     # 语法检查
```

如果含 CR，用 `tools/fix-crlf.py` 处理：

```bash
python3 "tools/agent-toolchain/fix-crlf.py" "tools/agent-toolchain/accept-agent-toolchain-update.sh"
# 输出 fixed: ... 或 clean: ...
```

`tools/agent-toolchain/fix-crlf.py` 只处理命令行参数显式指定的文件，不会递归、不会触碰任何技能/插件目录。脚本本身用 Python 编写，能容忍自己源码被 CRLF 污染。

### 10.2 接受 diff 的格式约束

`accept-agent-toolchain-update.sh` 只接受 `propose-agent-toolchain-update.sh` 产出的 `diff -ruN` 风格 unified diff：

- 缺失侧用 epoch 时间戳（`1970-01-01 ...`）标记；脚本据此把每个 file block 归类为 modified / upstream-only / local-only。
- 反向 patch 会用 `/dev/null` 标记被创建或被删除的一侧，便于 `patch -p0 --dry-run` 直接预览。
- 若 diff 中出现 `diff --git` 头（带 `a/...` `b/...`），脚本会 abort 而不是产出半正确的反向 patch；这种情况需要先把 diff 重新规范化或由人工 review。

### 10.3 不变量

任何 propose / accept / fix-crlf 调用都必须满足：

- 不写 `$HOME/.agents/skills`
- 不写 `$HOME/.codex/skills`
- 不写 `$HOME/.codex/plugins/cache`
- 不修改 `tools/agent-toolchain/data/agent-toolchain-inventory.yaml`、`tools/agent-toolchain/data/agent-toolchain-canonical-map.yaml`、`tools/agent-toolchain/data/agent-toolchain-source-resolution.yaml`、`tools/agent-toolchain/data/agent-toolchain-update-plan.yaml`

任何接纳决定的真正落地（`patch -p0`、`git mv`、`rm`）都必须由人工执行，脚本只生成可审阅的中间产物。
