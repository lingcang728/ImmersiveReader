# 2026-10-18 性能审计（只读）

- 目标：Android 端重做为流畅本地 Markdown/EPUB 阅读器 + 修复 1.2.1 Android 启动崩溃；同时记录桌面端启动/内存证据。
- 审计性质：**只读**。未运行 `vite build`、未修改 `src/**`、`src-tauri/**`、`gen/android/**`；所有数字来自现有构建产物、源码静态分析、`git show HEAD:...` 与 APK 解包，数据来源逐条标注。
- 基线提交：`73cf378`（本轮工作树另有并行改动；涉及源码行号均标注 `@73cf378`，工作树当前态另行注明）。
- 环境：Windows 11 26300 / Node v24.15.0 / Rust 1.92.0（前一审计记录）/ Python 3.12.7 + Playwright 1.62.0（全局已装）。
- 归属标记：`frontend-perf` / `rust-core` / `android-native` / `mobile-ui`。

## 0. 测量口径与数据来源

| 数据 | 来源 | 说明 |
|---|---|---|
| 旧构建产物（1.2.1 基线） | `apps/desktop/build/**`（2026-09-26 23:50 生成，本轮 18:44–18:48 读取时仍在） | 被 frontend-perf 于 18:49 重建覆盖；下列标 **[baseline-build]** 的数字为覆盖前实测记录 |
| 最终构建产物（1.3.0） | `apps/desktop/build/**`（2026-10-18 19:02 生成，version.json hash `1792321319455`） | 标 **[final-build]**；移动壳/Markdown/EPUB 阅读器已实现，详见 `QA_REPORT.md` |
| 浏览器端实测（最终 build） | `scripts/qa/mobile/verify_mobile.py`（Playwright + 系统 Chrome，3 视口） | 首屏资源字节来自 `PerformanceResourceTiming`；标 **[browser-measured]** |
| 中间产物（分流 WIP，已被覆盖） | 2026-10-18 18:49 构建 | 本报告部分对比数字来自覆盖前实测，标 **[wip-build]** |
| 构建图/体积/重复度 | `docs/audit/2026-10-18/scripts/analyze-build.mjs` + `build-analysis.json` | 解析 `build/_app/immutable` 的 import/export 与动态 import；worker↔main 用 token 多重集 Dice 相似度判重 |
| Rust 成本路径 | `git show 73cf378:apps/desktop/src-tauri/src/*.rs` | 行号 `@73cf378`；当前工作树修复态另列 |
| APK 证据 | `release/ImmersiveReader_1.2.1_android-universal-release.apk`（16,635,119 B，SHA-256 `F560BC74…A1638C`，即 1.2.1 QA 报告中的产物） | ZIP 解包 + dex 字符串扫描 |
| 历史合成基准 | `docs/audit/2026-07-15/BASELINE.md`（synthetic-temp-data） | 仅引用明确标注为合成数据的旧数字 |

复现命令：

```powershell
node docs/audit/2026-10-18/scripts/analyze-build.mjs          # 人读版
node docs/audit/2026-10-18/scripts/analyze-build.mjs --json > docs/audit/2026-10-18/build-analysis.json
```

## 1. 结论摘要（P0 / P1）

