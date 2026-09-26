# ImmersiveReader 手机版修复 + EPUB 支持 — 任务看板

> 基线：`07c3d1d`。来源：`C:\Users\15pro\Desktop\ir手机版.txt`（Android 阅读修复与 Windows／Android EPUB 支持计划）。
> 图例：⬜ 待做 · 🔨 进行中 · ✅ 完成 · ⚠️ 完成但需真机验证 · ❌ 受阻

## 第一阶段 · 手机阅读基础（交互/返回/进度）

| # | 条目 | 状态 | 说明 |
|---|------|------|------|
| B1 | 系统返回栈修复（`history.go(-N)` 误吞后续返回） | 🔨 | frontend-mobile Agent |
| B2 | 横滑误切章节：排除代码块/表格/横向滚动容器 | 🔨 | frontend-mobile Agent |
| B3 | 专注底栏跨章导航（消费 moveFocus 返回值） | 🔨 | frontend-mobile Agent |
| B4 | 统一触控仲裁（选择/控件优先→横滚→翻页；多指/取消失效） | 🔨 | frontend-mobile Agent |
| B5 | 原生音量键桥接，仅前台阅读+开关启用时接管 | 🔨 | android-native（Kotlin）+ frontend（事件消费）|
| B6 | `visibilitychange`/`pagehide`/原生暂停触发进度快照；保存串行化+修订号 | 🔨 | frontend-mobile Agent |
| B8 | 手机隐藏快捷键说明、返回按钮加宽、目录不自动聚焦 | 🔨 | frontend-mobile Agent |
| — | 原生 Insets/VisualViewport 安全区与键盘协调 | 🔨 | env() 安全区内边距 |

## 第二阶段 · 导入与存储

| # | 条目 | 状态 | 说明 |
|---|------|------|------|
| A1 | Kotlin `copyToDir` 移出主线程（后台 I/O） | 🔨 | android-native Agent |
| A2 | 超时≠取消：`begin_import`/`get_import_status`/`cancel_import` 操作模型 | 🔨 | rust-core Agent（import_ops.rs）|
| A3 | 临时文件+原子改名，失败不留半文件 | 🔨 | android-native Agent |
| A4 | 保留 Markdown 相对目录、图片、附件；章节自然排序 | 🔨 | rust-core Agent（importer.rs）|
| A5 | 暂存路径规范化校验（防 `..` 绕过），Kotlin 侧同名清洗+规范校验 | 🔨 | 两端 Agent |
| A6 | 复制前大小/总量/剩余空间预算；不强制改 `.md` 后缀 | 🔨 | 两端 Agent |
| A7 | 统一手机缓存根目录与清理统计 | 🔨 | rust-core Agent |
| A8 | 前端展示逐文件 issues（成功/跳过/失败原因），文集名可编辑 | 🔨 | frontend-mobile Agent |
| A9 | 超时预算表统一（`stage_content_uri` 入表，内外一致） | 🔨 | frontend-mobile（ipc.ts）|
| B7 | AndroidManifest：MD/EPUB/ZIP 的 VIEW + SEND/MULTIPLE + onNewIntent | 🔨 | android-native Agent |
| — | SAF 目录树/多选/ZIP 文集统一入口（选择→暂存→校验→提交→清理） | 🔨 | 三端协作（begin_import ops）|
| C7 | 书库/进度/书签/偏好导出与恢复（手机走系统文档选择器） | 🔨 | rust-core + frontend |

## 第三阶段 · 数据库与资源预算

| # | 条目 | 状态 | 说明 |
|---|------|------|------|
| C4 | 阅读不依赖 control.db（任务信息失败仅降级） | 🔨 | rust-core Agent |
| C5 | `bookId→目录` 索引（reader.db），翻章/存进度不再全库扫描 | 🔨 | rust-core Agent（reader_db.rs）|
| C1 | 手机隐藏无运行时的"连读"入口 | 🔨 | capabilities + frontend 门控 |
| C2 | "打开书库/缓存目录"在 Android 上的降级处理 | 🔨 | frontend-mobile Agent |
| C3 | 手机密钥：Keystore 插件存储、明文 `.key` 迁移、排除备份、删除入口 | 🔨 | android + rust-core |
| C6 | 手机跳过任务维护/轮询启动成本 | 🔨 | frontend-mobile Agent |
| C8 | DOM 分块挂载、最多预读一章、渲染缓存 64MiB、>8MiB 分块、>64MiB 拒绝 | 🔨 | frontend-mobile（分块挂载安全子集）|
| C9 | 后台停止轮询/预读/动画；缓存预算与可回收条目 | 🔨 | frontend-mobile Agent |
| — | reader.db：书目索引、全文搜索索引、EPUB 定位/书签 | 🔨 | rust-core Agent |
| — | 日志：操作ID/阶段/耗时/错误码 + 手机导出诊断 | 🔨 | import_ops 状态机覆盖核心路径 |

