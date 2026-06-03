# 东门老胡多 Agent 工程治理套件

[English version](README.en.md)

我做这个项目，不是因为我觉得自己发现了什么惊天秘密。

恰恰相反，是因为我在用 Claude Code、Codex、Cursor、DeepSeek 做项目的时候，越来越明显地感觉到一件事。

Vibe-coding 真正难的地方，不是让 Agent 写代码。

而是让它们在一个项目里稳定地协作。

单个 Agent 很强，但多个 Agent 接力时，问题就来了。提示词会漂移，任务边界会变形，验证会被跳过，第三方 skill 越装越多，最后你会发现，自己不是在开发项目，而是在维护一堆工具的脾气。

这就很离谱。

所以我开始做这套治理套件。

它不是一个让人瞬间变强的神器，也不是某种玄学 prompt 包。它更像是一套多 Agent 开发环境里的工程秩序，把任务卡、运行时适配、技能触发、验证门、回滚、边界扫描这些东西放到一个可管理的框架里。

我的目标很简单。

让个人开发者和小团队，用更低成本，把 Vibe-coding 做得更稳一点。

版本更新说明见 [更新日志](CHANGELOG.md)。

## 这个项目解决什么问题

如果你只是偶尔让 ChatGPT 写一段代码，可能用不上它。

但如果你已经开始让 Claude Code、Codex、Cursor 参与真实项目，它就会有价值。

真实项目里，最怕的不是 Agent 不够聪明。

最怕的是它太自信。

它可能改了不该改的文件，跳过验证，误解任务边界，或者把一个本来很小的需求写成一场大装修。

这套治理套件就是为了把这些风险压住。

它会要求任务开始前讲清楚目标，执行时遵守边界，完成后必须验证。它也会把不同工具之间的协作语言统一成任务卡，让人、Claude Code、Codex、Cursor 至少在同一张纸上说话。

## 两个公开版本

这个仓库提供两个公开版本。

### DIY/Core

DIY/Core 适合已经有自己工具栈的人。

它只提供治理框架，不默认安装完整 skill 栈。你会得到任务卡协议、运行时适配规则、验证门、项目模板、环境检查、差异检查、回滚和本地记忆入口。

它不会替你决定用哪些第三方 skill。

如果你已经知道自己要什么，用这个版本就够了。

### Full Installer

Full Installer 适合想快速复刻完整工作流的人。

它包含 DIY/Core 的全部内容，也包含套件管理的全局 rules、skills、Claude Code 和 Codex 的 hook 规范、项目工作流安装器、验证流程和回滚凭据。

如果你是第一次搭 Vibe-coding 环境，建议先从 Full Installer 开始。

先跑起来，再慢慢理解每一层在干什么。

## 保姆版安装教程

先说最重要的一句。

第一次不要直接写入。

先 dry-run。

dry-run 的意思是，只预览它准备做什么，不真正改你的文件。你先看清楚，再决定要不要 apply。

### 第一步，准备目标项目

假设你的项目目录是这个。

```bash
/Users/you/projects/my-project
```

如果还没有项目，可以先建一个空目录。

```bash
mkdir -p /Users/you/projects/my-project
```

如果这是一个 git 仓库，先进去看一下状态。

```bash
cd /Users/you/projects/my-project
git status
```

如果里面已经有很多未提交改动，建议先处理一下。

不是说不能装，而是安装器写入文件以后，你会很难分清哪些是原来的改动，哪些是这次安装带来的改动。

### 第二步，进入治理套件目录

如果你已经下载了这个仓库，进入它。

```bash
cd /path/to/Dongmenlaohu-multi-agent-engineering-kit
```

如果你还没下载，可以从 GitHub 拉下来。

```bash
git clone https://github.com/FernandeZ-hjm/Dongmenlaohu-multi-agent-engineering-kit.git
cd Dongmenlaohu-multi-agent-engineering-kit
```

### 第三步，先检查治理套件本身

```bash
bash scripts/verify.sh
bash scripts/security-doctor.sh
```

这一步是看治理套件自己是否完整，公开边界是否干净。

如果这里都不过，就先别装到你的项目里。

### 第四步，选择安装版本

如果你想安装完整工作流，用 Full Installer。

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

