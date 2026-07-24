# Host Operations Rules

## 远端与跨终端

查询另一台机器前先读 `$HOME/.ssh/config` 中相关 Host 块；用
`ssh -G <host>` 展开参数。不得凭记忆猜 IP、用户名或端口，也不默认读取私钥、
known_hosts 或历史会话。

## 本机 GUI

用户明确要求打开本机文件或操作原生应用时优先 `cua-driver`。读取代码、文本、日志或
diff 使用 shell/结构化工具；localhost 网页测试优先 Browser/Playwright。只读或不操作
界面的要求不得启动 GUI。

## 安装与敏感信息

安装第三方 skill、插件、工具或依赖前先列出候选并等待明确授权。不得把 API key、
token、secret 或 password 写进仓库和 Agent 配置；使用环境变量或系统密钥管理。

## 临时文件

交付前清理由本次任务创建的 `/tmp` 文件、临时 workspace、调试 dump 和中间脚本。
保留正式设计文档、实施计划及用户明确要求保留的产物。
