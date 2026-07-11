# Zhihu Packer

Zhihu Packer 是一个本地运行的知乎内容归档与本地阅读器生成工具，面向个人备份、离线阅读和知识整理场景。

它的核心目标是把可访问的知乎回答或文章整理成本地文件，并在本机生成便于检索和阅读的 Reader。项目包含 CLI、Web 控制台、Playwright 登录态、SQLite 状态持久化和 Reader Web 等模块。

## 当前能力

### 已实现

- TypeScript 项目骨架。
- CLI 入口与多个 package scripts。
- Playwright 依赖和独立浏览器 profile 目录约定。
- SQLite 本地数据库文件约定。
- Express 本地 Web 控制台入口。
- Reader 相关源码结构，包括扫描、元数据、搜索、Markdown 渲染和 UI 逻辑。
- 本地输出目录和临时 scratch 目录约定。

### 规划中

- 更清晰的单篇回答/文章归档流程。
- 作者主页索引、任务队列和断点续跑。
- 失败分类、重试策略和 dry-run 模式。
- Web 控制台中的任务状态、日志、暂停和重试能力。
- Reader 的离线打包、搜索体验和元数据展示。
- 示例数据、脱敏演示和发布说明。

### 待确认

- 各 CLI 子命令的完整参数和稳定性。
- `doctor` 的具体检查项是否会联网、启动浏览器或写日志。
- Web 控制台默认监听地址、端口和可访问范围。
- Reader 产物的具体输出路径和打包方式。
- 数据库 schema 与 output 文件之间的一致性恢复策略。

## 技术栈

- Node.js
- TypeScript
- Playwright
- Express
- SQLite
- DOMPurify
- marked
- pinyin-pro
- esbuild
- Reader Web

## 项目结构

```text
src/
  cli.ts                 CLI 入口
  index.ts               开发入口
  doctor.ts              环境自检相关逻辑
  login.ts               Playwright 登录态初始化
  browser.ts             Playwright 浏览器封装
  db.ts                  SQLite 数据库封装
  indexer.ts             内容索引相关逻辑
  extractor.ts           正文提取相关逻辑
  scheduler.ts           任务调度相关逻辑
  server.ts              本地 Web 控制台入口
  build-html.ts          Reader HTML 生成入口
  compile-reader.ts      Reader 编译入口
  serve-reader.ts        Reader 本地预览入口
  reader/                Reader Web 核心、模式和 UI
public/
  index.html             Web 控制台静态页面
tools/
  *.py                   可选辅助脚本
.browser-profile/        Playwright 登录态，本地私有数据
output/                  归档输出，本地私有数据
scratch/                 临时处理目录，本地私有数据
zhihu-packer.db*         SQLite 运行数据，本地私有数据
```

## 命令说明

以下命令来自 `package.json`。风险标签表示对本地数据、登录态、网络和写入行为的保守判断。

| 命令 | 脚本 | 风险 | 说明 |
|---|---|---|---|
| `npm run dev` | `tsx src/index.ts` | 中风险 | 开发入口，可能触发本地数据读写，具体行为需确认。 |
| `npm run build` | `tsc` | 低风险 | TypeScript 编译，主要用于类型检查和生成 `dist/`，但会写入 `dist/`。 |
| `npm run doctor` | `tsx src/cli.ts doctor` | 中风险 | 环境自检，可能联网、启动浏览器、检查输出目录或写日志。 |
| `npm run login` | `tsx src/cli.ts login` | 高风险 | 会启动 Playwright 浏览器，并写入 `.browser-profile/` 登录态。 |
| `npm run cli` | `tsx src/cli.ts` | 中到高风险 | 通用 CLI 入口，风险取决于子命令，可能写数据库或 output。 |
| `npm run web` | `tsx src/server.ts` | 中风险 | 启动本地 Web 控制台，可能读取本地 DB/output/log，也可能触发任务。 |
| `npm run build-reader` | `tsx src/build-html.ts` | 中风险 | 可能读取 output，并写入 Reader 生成产物。 |
| `npm run compile-reader` | `tsx src/compile-reader.ts` | 中风险 | 可能编译 Reader UI 或写入 Reader 产物。 |
| `npm run serve-reader` | `tsx src/serve-reader.ts` | 中风险 | 可能启动本地 Reader 预览服务，并读取本地归档内容。 |

在确认数据边界前，不建议把 `doctor`、`login`、`web` 或 Reader 相关命令当作纯只读命令使用。

## 本地数据与隐私边界

以下内容永远不应提交 Git：

- `.browser-profile/`
- `zhihu-packer.db*`
- `output/`
- `scratch/`
- `*.log`
- `.env*`
- `storageState*.json`
- `cookies*.json`
- `playwright/.auth/`

原因：这些文件或目录可能包含 cookies、localStorage、sessionStorage、任务状态、URL、作者信息、归档正文、运行日志、环境变量或其它本地私有数据。

当前推荐策略：

- 源码、配置、文档可以纳入 Git。
- 登录态、数据库、日志、真实 output 和临时 scratch 保留在本机。
- 如需展示项目，应使用脱敏示例数据，不使用真实归档内容。

## 合规说明

- 本项目仅用于个人备份、离线阅读和知识整理。
- 只应归档自己有权限访问的内容。
- 不建议公开分发第三方内容或真实归档结果。
- 不建议高频抓取，应保持低速、克制、可暂停的使用方式。
- Web 控制台应作为本地工具使用，不应暴露到公网。
- 不应使用日常 Chrome Profile，应使用项目独立的 Playwright profile。

## 后续路线

### 阶段 1：工程治理

- 完善 `.gitignore`。
- 补充 README。
- 明确本地数据目录和隐私边界。
- 初始化 Git 前确认不会提交登录态、数据库、日志和真实输出。

### 阶段 2：稳定运行

- 明确 `doctor` 的只读检查和写入检查边界。
- 增加或完善 dry-run 模式。
- 完善错误码、失败重试和任务恢复策略。
- 明确 DB 与 output 不一致时的处理方式。

### 阶段 3：阅读体验

- 改进 Reader 首页、搜索和元数据展示。
- 支持 packed/universal 等阅读模式。
- 优化 Markdown 渲染和本地资源解析。
- 提供脱敏示例内容用于演示。

### 阶段 4：打包发布

- 梳理发布包内容。
- 提供最小使用手册。
- 分离真实本地配置和示例配置。
- 明确隐私、合规和数据清理流程。
