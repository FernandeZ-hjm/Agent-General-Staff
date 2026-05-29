# Dongmenlaohu Multi-Agent Engineering Kit

[English version](README.en.md)

一个面向多 Agent 工程协作的开源可见开发套件，用来搭建可治理、可复现、可验证、可回滚的 Vibe-coding 环境，用顶级模型+顶级 agent 几十分之一的开发成本，达到七八成的开发效果。

## 个人介绍与项目背景

大家好，我是东门老胡。

我以前做审计，后来成了兴趣使然的科技媒体人，现在是一家硬科技公司的公关负责人。到了这个项目开源的时候，我又多了一个新身份：兴趣使然的 Vibe-coding 架构师（新手上路版）。

我走上这条路，不是突然拍脑袋。

2025 年下半年，我还在做科技媒体主笔。几位 AI 行业的朋友反复推荐我试用 Gemini 3.0 Pro，理由很直接：它的文本输出能力，当时明显领先其他顶级模型。不到一个季度，我就成了 Gemini 的深度用户，并开始用 Gem，也就是 Gemini 自带的 skill 工具，重构自己的写作和研究流程。

到 2026 年初，这套方法论甚至帮我赚到了第一桶金。

进入企业之后，问题变得更具体，也更现实。

我面对的是一个方兴未艾的硬科技业务。人手少，事情多，市场、公关、对外宣发、对内策略支持，全都需要更高频、更稳定地交付。如果不用 AI 放大效率，我很难靠一个人撑住这些工作。

所以在一个月前，我实装了 Claude Code + DeepSeek。半个月前，我又实装了 Codex 和 Cursor，用它们开发自己的项目。

新的问题很快出现了：

几个 Agent 可以各自很强，但它们怎么在同一个工程里稳定协作？

人类的提示词，如何能在多个 Agent 工具里不漂移，不变形？如何大量击中缓存，让 token 消耗可控？

第三方的 skills 当然好用，问题是经过本地化封装之后，如何溯源，如何管理？如何正确更新到它应该更新的位置上？

我不是大厂出身，也不背靠 AI 大厂的资源池。我的预算有限，不可能长期 Opus 或 GPT 无限续杯。使用 Deepseek 这类国产模型做主力开发几乎是必然的。

所以我如果想用几个 Agent 合作，用几十分之一的成本达到顶流模型，必须要有一套极其严苛的工程化开发环境，让不同天赋的模型，最大化发挥自己的优势。

所以我必须选择一条更讲效价比的路线：让 DeepSeek 在明确的编码规范、行为边界和工作流程里，把能力发挥到极致。

只有这样，我的项目开发才可能真正工程化，最后做出来的东西才不只是一个玩具。

我也经历过很多 Vibe-coding 新手都会经历的阶段：无脑安装各种 skills、hooks 和 MCP。

但很快我发现，工具越多，问题越多。有的版本混乱，有的互相冲突，有的触发时机不稳定，有的更新路径不清楚。它们本来是为了提升效率，最后却可能把注意力从开发本身拖回工具治理。

所以我开始做另一件事：把这些能力放进一套可管理的秩序里。该自动触发的时候自动触发，该人工确认的时候人工确认，该更新的时候能更新，该回滚的时候能回滚。

这个项目借鉴了几个方向的经验。

一部分来自 GitHub 上的热门开源项目，包括各种 Agent skills。它们补强了 Claude Code 的框架能力，也让 DeepSeek 能在更清楚的边界里工作。

另一部分来自 Claude Code 官方最新工作流实践。它真正有价值的地方，不是某个提示词写得漂亮，而是强边界、可追溯、交付范围明确。任务怎么开始，谁负责执行，谁负责审查，什么叫完成，失败后怎么回退，都要有规则。

一个月之后，这个工程化开发套件，终于到了可以开源见人的阶段。

我必须承认，中间很多能力并不是我原创的。但我做的事，是把这些分散的技能、规则、任务卡、hooks 和验证门，组装成一套足以支持个人开发者和中小团队使用的工程化套件。

对 Vibe-coding 新手来说，它不是让你少学 coding 工程。恰恰相反，它是把那些最容易被忽略、但迟早会补课的工程秩序，提前放到你的 AI coding 环境里。

装上它之后，面对中小型项目，你会更容易得到两件东西：更高的开发效率，以及更稳定的交付边界。

## 项目版本

这个仓库有两个公开版本：

- **DIY版**：面向成熟的 AI 行业朋友，只保留了我的治理框架、任务卡协议、运行时适配规则、验证门、项目模板和安装验证器。
- **满血版**：面向新手朋友，只需一道命令，就能复刻我完整的开发工作流。