| 级别 | 问题 | 关键证据 | 影响 | 归属 |
|---|---|---|---|---|
| **P0** | Android 启动崩溃根因：Android 路径注册了不存在的 Kotlin 插件 `SecretsPlugin` | `apps/desktop/src-tauri/src/lib.rs:4006-4012 @73cf378`；1.2.1 APK `classes.dex` 中 `SecretsPlugin` 出现 **0** 次、`ContentReaderPlugin` **6** 次 | setup 返回 Err → `build().unwrap_or_else(panic!)` → 进程崩溃，WebView 根本不加载（用户复现为"打开即闪退"） | rust-core（已修复；真机验证：1.3.0 冷启动 FATAL/ANR=0，见 §6/`QA_REPORT.md`；1.2.1 崩溃真机实证见 §2.1） |
| **P0** | Android `setup()` 同步执行桌面级恢复/清扫，阻塞 WebView 首帧 | `lib.rs:4143-4195 @73cf378`：control.db 打开+`recover_interrupted_tasks`、import 暂存清扫、publish residue/transaction 恢复、marker 清扫+publish 索引 reconcile 全在 setup 同步跑 | 冷启动 I/O + SQLite 事务叠加在首帧之前；书架大的用户可能数百 ms 到数秒，ANR 风险 | rust-core（已门控 + 后台清扫；真机冷启动 TotalTime 141–154 ms、无 ANR，见 QA_REPORT §5.2） |
| **P1** | Markdown 渲染管线在构建里存在 **两份完整拷贝**（worker 一份 + 主线程 fallback 一份），共 2,283,240 B 重复 | [final-build] worker 全图 2,283,381 B/22 文件；主线程 fallback 图根 `chunks/hOfXeXfW.js` 375,368 B；22 对 chunk token Dice≈1.0（如 `workers/chunks/Cux-tB8m.js 637,581B ↔ chunks/Ck_RvX-X.js 637,547B`），重复合计 2,283,240 B | 构建/APK 体积 +2.23 MB（APK 的 lib 13.75 MB 内含前端资源）；若 worker 失败回退主线程，手机端要再解析 2.2 MB 管线，阅读卡顿 | frontend-perf |
| **P1** | `scan_library` 每次扫描递归 stat 书库**整棵子树**（含每本书的章节/图片 payload）并逐本读 manifest+progress | `library.rs:90-195`（`collect_manifests` 对每个 entry `metadata()`，进入书目录后仍递归遍历 payload）；`library.rs:375` `load_book_at` 每本读 manifest+`.reading.json`；`library.rs:479-498` 每次扫描开 reader.db 全表比对+逐本 upsert；`lib.rs:1936-1942` 外层还读 settings | 书架越大越慢（旧审计合成数据：1,000 本 manifest-only 扫描 p50 214 ms，安卓内部存储更慢）；移动端每次 resume 都触发 `refreshLibrary`（HEAD `+page.svelte:3713-3721`）→ 频繁全库扫描 | rust-core + mobile-ui |
| **P1** | 每个阅读热路径 IPC 都重新读 `settings.json` 并多次重算 `StorageLocations::current()` | `settings.rs:202-226 @73cf378`：`load_settings()` 每次 `load_compatible_from`→`fs::read_to_string`（`settings.rs:110`），再 `validate_library_root`+`normalize_production_library_root`（各自再调 `StorageLocations::current()`，`storage.rs:162` 含 env/dirs/current_exe 查询）；`scan_library/open_book/get_book_chapter_path/save_book_progress/get_readable_chapter` 全部调用它（`lib.rs:1939/1962/1990/2015/2348`） | 翻章/存进度/开书都多一次磁盘读+路径计算；进度保存约 1 次/秒（HEAD `+page.svelte` 节流保存），在手机上放大抖动 | rust-core |
| **P1** | `reader.db` 每个命令都 `Connection::open`（连开库、建目录、schema 检查） | `reader_db.rs:150-210 @73cf378` `open_current→open_at→open_inner`；调用点 `reader_db.rs:367`(book_dir)/`399`(save_locator)，`find_book` 每次开书查目录（`library.rs:538`）；EPUB 滚动每 ~2s `save_reader_locator`（`lib/epub/ipc.ts` 注释明确为热路径） | 翻章/存进度/存 locator 各开一次 SQLite；Android 上带来额外 IO/CPU，非致命但纯属可省 | rust-core |
| **P1** | 移动端首屏预算需要持续门禁（已建立 harness 门禁，当前最终 build 达标） | [final-build] 构建图初始闭包 89,385 B（JS 88,363 + CSS 1,022）；[browser-measured] 首屏 JS 159,881 B / CSS 24,092 B（含移动壳 chunk `CheMj8JJ.js` 44,372 B + `MobileApp.css` 22,470 B）；桌面壳 `C8pXMjSg.js` 187,791 B 为 lazy，移动端未请求；[baseline-build] 首屏 488.7 KiB | 达标但无自动门禁时回归无法拦截；`scripts/qa/mobile/verify_mobile.py` 已加"首屏 JS≤250KiB/CSS≤80KiB + 不得请求桌面 chunk + 截图互异"断言，可纳入 CI/QA | frontend-perf + mobile-ui（门禁已由 qa-audit 落地） |
| **P1** | Android 冷启动无启动画面/无 WebView 预热参数（1.2.1 基线） | `gen/android/.../themes.xml @73cf378` 仅 `Theme.MaterialComponents.DayNight.NoActionBar`，无 windowBackground/splash；`MainActivity.kt @73cf378` 无 `installSplashScreen`、无 WebView background/layerType/overScroll/insets 设置 | 冷启动先显示系统白/黑窗口再闪到 WebView 首帧；WebView 首帧前白屏观感差（android-native 任务正在修，需真机验证首帧） | android-native |
| **P1** | Markdown 首次渲染在手机上要解析约 366 KiB worker 入口 | [final-build] `workers/markdown.worker-VE3_whl3.js` 374,701 B（366 KiB；静态闭包=自身）；代码已用 `requestIdleCallback` 预热（HEAD `+page.svelte:4090-4102`），但中低端机解析+编译仍可能数十至百 ms；Shiki 核心另有 81.8 KiB，语言包 10–170 KiB/种按需 | 首次打开长文出现"点了没反应"窗口；移动端应按需求关闭/延迟 Shiki+Mermaid，并测首章渲染耗时（本轮浏览器实测正文就绪 2,000+ 字符/32 块，真机耗时尚待 adb 测量） | mobile-reader（+ frontend-perf 提供 worker 瘦身） |

