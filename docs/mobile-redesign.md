# Android 移动端重设计规范（1.3.0）

> 目标：把 Android 端从「桌面页面的响应式补丁」改为独立的本地阅读器前端：只保留 **本地 Markdown / EPUB 阅读**，复用桌面端 MMbook 设计语言与阅读能力，冷启动快、滑动稳、内存低、适配 OPPO Find X9 与 Android 8+ 主流机型。
>
> 桌面端：外观与交互 **零变化**；只做内部拆分/懒加载与 Rust 启动优化。Android 端：全新 UI，不加载桌面页面代码。

## 1. 范围

保留（Android）：

- 本地书架：书库扫描、继续阅读、最近阅读、EPUB/Markdown 标识、搜索书名、导入（SAF 多选）、打开/分享 intent 自动导入并打开、移入回收站。
- 阅读：Markdown（库内多章书 + 单文件）、EPUB 2/3（无 DRM、可重排）、章节切换、恢复进度、目录、书内搜索、EPUB 书签。
- 排版：6 套 MMbook 主题（素纸/墨石/暮光 × 明暗）、字号、行高、页宽、字体族；AMOLED 深色。
- 专注模式：聚光灯 + 渐进模糊 + 点击跳句 + 底部 HUD（复用桌面 `splitSentences` / viewport anchor 算法）。
- 设置：只保留阅读相关 + 存储/缓存 + 版本关于。

移除（Android 不出现在 DOM/代码路径）：

- 知乎归档/索引/连读（reader server）、播客转写/任务队列/DeepSeek 密钥、桌面窗口 chrome、拖拽、快捷键说明、桌面路径 reveal、桌面更新检查、迁移/备份/发布恢复等运维面板。
- 不 import 任何桌面页面组件：`lib/desktop/**`、`Bookshelf.svelte`、`SettingsPanel.svelte`、`PodcastWorkflow`、`ZhihuWorkflow`、`TrashPanel`、`WindowChrome` 等。

## 2. 信息架构

```
MobileApp (fixed viewport, safe-area aware)
├─ Boot         后台并发：platform caps / library scan / intent drain，先渲染骨架
├─ Shelf ────── 继续阅读 hero → 书列表（虚拟/分批）→ FAB 导入；顶部搜索 + 设置
├─ Reader ───── 全屏阅读（shelf 之上覆盖，返回即回书架）
│   ├─ chrome 顶部栏（自动隐藏）：返回 / 书名+章节 / 目录 / 搜索 / 更多
│   ├─ 正文滚动区（MD 或 EPUB）
│   ├─ 底部进度条（章内 + 全书）+ 上一章/下一章
│   ├─ Sheets：目录 / 搜索 / 书签 / 阅读设置 / 章节列表
│   └─ 专注模式：聚光灯 + 底部 HUD
└─ Settings ─── 主题 / 排版 / 翻页 / 缓存 / 关于
```

状态机：`boot → shelf ⇄ reader`，`shelf ⇄ settings`。返回手势：reader 先关 sheet，再回 shelf；shelf 不拦截系统返回（让系统退出 App）。用 `history.pushState` + `popstate` 维护层级，避免吞掉后续返回（复用 B1 修复思路）。

## 3. 视觉语言（沿用桌面 MMbook）