## 第四阶段 · EPUB（Windows + Android 共享）

| # | 条目 | 状态 | 说明 |
|---|------|------|------|
| E1 | Rust EPUB 解析：ZIP+XML，EPUB2 NCX/EPUB3 nav，封面/作者/spine/图片/脚注/内链 | 🔨 | epub-rust Agent |
| E2 | 安全：禁路径穿越/符号链接/DTD/外部实体；归档≤256MiB、展开≤1GiB、≤10k 条目 | 🔨 | epub-rust Agent |
| E3 | `publication.json`（带版本）+ v1 manifest 兼容；旧书缺文件按 MD 处理 | 🔨 | epub-rust Agent |
| E4 | `ReaderLocator`（章节+元素/文本锚点+章内比例），书签/搜索同模型 | 🔨 | epub-rust + epub-frontend |
| E5 | 渲染：净化 HTML/SVG/CSS，去脚本/事件/表单/外链 CSS；资源限本书目录 | 🔨 | epub-rust（导入净化）+ epub-frontend（DOMPurify）|
| E6 | 前端 EPUB 阅读：纵滚、跨章、目录、全书搜索、书签、续读、主题/字号/专注 | 🔨 | epub-frontend Agent |
| E7 | 新命令：`get_platform_capabilities`/`begin_import`/`get_import_status`/`cancel_import`/`get_readable_chapter`/`save_reader_locator`/书签/`search_book` | 🔨 | rust-core Agent 注册 |
| E8 | 契约同步：Rust+TS+JSON Schema+fixtures+测试 | 🔨 | epub-rust Agent |

## 第五阶段 · 构建与交付

| # | 条目 | 状态 | 说明 |
|---|------|------|------|
| F1 | build-android.ps1：只收集本次构建 APK（变体/架构/时间限定）、失败可靠抛错 | ✅ | 主会话已改：时间水位线 + 变体目录匹配 + 双 throw |
| F2 | Gradle Release 签名配置（有密钥时启用）+ QA 独立 applicationId | 🔨 | android-native Agent（keystore.properties 存在才启用）|
| F3 | `scripts\verify.ps1` 全绿 | ⬜ | 集成后跑 |
| F4 | `ship:local` 生产包 + 提交号/EXE 时间戳/SHA-256 | ⬜ | |
| F5 | 逐条问题关闭表 + 未验证项标记 + git push | ⬜ | |

## 并行工作流（5 Agent）

| Agent | 负责域 | 状态 |
|-------|--------|------|
| rust-core | `src-tauri/src/`（reader_db.rs、import_ops.rs、importer/library/lib.rs、secrets、导出恢复）| 🔨 |
| android-native | `gen/android/**`（Kotlin 插件、MainActivity、Manifest、Gradle、备份规则）| 🔨 |
| epub-rust | `src/epub.rs`、`contracts.rs`、`packages/contracts/**` | 🔨 |
| frontend-mobile | `+page.svelte`、`lib/**`（除 epub/）、现有组件 | 🔨 |
| epub-frontend | `lib/epub/**`、`components/epub/**`（新文件）| 🔨 |

## 进度日志

- 2026-09-26：建立看板；4 个勘察 Agent 完成摸底（交互/导入/后端/渲染+构建四份报告）
- 2026-09-26：定下跨端接口契约（epub.rs 公共 API、EpubReader 组件契约、14 个新 Tauri 命令、reader.db 表结构）；Cargo.toml 增加 zip+quick-xml 直接依赖；build-android.ps1 修复完成；5 个实施 Agent 并行开工