## 2. P0 明细

### 2.1 P0-1 Android 启动崩溃：注册不存在的 Kotlin 插件

证据（基线 `@73cf378`）：

- `apps/desktop/src-tauri/src/lib.rs:4003-4014`：

```rust
.plugin(
    tauri::plugin::Builder::<tauri::Wry, ()>::new("secrets")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(
                    "com.lingcang.immersivereading",
                    "SecretsPlugin",
                )?;
                secrets::init_secrets_plugin(handle);
            }
```

- Kotlin 目录 `apps/desktop/src-tauri/gen/android/app/src/main/java/com/lingcang/immersivereading/` 只有 `ContentReaderPlugin.kt`、`MainActivity.kt`、`NameSanitizer.kt`、`PendingIntents.kt`（`git ls-tree @73cf378`），**无 `SecretsPlugin.kt`**。
- 1.2.1 发布 APK（`release/ImmersiveReader_1.2.1_android-universal-release.apk`，SHA-256 `F560BC74E620CFCC759BCE0AF095A56909D2A6BCCC5A739928C34BB681E1638C`）解包 `classes.dex` ASCII 扫描：`SecretsPlugin` = 0 次，`ContentReaderPlugin` = 6 次。

影响：`register_android_plugin` 在 setup 阶段同步 `find_class` 失败 → 错误沿 `?` 冒出 → `build().unwrap_or_else(panic!)`（`lib.rs:4310-4316 @73cf378`）→ 启动即崩溃。与用户"OPPO 安装后打开闪退"一致。

真机实证（2026-10-18 19:10，OPPO PLJ110 / Android 17 / SDK 37）：安装 1.2.1 APK 后冷启动 `TotalTime=154ms`，随即 `FATAL EXCEPTION: main ... Caused by: java.lang.ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin`（`WryActivity.getAppClass`），`Force finishing activity` 后进程死亡；系统重试拉起连续 3 次崩溃，`dumpsys meminfo` 为 "No process found"。证据：`docs/audit/2026-10-18/device/1.2.1-crash/`（EVIDENCE.md、logcat-1.2.1.txt、crash-lines.txt、01-after-crash.png，APK SHA-256 F560BC74…A1638C）。

建议（rust-core 已在做）：Android 端删除 secrets 插件注册与 Keystore 后端；`get_secret_status`/`set_deepseek_api_key` 等命令在 Android 返回明确的"不支持"错误；Windows Credential Manager 路径不动。**验收**：真机 `adb logcat` 无 FATAL，冷启动进书架。

当前工作树状态（只读核对，未验证构建）：`run()` 仅注册 `ContentReaderPlugin`（工作树 `lib.rs:4062-4080`），并保留注释说明"register_android_plugin 与 Kotlin @Command 必须一一对应"。**需要在最终 APK 上做真机确认（本报告 §6 未验证项）。**

### 2.2 P0-2 Android setup() 同步阻塞首帧

基线 `@73cf378` `lib.rs:4097-4200`，setup 依次同步执行：

1. `init_android_roots`（必需）；
2. `temporary_content::sweep_app_cache_staging`（扫 app cache）；
3. `secrets::migrate_file_secrets()`（读 Data/Private 迁移）；
4. `storage::app_log`；
5. `control::ControlDb::open_current()` + `recover_interrupted_tasks`（SQLite 打开 + 更新）；
6. `StorageLocations::current_with_library_settings()` → `importer::sweep_staging_dirs(library_root)`、`publish::sweep_publish_residue`、`publish::list_transactions` 并逐个 `recover_transaction`；
7. 再开 control.db：`reconcile_discard_markers` + `reconcile_publish_transaction_index`；
8. 启动后仅 `#[cfg(desktop)]` 的线程/tray 部分被门控。

影响：这些是桌面控制 DB/发布事务/知乎播客任务的恢复逻辑，Android 上既无对应产物（多数情况下目录为空也会走一遍 open/list）又要把 I/O 放在首帧之前；冷启动时间随 Library/control.db 规模增长，直接拖慢"打开就能读"的体感，并可能触发 ANR。

建议（rust-core 已在做）：Android 只保留 storage roots + app_log；暂存清扫移到 `std::thread::spawn` 后台；步骤 5–7 全部 `#[cfg(desktop)]`。**验收**：`adb shell am start -W` 的 `TotalTime` 在真机有明确数字（见 `scripts/qa/mobile/android-device-check.ps1`），并与修复前 APK 对比。

