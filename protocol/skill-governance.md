# Skill Governance Protocol

> AGS 0.2.8 技能本体、可见性与确定性解析协议。

## Source of Truth

- `manifests/skills-registry.yaml`：技能身份、canonical source、routing metadata、`demand_routes`。
- `manifests/suite.yaml`：套件 required / optional / personal 集合。
- 受管外部源或宿主系统目录：canonical 技能本体；公开版不捆绑第三方技能正文。
- host 目录：thin index 或宿主系统技能；不是第二份 canonical 本体。
- `<runtime_home>/capability-snapshot/<host>.json`：按 active host 隔离的单机可用性快照；Codex、Claude Code 等宿主互不覆盖，不入库、不进入 public projection。

## 治理生命周期

### 1. Discover（发现）

发现 suite、用户、宿主系统、project-local 和外部技能，但被发现不等于可路由。

### 2. Scan（扫描）

检查名称、canonical body、hash、重复实现、健康状态和 host visibility。

### 3. Proposal（提案）

所有变更先输出 dry-run proposal；外部 installer、registrar 和登录动作只给建议。

### 4. Apply（写入）

只有显式 `--apply` 才能写 AGS-owned thin index、受管 metadata 或可逆 quarantine。禁止静默覆盖 canonical 本体。

### 5. Verify（验证）

验证 canonical 单例、宿主可见性、registry 路由字段和快照 freshness。

## 写入规则（硬门禁）

- 用户未确认时不写。
- 写前检查 symlink/path traversal/canonical 边界。
- 批量写入必须事务化或可回滚。
- 不读取或写入凭据；tracked manifest 不得声称 auth 已配置。
- 外部 MCP/CLI 注册保持 advice-only，除非另有明确授权的专用命令面。

## 与 AGS 任务卡系统的关系

Request Router 返回闭集 `SkillDemand`；Skill Resolver 再把 demand 映射为 `SkillSelection`。任务卡 compiler 不参与自然语言技能选择。

任务卡 `[skill: name]` 需要三闸：

1. registry 中 `route_state: routable`；
2. `invoke_hint` 合法且与父技能身份一致；
3. 当前 ActiveSkillTable 中可用。

任何一闸失败都停止。不得自动换成职责相似技能。

## 技能边界

技能是方法能力，不是权限层。它不能改变 task level、permission mode、review gate、verification gate、protected-path 或 release boundary。

Superpowers 等技能族只向宿主暴露父技能。内部 playbook 通过 `demand_routes.entrypoint` 选择，但不作为独立 host skill 或任务卡 tag。

## 第三方技能与 MCP 纳管控制台（`ags skill` console）

`ags skill` 是人类前台命令面；`ags capability` 是跨宿主可见性和快照的底层命令面。两者不承担自然语言需求路由。

### Canonical 本体 + per-host thin index（核心心智模型）

一个技能只有一个 canonical body。宿主入口可以是符号链接、插件入口或系统技能，但不得复制并分叉实现。

### 统一能力模型

inventory 可以包含 Skill、MCP、Suite Interface、CLI-backed capability。只有 `kind=Skill` 能进入 ActiveSkillTable。

### 路由纳管契约（Skill Resolver metadata）

技能 registry 的 routing block 至少声明：

```yaml
routing:
  route_state: routable | not-routable | retired
  invoke_hint: "[skill: canonical-name]"
```

顶层 `demand_routes` 必须覆盖 `SkillDemand::all()`，且 demand 唯一：

```yaml
demand_routes:
  - demand:
      category: engineering
      demand: debugging
    skill_id: diagnosing-bugs
```

### Host visibility 与 runtime health 分层

registry 说明“允许路由什么”；机器 inventory 说明“当前真的有什么”。两者不能互相代替。

## ActiveSkillTable 与快照

ActiveSkillTable 的集合定义：

```text
Skill
∩ canonical_present
∩ healthy
∩ routable
∩ active_host visible
```

快照字段至少包括：schema version、active host、registry hash、runtime hash、active-table hash、snapshot hash、active skills。

路由调用只读取并校验快照；不一致时返回 `skill_snapshot_stale`。刷新必须是显式命令或已授权写入动作的收尾步骤，不允许在路由途中静默刷新。

系统不再使用额外的机器加入模式。是否可路由由 registry + 真实机器状态的交集直接决定。

## 写入与验证的失败语义

- dry-run：报告 blocked/needs-action，但不因未写而失败。
- `--apply`：写入失败或被阻止时非零退出。
- advised-only 外部动作不得伪装成 applied。
- snapshot stale 是 Skill 目标的治理前置条件失败，不影响纯 DirectResponse 或纯 MachineCli。

## 当前实现状态

0.2.8 已实现：

- 唯一 Request Router；
- 闭集 SkillDemand；
- registry 固定 demand 映射；
- 单 host ActiveSkillTable 快照与 hash 校验；
- 写入动作自动刷新 Codex 快照；
- 无额外加入层、无相似技能 fallback、无自然语言二次分类。

## Public / stable 投影边界

registry 和协议可投影；机器 snapshot、runtime path、host inventory、credential/auth 状态不得进入公开投影。

## 协议引用

- `protocol/agent-task-protocol.md`
- `protocol/task-routing.md`
- `protocol/mcp-server.md`
- `protocol/task-card-template.md`