- 颜色：只用 `lib/theme/themes.ts` 的 CSS 变量（`--bg/--bg-secondary/--text/--text-secondary/--text-faded/--heading/--link/--danger/--hr/--code-bg/--selection` 等），不新增品牌色；链接蓝仅用于焦点/进度/次级操作。
- 字体：UI 用系统无衬线（`system-ui, -apple-system, "Noto Sans SC"`）；正文默认无衬线、可选衬线（`readingFontFamily`）。
- 间距：4px 节奏；页面左右安全边距 20px；触控目标 ≥ 44×44；圆角 12/16px；边框 1px `--hr`。
- 动效：150–220ms `cubic-bezier(.2,.7,.2,1)`，只动 `transform/opacity`；列表入场 stagger ≤ 6 项；尊重 `prefers-reduced-motion`。
- 禁止：玻璃拟态、彩色渐变、装饰 emoji、金色/铜色、统计卡片。
- 深色：跟随 `currentTheme`（已含 OS 跟随逻辑），纯黑让位 AMOLED 省电由 `moshi-dark`（#141414）/`suzhi-dark` 提供，不新增纯黑主题。
- 安全区：`--ir-safe-top/bottom/left/right`（Kotlin 注入，CSS px）优先，fallback `env(safe-area-inset-*)`：
  `padding-top: max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px))`，底部同理。底部进度条/HUD 必须留 ≥ 24px 手势条间距。

### Find X9 画像

6.59" AMOLED，2760×1256，~120Hz LTPO，居中挖孔，ColorOS 16 / Android 16。竖屏 CSS 宽约 360–430px（按 density）；布局全部流体/`clamp()`，不写死 393px。横屏（可选）保持可用：正文最大行宽 `min(100%, 46rem)`，工具栏不重叠挖孔。

## 4. 页面规格

### Shelf

- 顶部：左侧 `沉浸阅读`（24px，`--heading`）+ 副标题「本地 EPUB · Markdown」；右侧 44px 图标按钮：搜索、设置。
- 继续阅读 hero（仅当有最近书）：整卡可点；封面 56×76（EPUB 有封面图则 `convertFileSrc`，否则 4 字书名首字 + `--bg-secondary` 底）；书名 17px/600 两行截断；当前章节 13px `--text-secondary`；进度条 4px + 百分比；右侧「继续」按钮（`--link` 描边）。
- 书列表：每行 72px：封面缩略 44×60 / 信息（书名 16px、`EPUB`/`Markdown`/`N 章` 13px、最近阅读时间）、右侧进度环或百分比；超过 100 本分批渲染（每批 30，IntersectionObserver）。
- 空状态：图标 + 「书库还是空的」+ 说明 + 「导入 EPUB / Markdown」主按钮。
- 导入 FAB：底部右侧 56px 圆形（`--link` 底、`--on-accent` 图标），距安全区 20px；导入中变成进度胶囊（文件名/百分比/取消）。
- 删除：长按书卡 → 底部操作表「移入回收站」；失败 toast。
- `data-testid`: `shelf`、`shelf-continue`、`shelf-book-<bookId>`、`shelf-import`、`shelf-settings`、`shelf-search`、`shelf-empty`、`import-sheet`。

### Settings

- 顶部返回 + 「设置」。
- 分组：外观（主题三选一明暗切换）、排版（字号滑杆 `-`/`+`、行高 3 档、页宽 3 档、字体族 2 档）、翻页（音量键翻页开关、自动进入专注开关）、存储（书库/缓存用量、「清理缓存」）、关于（版本、开源说明）。
- 不出现知乎/播客/连读/桌面路径/更新检查。`data-testid="settings-screen"`。

### Reader