当前工作树状态：Android setup 已缩为 roots+app_log+后台暂存清扫（工作树 `lib.rs:4165-4200`），桌面恢复逻辑在 `#[cfg(desktop)]` 块内（工作树 `lib.rs:4204-4275`）。**待最终 APK 真机验证。**

## 3. P1 明细

### 3.1 P1-1 Markdown 管线双份打包（2.23 MB，worker + 主线程 fallback）

证据（`analyze-build.mjs` 输出，[final-build]、[wip-build]、[baseline-build] 都成立）：

| 项 | 字节 |
|---|---|
| worker 入口 `_app/immutable/workers/markdown.worker-VE3_whl3.js` | 374,701 |
| worker 完整静态+动态闭包 | 2,283,381（22 文件） |
| 主线程 fallback 图根 `_app/immutable/chunks/hOfXeXfW.js` | 375,368 |
| 主线程 fallback 完整闭包（与 worker 图 22 文件一一对应） | ≈2.28 MB |
| worker↔main 判重（22 对，token Dice≈0.999–1.0）合计 | **2,283,240** |

典型重复对（[final-build] 实测对）：`Cux-tB8m.js 637,581 ↔ Ck_RvX-X.js 637,547`（Shiki 语言注册绑定，Dice 1.0）、`DSex4Zst.js 267,142 ↔ U0RLOqka.js 267,125`、`BqvgUolh.js 181,084 ↔ BPQ3VLAy.js 181,080`、`WyGyM1wV.js 174,831 ↔ wDzz0qaB.js 174,827`。
来源：`src/lib/render/markdown.worker.ts:1` 静态 import `./markdown`；`src/routes/+page.svelte:1650 @73cf378`（现 `lib/desktop/DesktopApp.svelte`）在 worker 传输失败时 `await import("$lib/render/markdown")` 回退主线程，于是 Vite 为主线程再打一份完整管线（`getHighlighter`、remark/rehype、KaTeX 动态图）。

影响：APK 的 `lib/arm64-v8a/*.so` 为 13,751,000 B（APK 16,635,119 B 的 82.7%），前端资源内嵌其中；多 2.23 MB 直接抬高安装包与安装后占用。运行期正常走 worker，主线程副本是死重；一旦 fallback 触发，手机端要在 UI 线程再解析 2.2 MB 管线，表现为阅读卡住。

建议：生产构建移除主线程 fallback 图（例如 `if (import.meta.env.DEV)` 才允许 fallback，生产环境 worker 失败显示一次可恢复错误）；或让 fallback 复用 worker 同一 chunk 图（Vite 主/worker 分开发图，需要额外拆包配置）。**验收**：`build/_app/immutable/workers/**` 与主包重复 token 比对 < 5%（可用本报告脚本），APK/`lib*.so` 相应下降。

### 3.2 P1-2 `scan_library` 递归全库 stat + 逐本 manifest/progress + 索引 reconcile

证据 `@73cf378`：

- `library.rs:90-195 collect_manifests`：对每个 entry 调 `entry.metadata()`（第 143 行附近）判断 reparse point；**进入书目录后（`inside_book=true`）仍继续递归**（第 176-190 行只抑制"孤儿目录"告警，没有提前返回），也就是章节子目录、图片目录都会被 stat 一遍（深度上限 3）。
- `library.rs:375-382 load_book_at`：每本书 `read_manifest` + `load_progress`（`progress.rs:52`，每个 `.reading.json` 一次 read）；书多时 2 次文件读/本。
- `library.rs:479-498 reconcile_reader_index`：`ReaderDb::open_current` + `all_books()` + 逐本 `unindex_book/upsert_book`，每次扫描都跑。
- `lib.rs:1936-1942` 命令外层 `settings::load_settings()`（见 P1-3）。
- 触发频率：HEAD `+page.svelte:2002-2045 refreshLibrary` 在启动与每次 resume/任务事件后调用（`+page.svelte:3713-3721`），移动端 `visibilitychange/focus` 都会触发。

影响：书架扫描成本 ∝ 书库全部文件数（不只是书数）。旧审计合成基准（`docs/audit/2026-07-15/BASELINE.md`）：manifest-only 扫描 100 本 p50 29.6 ms、1,000 本 p50 214.7 ms、10,000 本 p50 3.85 s；该基准不含 payload stat，也不含 Android 存储与 reader.db reconcile，真机只会更差。