## 快速开始

先预览，不要急着写入。

Full Installer 会把完整开发套件安装到目标项目，并接入本地 Agent runtime。第一次运行建议只看 dry-run 输出：

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

如果你只想接入治理框架和项目工作流，不想默认安装完整 skill 栈，使用 DIY/Core：

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

确认 dry-run 输出符合预期后，再执行 apply：

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /path/to/project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

检查本地冲突、已安装 runtime 漂移、目标项目状态，以及公开版本更新：

```bash
bash scripts/kit-doctor.sh doctor --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
```

第三方 skill 来源在这里看：

```text
docs/third-party-skills.md
```

支持的 MCP server 在这里看：

```text
docs/mcp-servers.md
```

## 版本说明

### DIY/Core

DIY/Core 适合已经有自己工具栈的人。

它不替你决定使用哪些第三方 skill，而是提供一套工程秩序：缓存稳定的任务卡、轻中重任务路由、审查与验证门、项目 profile、runtime adapter 约定、本地 memory capsule、dry-run、rollback、diff 和 doctor 工具。

Core 不假设任何第三方 skill 一定存在。规则层只引用 capability slots。某个能力没有安装时，系统应该降级，而不是崩掉。

### Full Installer

Full Installer 适合想快速复刻完整开发环境的人。

它包含 Core 的全部内容，也包含必要的 global skills、Claude Code 与 Codex 的 hook 标准化、项目工作流安装器、验证流程和 rollback receipt。

可选第三方 skill pack 不会默认打包安装。`skill-packs/optional/` 只是预留扩展点。当前第三方上游和候选 skill，按 GitHub 作者列在 `docs/third-party-skills.md`。

CodeGraph MCP 被记录为可安装的 MCP server，说明见 `docs/mcp-servers.md`。`bootstrap.sh` 不会静默安装它。

Full Installer 也不会绑定私有项目。目标项目信息来自 CLI 参数和 `config/agent-project-profile.yaml`。

## 仓库结构

```text
├── AGENT_SUITE_PROTOCOL.md
├── README.md
├── docs/
├── global-rules/
├── global-skills/
├── governance/
├── manifests/
├── project-integration/
├── protocol/
├── scripts/
├── skill-packs/
└── templates/
```

## 项目接入

`scripts/kit-install.sh` 是公开安装入口。

它负责调度两件事：项目层面的工作流写入交给 `scripts/install-suite-to-project.sh`；Full profile 的 runtime 写入交给 `scripts/bootstrap.sh`。它不会静默覆盖文件。

它会写入或更新：

- `AGENTS.md`
- `CLAUDE.md`
- `docs/agent-workflow/`
- `config/agent-project-profile.yaml`

执行 `--apply` 前，它会生成带时间戳的备份，并打印一张 receipt，里面包含 rollback 位置。

## 运行时安装

Full profile 的 runtime 安装，会把 global rules 和 skills 放进目标 home。默认模式仍然是 dry-run。

```bash
bash scripts/kit-install.sh --profile full --scope runtime --dry-run
bash scripts/kit-install.sh --profile full --scope runtime --apply
bash scripts/diff-local.sh
```

DIY/Core 会刻意跳过内置 global skill 的 runtime 安装。除非下游用户自己提供 capability implementations，否则它只提供治理框架。

## 环境检查

`scripts/kit-doctor.sh` 是公开环境检查器。

它会运行 suite doctor、security doctor、可选的目标项目冲突检查，以及 update gate。

```bash
bash scripts/kit-doctor.sh doctor --profile full --target-project /path/to/project
bash scripts/kit-doctor.sh update --check
bash scripts/kit-doctor.sh update --diff
bash scripts/kit-doctor.sh update --apply
```

`update --check` 只读。`update --diff` 会抓取到 `FETCH_HEAD`，然后打印 diff 摘要。`update --apply` 要求 suite worktree 干净，只允许 fast-forward，随后运行验证和 security doctor。

安装器会把被替换文件备份到：

```text
$HOME/.agents/backups/
```

## 安全模型

自动化不能在没有明确授权的情况下，运行破坏性命令或外部依赖安装命令。

manifest 默认禁止这些行为：

- `git push --force`
- `curl | bash`
- `npx skills add/remove/update`
- 未经审查的依赖安装
- 删除用户的 global skill 目录

## 公开边界

这个公开仓库不能包含私有项目名、私有发布拓扑、个人 persona profile、私有同步 registry、真实 secret、token 或 API key。

发布前运行边界扫描：

```bash
bash scripts/security-doctor.sh
```