如果你只想接入治理框架，用 DIY/Core。

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --dry-run
```

这里有三个参数要注意。

`--target-project` 是目标项目路径。

`--project-name` 是给人看的项目名，可以有空格。

`--project-slug` 是稳定项目标识，建议用小写英文、数字和短横线。

### 第五步，认真看 dry-run 输出

dry-run 会告诉你它准备写入哪些东西。

常见的是这些。

```text
项目 Agent 入口文件，AGENTS.md
Claude Code 项目协议文件，CLAUDE.md
Agent 工作流文档目录，docs/agent-workflow/
项目画像配置，config/agent-project-profile.yaml
```

Full Installer 还会处理本机运行时里的 rules、skills 和 hooks。

如果你看到它准备覆盖你很在意的文件，先停下来。

这一步不要省。

### 第六步，确认后再写入

Full Installer 写入。

```bash
bash scripts/kit-install.sh \
  --profile full \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

DIY/Core 写入。

```bash
bash scripts/kit-install.sh \
  --profile diy \
  --target-project /Users/you/projects/my-project \
  --project-name "My Project" \
  --project-slug my-project \
  --apply
```

写入前，安装器会生成备份。

备份默认放在这里。

```text
$HOME/.agents/backups/
```

安装完成后，它还会输出一份 receipt，里面会记录这次写了什么，以及以后怎么回滚。

### 第七步，安装后检查

Full Installer 检查。

```bash
bash scripts/kit-doctor.sh doctor \
  --profile full \
  --target-project /Users/you/projects/my-project
```

DIY/Core 检查。

```bash
bash scripts/kit-doctor.sh doctor \
  --profile diy \
  --target-project /Users/you/projects/my-project
```

这个命令会看目标项目里该有的文件是否存在，也会检查本机运行时状态。

### 第八步，只安装运行时

有时候你已经接入过项目，只想补本机运行时。

Full Installer 可以这样预览。

```bash
bash scripts/kit-install.sh \
  --profile full \
  --scope runtime \
  --dry-run
```

确认后再写入。

```bash
bash scripts/kit-install.sh \
  --profile full \
  --scope runtime \
  --apply
```

DIY/Core 不安装内置 full skill runtime。

这是刻意设计的。

DIY/Core 只负责治理框架，不替你接管工具栈。

### 第九步，检查本地差异

如果你想知道本机运行时和治理套件里的版本有没有漂移，可以跑这个。

```bash
bash scripts/diff-local.sh
```

### 第十步，以后怎么更新

只检查有没有更新。

```bash
bash scripts/kit-doctor.sh update --check
```

查看更新差异。

```bash
bash scripts/kit-doctor.sh update --diff
```

确认后再应用。

```bash
bash scripts/kit-doctor.sh update --apply
```

`update --apply` 只允许 fast-forward 更新，并且会在更新后运行验证和安全检查。

这个设计有点啰嗦。

但我觉得 Vibe-coding 环境宁可啰嗦一点，也不要偷偷改坏你的机器。

## 重要文件说明

真实文件名仍然保留英文路径。

这是为了兼容命令行、git、Claude Code、Codex 和各种运行时。

但你理解的时候，可以把它们当成中文职责。

```text
安装入口脚本，scripts/kit-install.sh
环境检查脚本，scripts/kit-doctor.sh
完整性验证脚本，scripts/verify.sh
安全边界扫描脚本，scripts/security-doctor.sh
任务卡校验脚本，scripts/validate-task-card.sh
Agent 工作流协议，protocol/
项目接入模板，project-integration/
全局规则目录，global-rules/
全局技能目录，global-skills/
任务卡模板目录，templates/
第三方 skill 说明，docs/third-party-skills.md
MCP server 说明，docs/mcp-servers.md
```

## 安全边界

这个公开仓库不能包含私有项目名、私有发布拓扑、个人 persona profile、私有同步 registry、真实 secret、token 或 API key。

它也不会默认做这些事。

不会强推 git。

不会执行 `curl | bash`。

不会静默安装第三方依赖。

不会删除你的 global skills 目录。

不会默认安装 optional skill packs。

CodeGraph MCP 只记录说明，不会被 `bootstrap.sh` 偷偷安装。

我一直觉得，Agent 工程最难的不是让它开始行动。

而是让它知道什么时候应该停。

这套治理套件也是这个思路。

它不是为了让 Vibe-coding 看起来更酷。

它是为了让 Vibe-coding 在真实项目里，少一点玄学，多一点秩序。

以上。
