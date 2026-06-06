# Context Capsule: <project-slug>

Manual-maintained stable project memory.

## 项目设计目的

(TODO: describe this project's purpose in one or two concrete paragraphs.)

Rules:
- Runner, hook, capture, or automated summarization must not overwrite this section.
- Automated summaries must not rewrite this section.
- Modify this section only when the project owner explicitly asks.
- Agents must read this file before task execution.
- If a task conflicts with this section, stop and report before changing files.

## Stable Facts

- Project path: `<project-path>`
- Memory dir: `$HOME/.agents/memory/projects/<project-slug>`

## 项目长期边界

- (TODO: define non-negotiable project boundaries.)

## 核心业务定位

- (TODO: define what this project is for and what it is not for.)

## 原则性决策

- (TODO: record durable decisions that should survive context compaction.)

## 自动记忆入口

- Progress log: `$HOME/.agents/memory/projects/<project-slug>/progress-log.md`
- Archive index: `$HOME/.agents/memory/projects/<project-slug>/archive-index.md`
- Sessions: `$HOME/.agents/memory/projects/<project-slug>/sessions`
- Task archive: `$HOME/.agents/memory/projects/<project-slug>/task-archive`