- 进入：书架点书 → 全屏；先显示骨架（标题 + 3 行灰条）立即渲染，不阻塞 chrome 出现。
- 顶部栏：高 52px + 安全区；左返回（44×44）、中间两行（书名 15px/600 单行、章节 12px `--text-secondary`）、右侧目录/搜索/更多（44px，间距 4px）；背景 `--bg` + 底边 `--hr`，随 chrome 显隐淡入。
- 正文：滚动容器 `overflow-y:auto; -webkit-overflow-scrolling:touch; overscroll-behavior:contain;`；左右 padding 20px；`font-size: calc(17px * var(--font-scale))`；`line-height: var(--article-line-height, 1.8)`；`max-width: var(--article-max-width, 760px)` 居中（手机上即满宽）；`--article-font-family`。段落间距 1em；标题层级 1.6/1.35/1.2em；代码块 13px 等宽、横向滚动；表格横向滚动；图片 `max-width:100%; height:auto; loading="lazy"`。
- 性能：正文渲染后对块级子元素加 `content-visibility:auto; contain-intrinsic-size:auto 600px`（首屏元素除外）；大章节（>1MiB）分块挂载；切换章节时新章先渲染、旧章立即移除；不保留整本文档 DOM。
- 触控：单击正文中部切换 chrome；左侧 25% 双击/单击上一页、右侧 25% 下一页？→ 移动端以「单击中部切 chrome、左右边缘轻点翻页（平滑滚一屏 82% 高）、横滑切章（>60px，排除选择/横向滚动容器/代码块/表格）」；长按选词时挂起翻页/滑动手势；多指/取消不触发。
- 音量键：开关开启且阅读前台时，上/下 = 上一页/下一页（专注模式 = 上一句/下一句），由 Kotlin `ir:volume-key` 事件驱动，调用 `setVolumeKeyCapture` 控制。
- 底部进度：chrome 显示时出现，4px 轨道 + 章内百分比；下方「第 x/y 章 · 全书 z%」13px；左右 44px 上一章/下一章按钮。
- 章节无缝：滚动到章末 92% 显示「下一章」提示条；切章后停在顶部/底部边界（上一章时停在新章末尾）。
- 目录 sheet：底部 80% 高、圆角 20px、拖拽条；MD 显示章节列表（当前章高亮 `--link`，右侧已读勾选）；EPUB 显示多级 nav（缩进 16px/级）；点击关闭并跳转。`data-testid="toc-sheet"`、`toc-item-<i>`。
- 搜索 sheet：全高底部抽屉；输入框自动聚焦（键盘弹出用 `visualViewport` 调整）；MD 章内搜索（标题/正文，高亮 `--search-highlight`，上/下一个），EPUB 用 `search_book` 全书搜索（snippet 高亮、点击跳 locator）；空结果提示。`data-testid="search-sheet"`。
- 书签 sheet（EPUB）：列表 + 当前页「添加书签」；点击跳转、左滑/按钮删除。
- 阅读设置 sheet：主题网格（3 组明暗）、字号 A-/A+、行高、页宽、字体族、音量键翻页；改动立即生效并持久化（复用 `lib/stores/app.ts` 的 writable）。
- 专注模式：点击正文句子 → 该句聚光灯（其余句 opacity 递减 + 渐进模糊），视口平滑对齐到黄金锚点；底部 HUD 胶囊（上一句 / 第 x / y 句 / 下一句 / 退出），距安全区 16px；音量键/左右轻点切换句子；退出时 viewport anchor 恢复原位置。`data-testid="focus-hud"`。
- EPUB 资源：章节 HTML 经 `sanitizeChapterHtml`；图片/字体通过 `convertFileSrc(join(bookDir, resourceDir, relative))`；外链仅 http(s) 用系统浏览器；内链解析为 chapterId + fragment；书内 CSS 作用域到 `.epub-article`。
- 进度写入：滚动 ~1s 节流 + 切章 + 退出 + `visibilitychange/pagehide` 立即 flush；MD 库内书用 `save_book_progress`（chapter/position/read），单文件用 `save_reading_state`，EPUB 用 `save_reader_locator`；保存失败只 toast 一次，不阻塞。

## 5. 数据与代码结构

```
src/lib/mobile/
├─ api.ts            # 唯一 IPC 边界（lead 提供，勿改签名）
├─ contracts.ts      # 类型契约（lead 提供）
├─ MobileApp.svelte  # shell：boot、view、back 栈、toast、intent drain
├─ Shelf.svelte
├─ SettingsScreen.svelte
├─ ImportSheet.svelte
├─ components/Sheet.svelte, Toast.svelte, Icon.svelte
├─ reader/
│   ├─ ReaderScreen.svelte   # props: { target: ReaderTarget; onExit: () => void }
│   ├─ MarkdownReader.svelte # 库内多章 + 单文件
│   ├─ EpubReaderMobile.svelte
│   ├─ FocusMode.svelte, TocSheet.svelte, SearchSheet.svelte,
│   ├─ BookmarksSheet.svelte, ReadingSettingsSheet.svelte, ReaderChrome.svelte
│   └─ progress.ts           # 纯函数 + vitest
└─ *.test.ts
```