建议：
1. `collect_manifests` 找到 `manifest.json` 后不再深入 payload（孤儿检测只要求"子树内无 manifest"，可改为发现 manifest 即返回；本书内部无需继续枚举）；
2. 扫描结果按 Library 根目录 mtime/记录数缓存，resume 时不无条件重扫（例如 ≥30 s 节流 + 只在书架可见时刷新）；
3. `reconcile_reader_index` 改为按差异更新（仅对新增/消失 bookId 操作）。

**验收**：真机书架冷启动 ≤ 300 ms（2 本 MD+1 EPUB fixture），1,000 本合成库扫描 p95 ≤ 100 ms（Rust 基准测试）。

### 3.3 P1-3 每个 IPC 重新读 settings.json / 重算 StorageLocations

证据 `@73cf378`：`settings.rs:202-226 load_settings` → `load_compatible_from`（`settings.rs:110-150`，每次 `fs::read_to_string` + JSON 解析 + schema 校验）→ `validate_library_root` + `normalize_production_library_root`，两者各调一次 `StorageLocations::current()`（`storage.rs:162-200`：`AppChannel::detect` + `dirs::data_dir/data_local_dir/document_dir` + `current_exe()`，Android 分支走 `android_base_dirs()`）。
调用它的阅读热路径：`scan_library`（`lib.rs:1939`）、`open_book`（1962）、`get_book_chapter_path`（1990）、`save_book_progress`（2015）、`get_readable_chapter`（2348）。

影响：每翻一章至少 `get_book_chapter_path`+`read_markdown_file`+`save_book_progress` 三条命令（约 2–3 次 settings 读 + 多次路径计算）；滚动保存 ~1 次/秒再叠一次。对单次是亚毫秒级，但 Android 共享存储/低端机上属于纯粹的额外 IO 抖动。

建议：把 `AppSettings` 放进 Tauri managed state，用 `settings.rs` 已有 `SETTINGS_FILE_STAMP`（mtime）做失效判断；`StorageLocations::current()` 在进程内缓存（env/dirs 在进程生命周期内不变，只有 `IMMERSIVE_QA_RUN_ID` 启动时固定）。**验收**：阅读热路径命令在 Rust 基准中不再出现 `settings.json` 读取（可在 `load_settings` 计数）。

### 3.4 P1-4 reader.db 每命令开连接

证据 `@73cf378`：`reader_db.rs:150-210` 每次 `Connection::open` + `create_dir_all(parent)` + schema/version 检查；热路径调用 `book_dir`（`reader_db.rs:367`，`find_book` 每次开书/翻章都走）、`save_locator`（399，EPUB 滚动约 2 s 一次，`lib/epub/ipc.ts:37-40` 注释明确"hot path"）、`save_progress` 路径中的 `find_book`。

建议：`ReaderDb` 单连接放 managed state（`Mutex<Connection>`，懒初始化），或连接池；跑批量写时复用同一连接（`scan_library` 的 reconcile 已经复用了一个）。**验收**：翻章与滚动保存不出现重复 open（Rust 测试/计数断言）。

### 3.5 P1-5 首屏预算（分流已生效，门禁已由 QA harness 落地）

证据（[final-build] 19:02 产物 + [browser-measured] Playwright 3 视口实测）：

| 指标 | [baseline-build]（1.2.1 产物） | [wip-build]（18:49 分流中间态） | [final-build]（1.3.0） |
|---|---|---|---|
| 构建图首屏 JS | 387,272 B（378.2 KiB） | 88,063 B | **88,363 B** |
| 构建图首屏 CSS | 113,113 B（110.5 KiB） | 1,022 B | **1,022 B** |
| 构建图首屏合计 | 500,385 B（488.7 KiB） | 89,085 B | **89,385 B（-82.1%）** |
| 浏览器实测首屏 JS（含移动壳） | — | — | **159,881 B** |
| 浏览器实测首屏 CSS | — | — | **24,092 B** |
| 桌面 UI chunk | 含在首屏 page chunk 302,459 B | lazy `CzYJSkK1.js` 210,217 B | lazy `C8pXMjSg.js` 187,791 B（移动端不请求） |
| 移动壳 chunk | — | `C8N_Qw8f.js` 666 B（stub） | `CheMj8JJ.js` **44,372 B** + `MobileApp.css` 22,470 B |
| 构建 JS 总量 | 8,382,464 B | 8,387,366 B | 8,496,822 B |
| 构建总字节 | 9,672,914 B | 9,678,925 B | **9,832,320 B** |

来源：`analyze-build.mjs`（构建图闭包）+ `scripts/qa/mobile/verify_mobile.py`（浏览器 `PerformanceResourceTiming`，390x844@3 / 360x800@2 / 412x915@2.6 三视口一致）。[baseline-build] 数字来自 18:44–18:48 对旧产物的实测（旧 build 已被重建覆盖，值记录于本节）。注意构建总字节反而上升：移动端新 UI 是新增代码（+44 KB 移动壳、+22 KB CSS、+21 KB ReaderScreen CSS），而 Markdown 管线双份、Mermaid、KaTeX 字体等"死重大户"仍在（见 §3.1）。

