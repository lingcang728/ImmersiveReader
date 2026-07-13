# 沉浸阅读 1.1.0

1.1.0 将 Markdown 阅读、知乎归档和 Podcast 转写统一到同一座本地书库，并补齐任务恢复、来源溯源、数据迁移和安全收尾能力。

## 主要变化

- 书架详情展示来源、revision、provenance、章节和关联任务记录。
- 保留 MMbook 的 Focus Mode、章节导航、滚动恢复和 viewport-anchor 行为。
- Podcast 与知乎使用统一任务快照、结构化事件、幂等控制和受管运行时路径。
- Podcast 支持本地 faster-whisper ASR、DeepSeek/Ollama 翻译与预算预检；凭据只存 Windows Credential Manager。
- 知乎支持受管登录、回答/文章合并 Top N、`.incoming` 发布事务和 archive revision。
- 正式数据迁移提供 receipt、reconciliation、rollback 和 Profile/Library 路径隔离。
- 桌面端收紧 CSP、外链协议、文件访问和 Tauri capabilities；Release Action 只构建 Windows x64 NSIS。

## 数据边界

真实书库、登录态、数据库、临时音频、模型、输出和本地配置不属于 Git 发布内容。生产安装、默认应用选择和远端历史均有独立 QA 证据，详见同目录 [QA_REPORT.md](QA_REPORT.md)。
