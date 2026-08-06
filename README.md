# Agent General Staff (AGS)

[![CI](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml/badge.svg)](https://github.com/FernandeZ-hjm/Agent-General-Staff/actions/workflows/ci.yml)
[![License: GPL-3.0-only](https://img.shields.io/badge/License-GPL--3.0--only-blue.svg)](LICENSE)

[中文](README.md) | [English](README_EN.md)

AGS 是多 Agent 开发的治理控制面：它负责准入、精确能力路由、授权、维护事务、
验证、回执和恢复；它不是 Agent 调度器、任务队列或自然语言分类器。

当前源码发布候选是 **v0.5.0**；latest published release 是 **v0.5.0**。

## 安装方式

CLI 与 MCP 是两个可独立选择的 npm 入口，底层使用同一签名 Rust 内核、内容寻址缓存
和机器状态目录。同时安装不会重复下载内核。

```bash
# 终端用户
npx -y @agent-governance-suite/cli --help

# MCP Host
npx -y @agent-governance-suite/mcp
```

也可从源码安装：

```bash
cargo install --path crates/ags-cli --locked --force
ags setup --yes
```

## 维护闭环

所有 AGS、Skill、setup 和宿主激活变更都经过同一个维护事务：

```text
Intent -> hash-bound Plan -> 用户确认 -> Apply
       -> Host 激活 -> Verify -> Receipt
                       \-> 失败时 Recover
```

- Plan 固定来源、版本、内容哈希、风险、写入集合和回滚点。
- Apply 只接受未过期的精确 `plan_hash`。
- 文件复制不算完成；实际 Host 投影、snapshot 和 RouteResolution 必须通过。
- 所有更新默认由用户确认，不静默应用。

## 第三方 Skill

推荐目录只用于发现和审查提示，不是安装白名单。用户可以安装推荐 ID，也可以指定任意
GitHub 仓库、tree 子目录、分支、标签或提交；真正应用前一律解析到不可变 commit。

```bash
ags skill recommend
ags skill inspect <catalog-id|github-url|local-path>
ags skill install <catalog-id|github-url>
ags skill adopt <local-path>
ags skill check [skill-id]
ags skill update <skill-id>
ags skill rollback <skill-id>
ags skill status [skill-id]
ags skill verify <skill-id>
```

机器上的 `InstalledSkillRecord` 是安装唯一真相。目录推荐、安装记录、Host 激活和更新
策略是四层独立事实；旧 JSON 状态不兼容读取，只做一次性迁移。默认更新策略为
`notify`，也可设为 `manual` 或 `pinned`。AGS 不执行第三方仓库自带脚本。

路径穿越、symlink 逃逸、特殊文件、越界写入、ID 覆盖和哈希漂移会硬阻断；未知许可、
脚本、二进制、外部依赖和高权限请求会显示风险并要求逐项确认。

## setup 与五宿主投影

`ags setup` 和维护更新会把 required suite Skill 投影到 Codex、Claude Code、OMP、
Cursor 与 CodeBuddy-Code，迁移已声明的上游改名并清理 AGS 拥有的退役 symlink。
候选状态先整体准备，再原子切换；失败恢复旧 body、索引、snapshot 和 Host 指针。

本机私有发行工具可额外要求所有目标来自 stable authority；公开内核只提供通用的
authority-root policy seam，不硬编码维护者机器路径。

## AGS 更新提醒

CLI 启动、MCP preflight 或 Doctor 默认每七个自然日惰性检查一次签名 release index。
离线或远端不可达不会阻断现有能力。用户可以稍后提醒、忽略某版本或关闭检查。

```bash
ags update check
ags update plan
ags update status --plan-hash <HASH>
ags update apply --plan-hash <HASH>
ags update verify --plan-hash <HASH>
ags update recover
```

## 治理请求链

自然语言始终由 Host 解释。AGS 只消费 typed contract：

```text
preflight
  -> ags://capabilities/current-host
  -> typed HostRouteProposal
  -> read-only ags_route_request
  -> Host 原生动作或一次性 ags_apply_action
  -> evidence / receipt closure
```

`ags_route_request` 不接收 raw request，不做关键词或相似度回退；`ags_apply_action` 只
消费当前连接保存的一次性 action。任务卡必须同时满足明确交接指令和 confirmed
handoff contract；Heavy 只增加独立审查门禁，不扩大执行权限。

## 架构与验证

公开 workspace 只有十二个权威 Cargo module。CLI 与 MCP 只是 adapter，领域规则位于
capability、decision、session、lifecycle、evidence 和 verification module 中，不保留
第二套旧 update/bootstrap/registry 实现。详见 [WORKSPACE.md](WORKSPACE.md) 与
[docs/architecture.md](docs/architecture.md)。

发布链只允许两次完整门禁：A 精确候选一次、B 精确公开 commit 一次。Promotion、Tag、
Release 和 npm 消费内容寻址 `VerificationBundle`，不重复 workspace 全量测试。

## 公开边界

公开版包含完整公共 Rust 内核、双 npm 入口、协议、typed manifests、命令 Skill 和发布
工作流；不包含私人 Skill body、真实 memory/receipt/archive、凭据、本机路径、Host
配置或运行时状态。公开能力投影由 typed canonical manifest 生成，推荐项永远不会仅因
出现在目录中就成为 installed 或 routable。

许可证：**GPL-3.0-only**。