风险与门禁：移动端实现（Shelf/ReaderScreen/FocusMode）不得静态 import 桌面组件、`lib/epub/EpubReader.svelte`、`lib/render/markdown`（主线程实例）或 `mermaid`，否则首屏/首次交互会被拉回数百 KB–MB 级。`scripts/qa/mobile/verify_mobile.py` 已内置门禁并通过：
- 首屏 JS ≤ 250 KiB、CSS ≤ 80 KiB（[browser-measured] 159,881 B / 24,092 B，余量充足）；
- 移动视口请求列表中不得出现 `DesktopApp`/`PodcastWorkflow`/`ZhihuWorkflow`/`TrashPanel`/`SettingsPanel`/`window-chrome`（实测 15 个资源，0 泄漏）；
- 每视口 ≥4 张截图且 sha256 互异（实测 3 视口 × 12 张全互异）。


### 3.6 P1-6 1.2.1 Android WebView 首帧/启动画面缺失

证据 `@73cf378`：

- `gen/android/app/src/main/res/values/themes.xml`：仅 `Theme.MaterialComponents.DayNight.NoActionBar`，无 `windowBackground`、无 `core-splashscreen`（`androidx` 依赖也不存在）。
- `MainActivity.kt`：`onCreate` 只有 `enableEdgeToEdge()` + intent 入队；`onWebViewCreate` 只保存引用，无背景色、无 `overScrollMode`、无 `LAYER_TYPE_HARDWARE`、无 `WindowInsets` 注入。
- 影响：冷启动先由系统窗口背景（日/夜模式白/黑）占位 → WebView 首帧前视觉闪白/闪黑；无内容安全区（挖孔/手势条）信息，前端只能靠 `env(safe-area-inset-*)`（Android WebView 支持有限）。

建议（android-native 任务在做）：`core-splashscreen` + 主题背景色（浅 `#faf8f5` / 深 `#1e1e1c`）、`installSplashScreen()`；WebView 创建即设置背景色/禁用 overscroll/硬件层；注入 `--ir-safe-*`。**验收**：`adb exec-out screencap` 冷启动连续帧无纯白首帧，或 `am start -W` 与首帧时间差 ≤ 300 ms（真机记录）。

### 3.7 P1-7 Markdown worker 入口 366 KiB / Shiki 按需加载

证据 [final-build]：worker 入口 `374,701 B`（366 KiB）；Shiki 核心 81,818 B（`workers/chunks/…`）+ 语言包按需 10–170 KiB/种；`render/markdown.ts:58-76` 15 种语言 dynamic import（设计正确）；但 worker 入口本身包含完整 unified/remark/rehype/sanitize 管线（与 P1-1 同源）。前端已用 `requestIdleCallback` 预热 worker（HEAD `+page.svelte:4090-4102`，不阻塞首屏），首次渲染仍要等 worker 脚本解析。[browser-measured] 首章正文就绪 = 2,363–2,495 字符 / 32 块（Playwright 3 视口），但真机解析耗时尚待 `android-device-check.ps1` 测量。

建议（mobile-reader）：移动端首章渲染前预热 worker（首屏空闲时段）；关闭/延迟 Shiki —— 规划是"仅当正文出现代码块且非低端设备才按需加载"；Mermaid 在移动端默认禁用（构建里 Mermaid 全图 2,102,226 B/43 chunk，仅当渲染到 mermaid 代码块时才加载，见 P2-1）；首章渲染耗时纳入真机验收（目标 ≤ 300 ms 可见正文）。

## 4. P2 明细

