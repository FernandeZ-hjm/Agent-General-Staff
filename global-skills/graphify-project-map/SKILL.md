---
name: graphify-project-map
description: |
  手动触发技能。当用户输入 `/graphify-project-map` 时使用。
  为新项目接入、老项目迁移、大型项目首次理解、架构优化提供项目知识图谱生成和画像分析。
  不要自动触发，仅在用户明确调用时使用。
---

# Graphify 项目画像

利用 Graphify CLI 生成项目知识图谱，并基于图谱输出结构化的项目画像和优化建议。

## 触发方式

仅通过 `/graphify-project-map` 手动触发。不自动匹配任何对话内容。

## 需要安装

本技能依赖 `graphify` CLI 工具。PyPI 包名是 `graphifyy`，CLI 命令是 `graphify`。

推荐安装方式：

```bash
uv tool install graphifyy
```

备选：

```bash
pipx install graphifyy
```

## 第一步：环境检查

1. 检查 `graphify` CLI 是否可用：

```bash
which graphify || echo "NOT_INSTALLED"
```

2. 如果输出 `NOT_INSTALLED`，告知用户并给出安装命令，等待用户安装确认。
3. 安装完成后，验证版本：

```bash
graphify --version
```

## 第二步：敏感文件排除

1. 检查项目根目录是否存在 `.graphifyignore`。
2. 如果不存在，使用 `assets/.graphifyignore.template` 创建一份。
3. 创建前向用户确认：
   - 项目中是否有敏感文档（合同、客户资料、财务报表等）
   - 是否有图片、PDF、Office 文件不希望被扫描
   - 是否有其他需要排除的目录或文件类型
4. 根据用户反馈，在模板基础上增删排除项。

模板已默认排除：

```
.git/
node_modules/
dist/
build/
coverage/
.next/
.nuxt/
.turbo/
.cache/
.env
.env.*
secrets/
credentials/
*.pem
*.key
*.p12
*.crt
*.sqlite
*.db
*.log
```

## 第三步：选择扫描范围

根据项目规模给出建议：

- **小型项目（<50 文件）**：建议扫描项目根目录
- **中型项目（50-500 文件）**：建议扫描核心目录
- **大型项目（>500 文件）**：强烈建议只扫描核心目录，不要全量扫描

默认建议扫描的核心目录（按存在性筛选）：

```
src/ app/ pages/ components/ lib/ server/ api/ docs/ db/ prisma/ scripts/
```

1. 列出项目根目录结构，让用户确认哪些目录需要扫描。
2. 对大型项目，主动提醒全量扫描可能耗时较长且消耗较多 API 配额。
3. 用户确认后，将扫描范围写入 `.graphifyignore` 的取反逻辑，或通过 CLI 参数指定。

## 第四步：运行 Graphify

```bash
graphify update <target-directory>
graphify cluster-only <target-directory>
```

- `<target-directory>` 是用户确认的扫描目录
- 输出目录固定为 `graphify-out/`
- 本机 `graphify` wrapper 已配置：`graphify update` 会自动改写为 LLM 语义抽取，使用 Claude Code 的 `claude-cli` 后端，并限制并发与单块 token 预算。
- 不要把 API key/token 写入命令、技能、项目文档或配置文件。
- 如果项目已有 `graphify-out/`，先确认它是否是旧图谱；需要重建时可以直接运行上面的两条命令。

如果不在本机 wrapper 环境中，需要显式运行：

```bash
graphify extract <target-directory> --backend claude-cli --token-budget 3000 --max-concurrency 1
graphify cluster-only <target-directory>
```

运行期间保持与用户的沟通，告知进度。

## 第五步：读取报告

扫描完成后，读取 `graphify-out/GRAPH_REPORT.md`。

如果文件过大，分段读取，先读前面部分了解结构，再根据需要深入。

## 第六步：输出项目画像

基于 `GRAPH_REPORT.md` 的内容，输出以下结构化的项目画像：

### 6.1 技术栈

- 语言、框架、运行时
- 构建工具、包管理器
- 数据库、存储方案
- 关键依赖库

### 6.2 目录结构

- 顶层目录职责说明
- 每个目录的用途和内容概况

### 6.3 核心模块

- 列出主要模块及其职责
- 模块间的依赖关系

### 6.4 数据流 / 调用链

- 请求入口 → 中间层 → 数据层的典型路径
- 关键 API 端点或路由
- 数据模型概览

### 6.5 外部依赖

- 第三方服务（支付、邮件、存储等）
- 外部 API 调用
- 数据库连接

### 6.6 潜在架构风险

- 循环依赖
- 过度耦合的模块
- 单点故障
- 缺少抽象层
- 安全风险（未加密数据传输、硬编码凭证等）

### 6.7 可优化点

- 性能瓶颈（N+1 查询、缺少缓存、重复计算）
- 代码冗余
- 可拆分的单体模块
- 缺少测试覆盖的关键路径

### 6.8 后续开发建议

- 短期可做的低成本改进
- 中期架构优化方向
- 技术债务偿还优先级

### 6.9 适合继续追问的问题

根据项目特点，列出 5-10 个有助于深入理解项目的问题，例如：

- "这个模块的异常处理策略是什么？"
- "为什么选择了这个数据库方案而不是其他方案？"
- "XX 和 YY 之间的耦合是否可以通过接口抽象解耦？"

## 团队协作建议

在输出完项目画像后，向用户提供以下建议：

### 推荐的 .gitignore 配置

```gitignore
# Graphify 输出 - 团队共享
graphify-out/manifest.json
graphify-out/cost.json

# Graphify 输出 - 可选提交
# graphify-out/GRAPH_REPORT.md
# graphify-out/graph.json
```

### 建议

- **可以提交** `GRAPH_REPORT.md`：便于团队成员快速了解项目结构，适合放入新人 onboarding 文档
- **可以提交** `graph.json`：如果团队需要在 CI 或工具链中使用结构化图谱数据
- **建议忽略** `manifest.json`：扫描元数据，每次扫描都会变化，无共享价值
- **建议忽略** `cost.json`：API 消耗统计，仅个人参考
- **建议忽略** `graphify-out/` 整体或按上述细则配置 `.gitignore`