- 入口：`routes/+page.svelte` 在移动端动态 `import("$lib/mobile/MobileApp.svelte")`；桌面端动态 `import("$lib/desktop/DesktopApp.svelte")`。
- 移动端禁止静态 import：`$lib/desktop/**`、`$lib/components/**`（除 `components/epub/**` 由阅读器按需覆盖）、`$lib/epub/EpubReader.svelte` 只能被 reader 动态 import。
- Markdown 渲染复用 `$lib/render/markdown.ts` 的 worker 管线；移动端通过 URL query/首参选择轻量模式（关闭或按需 Shiki/Mermaid；KaTeX 仅在出现 `$` 时加载），**不新增 Markdown 解析器**。
- EPUB 阅读内核复用 `$lib/epub/{ipc,dom,paths,locator,sanitize,timing}.ts`；不得出现第二套 locator/进度语义。
- 状态：主题/字号/行高/页宽/字体/专注开关复用 `$lib/stores/app.ts`；书库快照/阅读目标/视图由 MobileApp 持有；不引入全局事件总线。
- 图标：内联 SVG（stroke=currentColor），不引入图标库。
- 所有 async 调用包 try/catch + 中文错误文案（`api.errorText`）；首屏只允许并发 ≤ 3 个 IPC。

## 6. 性能预算与验收

| 指标 | 目标 |
|---|---|
| 移动端登录包（shell 首屏 JS，未压缩） | ≤ 250KB；桌面 chunk 不在移动端加载 |
| 冷启动到书架可交互（Find X9） | ≤ 1.8s（`am start -W` WaitTime 记录） |
| 打开一本 1MB Markdown 到正文可见 | ≤ 800ms（骨架 ≤ 100ms 出现） |
| 滚动掉帧 | `dumpsys gfxinfo` 慢帧 < 5%，无 >32ms 主线程任务连续出现 |
| 阅读时 PSS | ≤ 260MB（EPUB 单章渲染后） |
| 后台切换 | 停止动画/预读/定时器；恢复不重新全量扫描 |
| 内存泄漏 | 关闭书后清空章节 DOM/大字符串/对象 URL |

## 7. 测试与适配

- vitest：进度/章节映射、URI 判定、导入状态、排序/分组、手势仲裁纯函数。
- Playwright mock 冒烟（scripts/qa/mobile）：三种视口 × 书架/阅读/目录/设置/深色；console error = 0。
- 真机：OPPO Find X9（主）、Android 8/12/14 模拟器或云真机矩阵（无设备则诚实标注未验证）；覆盖字体放大、深色、安全区、横竖屏、系统返回手势、音量键、通知栏下拉后恢复。
- 无障碍：所有图标按钮有 `aria-label`；正文选词/朗读不被手势吞；对比度 ≥ 4.5:1（主题 token 已满足）；`prefers-reduced-motion` 关闭动画。
- 构建：release APK，arm64-v8a 主包 + universal 兼容包；minSdk 26/target 36；新签名密钥，需卸载旧 1.2.1 后安装。

## 8. 协作边界

- `mobile-shell`：`lib/mobile/` 下除 `reader/`、`api.ts`、`contracts.ts` 之外的文件 + 测试。
- `mobile-reader`：`lib/mobile/reader/**`、`lib/epub/**`、`lib/components/epub/**`（桌面路径零变化）。
- Lead：`api.ts`、`contracts.ts`、`MobileApp.svelte` 初始 stub、路由分流（与 `frontend-perf` 协作）、版本/构建/真机/文档。