| # | 问题 | 证据 | 建议 | 归属 |
|---|---|---|---|---|
| P2-1 | Mermaid 动态图巨大（仅当文档含 mermaid 才会加载） | [final-build] 根 `8Nzaj9JP.js` 603,575 B，全闭包 2,102,226 B / 43 文件；[baseline-build] 同规模；`+page.svelte:1694-1700 @73cf378` `import("mermaid")` | 移动端默认禁用 Mermaid（任务已要求）；桌面保留但加"先显示源码再按需渲染"开关；构建侧不优化也可接受（懒加载正确） | mobile-reader / frontend-perf |
| P2-2 | KaTeX 字体/样式体积 | [final-build] 字体 1,072,948 B / 59 文件；`katex.DyJzwzQf.css` 28,933 B 与 `katex.f9WQ10tb.css` 28,933 B 是**同内容两份**（main/worker 各一份）；KaTeX JS 亦双份（主线程 `U0RLOqka.js 267,125 ↔ worker `DIdowJBa.js 267,142`） | 随 P1-1 一起消除主线程副本；字体按 KaTeX 子集裁剪（可选）；移动端仅渲染到数学时才加载 CSS（已 lazy：`+page.svelte:1661-1668 @73cf378`） | frontend-perf |
| P2-3 | 桌面端启动即做更新检查 + 最近文件清理 + 任务机制 | HEAD `+page.svelte:3142` `checkForDesktopUpdate(false)`（内部 `service.ts:86-99` 有 mobile 短路，桌面触发网络/updater 动态 import）；`loadRecentFiles`→`load_recent_files` IPC + `cleanup_recent_files_json`（对每个 recent path `fs::metadata`，文件缺失时再读 state）；任务轮询 15 s（`+page.svelte:826-845`，mobile 已由 capabilities 门控） | 桌面端：更新检查延迟到首帧后 2–5 s / 空闲；recent cleanup 放后台且只在 win 首次启动跑；均为桌面体验优化，非移动阻塞 | frontend-perf / rust-core |
| P2-4 | 旧页面文件监视每 2 s 一次 IPC（桌面保留，移动新壳不得继承） | HEAD `+page.svelte:4114-4200` `setInterval(get_file_mtime)`，hidden 时跳过、失焦降频；mobile 端旧页面同样挂了这个定时器 | 新 `MobileApp` 不引入常驻定时器（任务要求）；桌面端可保留 | mobile-ui |
| P2-5 | 书卡列表无窗口化/分批渲染（旧 Bookshelf） | HEAD `lib/components/Bookshelf.svelte:452` `{#each filteredBooks as book}`；`+page.svelte:7015` 传入全量 | 移动书架分批/窗口化（任务已要求）；桌面书架 1,000 本时 DOM 节点数过大 | mobile-ui / frontend-perf |
| P2-6 | 首次打开书籍的 IPC 串行数偏多 | HEAD `+page.svelte` 打开 MD：`open_book` + `get_book_chapter_path` + `read_markdown_file`（3 IPC）；EPUB：`open_book` + `get_reader_locator` + `list_bookmarks` + `get_readable_chapter`（4 IPC） | MD 移动端直接复用 `get_readable_chapter`（`lib.rs:2343 @73cf378`，已同时支持 MD/EPUB 且返回 prev/next + resource_dir），翻章 1 IPC 完成；EPUB 的 locator/bookmarks 可与章节首屏并行（Promise.all）而非串行 | mobile-reader |
| P2-7 | 大文件读取上限 32 MiB（`MAX_MARKDOWN_FILE_BYTES`）与移动端 >8 MiB 分块要求之间的边界需实测 | `lib.rs:620-642 @73cf378` `read_markdown_file` 读入内存后 `decode_markdown_bytes` 生成 String（≈2× 峰值为 UTF-8 时再翻倍）；`get_readable_chapter` 同 | 移动端已规划 >1 MiB 分块挂载、>8 MiB 分块、>64 MiB 拒绝；建议 Rust 侧提供分段读取命令或流式返回，避免整章 String 在 JS 堆再复制一份 | rust-core + mobile-reader |
| P2-8 | Android APK 内前端资源内嵌在 `lib*.so` 且未压缩；assets/ 无前端产物 | APK ZIP：`lib/arm64-v8a/libimmersive_reader_lib.so` 13,751,000 B（stored）、`classes.dex` 1,999,236 B（deflate 后 947,314 B）、`assets/` 仅 `tauri.conf.json` + baseline profiles | 这是 Tauri Android 的形态，正常；但意味着构建 JS 体积直接等于 so 体积增量 → P1-1 的 2.23 MB 值得修。android-native 的 `isShrinkResources`/R8 只影响 dex/res，不影响内嵌前端 | android-native（记录）+ frontend-perf |

## 5. 桌面端启动 / 内存证据

- 已安装生产 EXE：`%LOCALAPPDATA%\Programs\ImmersiveReader\immersive-reader.exe`，24,027,648 B，时间戳 2026-09-26 23:46:54，SHA-256 `9339375F3358086EFCC3692647103222D3134E4C774AEF33866627A6F0C6355C`（与 1.2.1 QA 报告一致）。
- 桌面首屏（旧产物）488.7 KiB JS+CSS；[final-build] 构建图首屏 89,385 B（§3.5），桌面壳 lazy `C8pXMjSg.js` 187,791 B + `DesktopApp.css` 73,047 B，桌面端首次渲染实际支出 ≈ 首屏 + lazy 桌面壳（~350 KiB JS/CSS 量级），比 1.2.1 基线的首屏 488.7 KiB 更优；[browser-measured] 移动视口不会请求桌面壳。
- [final-build] 桌面壳 `C8pXMjSg.js` 187,791 B + `DesktopApp.css` 73,047 B 为 lazy；桌面首屏 = 89,385 B dispatcher + lazy 桌面壳（≈373 KiB JS/CSS 量级），比 1.2.1 基线的首屏 488.7 KiB 更省"打开即用"成本。
- **未实测**：本轮审计没有启动生产应用做 WorkingSet/PSS 采样（避免触碰真实用户 Library；且桌面端 1.3.0 生产包尚未 ship:local）。桌面运行内存/启动耗时留待 Lead 统一构建后由本 QA 脚本补测；可复现命令建议：

```powershell
$p = Start-Process -FilePath "$env:LOCALAPPDATA\Programs\ImmersiveReader\immersive-reader.exe" -PassThru
Start-Sleep -Seconds 8
Get-Process -Id $p.Id | Select-Object Id,WorkingSet64,PrivateMemorySize64,CPU,StartTime
```

- 旧审计（`docs/audit/2026-07-15/BASELINE.md`，合成数据）可作对照：16 MiB 全书读取整读 3.5 ms/峰 16.8 MB vs 流式 74 KB 峰；本次 Rust 阅读路径改为逐文件读 + 上限（`lib.rs:620-642/get_readable_chapter`），大章节仍是"整章进内存"，见 P2-7。

## 6. 未测量 / 限制（诚实标注）

- 移动端真实 WebView 性能已补测（1.3.0 arm64，OPPO PLJ110/Android 17）：冷启动 TotalTime 141/141/141 ms（首装）与 154/153 ms（arm64 重装后，median 153.5）；阅读中 janky 0.00%（184 帧，p50 6 ms/p90 8 ms/p99 9 ms）；PSS 阅读中 98,631 KB、启动后 100,771–113,228 KB；logcat FATAL/ANR 0。详见 `QA_REPORT.md` §5.2 与 `device/EVIDENCE.md`。仍未测：WebView JS-ready 毫秒数、文末最后一行底部安全区、多机型/旧 Android 矩阵。
- 浏览器验收已对 [final-build] 全量执行：`scripts/qa/mobile/verify_mobile.py` 3 视口 **71/71 PASS**，36 张互异截图 + `mobile-smoke-report.json` 在 `docs/audit/2026-10-18/screens/`（截图哈希/字节数在报告中）。这只证明 WebView 逻辑/资源预算，**不能替代**真机首帧/PSS/帧率。
- 桌面端运行内存/启动**未测**（原因见 §5）。
- `analyze-build.mjs` 的重复度判据为 token 多重集 Dice（≥0.95 视为重复），不是字节级相等：Chunk 名/包装代码差异会让字节不同（例如 `Cux-tB8m` 637,581 B vs `Ck_RvX-X` 637,547 B，Dice=1.0），所以报"重复 2,283,240 B"是保守的 token 等价量级。
- 未测量生产 Library 规模；P1-2 的影响推断基于旧审计合成基准 + 代码路径，不是本轮真机数字。
- `docs/audit/2026-07-15/BASELINE.md` 的 p50/p95 均为合成临时数据，不能当作真实用户库结论；本报告只引用其量级作对照。

## 7. 验收门槛建议（写入 Lead 的最终 QA）

| 指标 | 目标 | 测量方式 |
|---|---|---|
| Android 冷启动 | 无 FATAL；`am start -W` TotalTime ≤ 1,500 ms（实测 141–154 ms，达标） | `android-device-check.ps1` / `device/EVIDENCE.md` |
| 移动首屏 JS/CSS | JS ≤ 250 KiB / CSS ≤ 80 KiB（浏览器实测；当前 159,881 B / 24,092 B） | `verify_mobile.py` resource timing |
| 桌面 chunk 泄漏到移动 | 0（移动视口不请求 DesktopApp/Podcast/Zhihu/SettingsPanel chunk 或 CSS） | `verify_mobile.py` request 断言 |
| Markdown 首章可见 | ≤ 300 ms（真机，2 本 MD fixture） | 真机 logcat timing / Playwright |
| 书架扫描 | 2 本 MD+1 EPUB 冷扫描 ≤ 300 ms；1,000 本 p95 ≤ 100 ms（Rust 基准） | Rust 测试 + 真机 |
| worker/main 重复 | ≤ 5%（token 判重） | `analyze-build.mjs` |
| Android PSS（阅读中） | ≤ 350 MB（实测 98.6 MB，达标） | `dumpsys meminfo` |
| 滚动 jank | janky 帧 ≤ 5%（实测阅读滚动 0.00%，达标） | `dumpsys gfxinfo` |
