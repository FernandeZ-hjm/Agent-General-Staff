# AGS Core Agent Rules

## 决策优先级：第一性原理

在诊断、调试、架构设计、修改方案、取舍判断和根因分析中，先从目标、事实、
约束、不变量、成本和可验证证据推导结论，再选择技能或 MCP。技能和 MCP 只提供
方法、证据与执行接口，不能代替宿主判断，也不能预设或强迫结论；若其建议与
第一性原理推导冲突，应明确说明冲突并采用推导后的结论。

该优先级只作用于授权范围内的决策形成，不得扩大用户授权、绕过系统或仓库安全
边界，也不得取消执行阶段的 fail-closed 门禁。

## AGS 入口

在 AGS 项目中，任何 AGS 工具、专用技能或工程流程之前，先调用 MCP
`ags_preflight`；不可用时使用：

```bash
ags session preflight --for <agent> --target <repo>
```

preflight 成功后读取 `ags://capabilities/current-host`。宿主保留完整对话并完成唯一一次
自然语言解释，提交 typed `HostRouteProposal` 给只读 `ags_route_request`，再消费
`RouteResolution`。不得把原始用户文本交给 AGS，也不得让 Compiler、Policy、Gate、
Runner 或 Skill Resolver 解释自然语言。`DirectResponse` 独占；否则至多一个精确
`SkillTarget` 与一个闭集 `MachineCliTarget`。只有 `ags_apply_action` 可消费连接内固定动作。

## 执行判断

- 纯摘要、翻译、格式转换或按既定结构重排：直接完成。
- 已确认方案加同会话明确修改指令：`direct-edit`，不重开方案流程。
- 边界未定或错误方向代价高：先形成方案；复杂故障优先 `diagnosing-bugs`。
- 已存在且以 `## 任务卡` 开头的输入：validate-first，不重新生成。
- 任务卡生成要求明确交接指令和已确认 handoff contract；不得从原始请求直接编译。
- 发布、外部写入、凭据、迁移、破坏性操作和受保护路径始终是独立停止条件。

任务卡、Plan mode 和 OMP 单卡出口的详细规则见同目录
`ags-task-handoff.md`，仅在生成、验证或执行任务卡时读取。

## 工程底线

- 只实现当前目标；不顺手重构或扩大范围。
- 先检查工作树，保留用户已有改动。
- 每项非简单修改都要有可验证的验收标准。
- 声称完成、修复、可提交或可发布前，执行
  `superpowers` 的 `verification-before-completion` playbook 或等价验证。
- 重要判断以代码、diff、测试、日志或实时状态为证据。
- 安装第三方 skill、插件、工具或依赖前列出候选并等待明确授权。
- 不把密钥写入代码、仓库文档或 Agent 配置。

远端、GUI、安装和临时文件规则见同目录 `host-operations.md`，仅在相应任务中读取。
