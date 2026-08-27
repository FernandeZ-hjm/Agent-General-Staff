# Agent Governance Suite

[English](README_EN.md)

Agent Governance Suite（AGS）v3 是一个**薄壳式 Agent 治理套件**：把治理从
"流程总线"改为"执行点上的薄壳"——平时隐形，危险时现身。

## 一句话理解

```text
ags run --task card.md          # 校验 + 矩阵判权 + 审查级推导
   （宿主执行，ags-policy hooks 静默判权每个工具调用）
ags run --task card.md --verify # 结构化验证命令（无 shell）
ags run --task card.md --close --report report.json --effective heavy
                                # 证据链闭包 + 记忆指针
```

危险操作走封条：`ags update` 等 sealed 命令先出 `action_ref`，
`ags apply <ref>` 单次消费，防重放、防篡改、跨绑定 fail-closed。

## 三个文件认识 AGS

- **`ags.toml`**：每 workspace 一份的单一策略文件。权限矩阵
  （`surface:action` → allow/ask/deny）、写边界、封条清单、验证命令、
  审查升级表、宿主注册、能力源。`ags init` 生成脚手架。
- **`.ags/evidence/events.jsonl`**：append-only 证据日志。每条事件内容
  寻址 + 链式（prev_sha256），断链可检测；按日 + 10MB 滚动归档。
- **`.ags/capabilities.lock`**：能力 hash 钉选。精确路由（id+hash），
  无 staleness 状态机，只有 `ags update` / install / remove 刷新。
- **本地入口投影**：`AGENTS.md` 等工作树文件包含 AGS managed block；
  repository-local clean/smudge filter 保证该区块不进入 Git。
- **可核验构建**：所有用户表面报告 `v0.4.21`，Doctor 单列
  `build=<commit>[.dirty]`，区分同版本的具体构建。

## 命令面（lark-cli 三层风格，每条命令带风险级）

| 层 | 命令 |
|---|---|
| 快捷层（+shortcut） | `ags run`（一条命令的任务流）、`ags init`、`ags doctor` |
| 类型化命令 | `ags mcp`、`ags check`、`ags test`、`ags log`、`ags status`、`ags govern skill install/remove`、`ags update`、`ags schema` |
| 封条逃生舱 | `ags apply <ACTION_REF>`（唯一 mutation 面） |

风险标注：`read`（零写入）/ `write`（封条计划 + apply）/
`high-risk-write`（封条 + 边界）。

## 宿主支持

| 宿主 | 判权 |
|---|---|
| Claude Code / Codex / Cursor / CodeBuddy | 全量 hooks（PreToolUse / PermissionRequest / PostToolUse → `ags-policy`） |
| OMP / DSH | MCP `ags_decide` / `ags_apply` 降级模式 |

hook 失效 fail-open 回落到宿主默认（D5），`ags doctor` 监控健康度。

## 五个 crate

`ags-kernel`（唯一深模块）· `ags-task-contract`（任务卡 + `ags run`）·
`ags-cli` · `ags-mcp` · `ags-release`（公开投影边界）。用户侧 CLI 与 MCP 统一从 `ags` 进入；
适配器是内核的薄投影，不持有平行领域逻辑。

## 快速开始

```bash
cargo build --release
ags setup --source-root .                   # 安装公开 v3 Skills + machine lock
ags init --workspace . --slug my-project     # 密封计划
ags apply <ACTION_REF> --workspace .         # 单次消费
ags govern host-register --id my-host --surface cli --workspace .
ags apply <ACTION_REF> --workspace .         # 消费宿主注册计划
ags doctor --workspace .                     # 体检
```

## 公开契约

- 架构与边界：`docs/architecture.md`
- 命令与 sealed operation：`ags --help`、`ags schema`
- 官方宿主 Skills：`ags-skills/`
- 发布变化：`RELEASE_NOTES.md`

私有协议、提案和 promotion 运行手册只存在于 A 权威工作区，不属于公开
投影，也不是公开安装的隐式依赖。

## 验证

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
RUSTFLAGS="-D warnings" cargo test --workspace
cargo build --release
ags check governance --workspace . --format json
git diff --check
```

## License

GPL-3.0-only。见 `LICENSE`。
