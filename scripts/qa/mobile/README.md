# 移动端 Playwright 冒烟 harness（task-7）

用本机已装的 Python Playwright（`G:\python\python.exe`，1.62.0）+ 系统 Chrome
channel 跑 **apps/desktop/build** 的移动端冒烟/截图；不需要 Tauri shell、不联网、
不安装任何新依赖。mock 注入 `window.__TAURI_INTERNALS__`，覆盖移动端用到的
Rust 命令，因此可以在真实构建产物上验证 UI 行为。

> 本目录属于 QA 专用，不参与产品构建；mock/fixture 全部为合成数据。

## 文件

| 文件 | 作用 |
|---|---|
| `verify_mobile.py` | 主 harness：静态服务 build、注入 mock、3 视口流程、断言、截图、JSON 报告 |
| `mobile_mock.js` | `window.__TAURI_INTERNALS__` mock（invoke 路由 + event/convertFileSrc/transformCallback） |
| `fixture/index.html` | 最小移动壳 fixture，用于在真实 build 未就绪时自检 harness 逻辑（`--fixture`） |
| `android-device-check.ps1` | 真机 adb 验证脚本（安装/卸载旧包/冷启动/logcat/PSS/gfxinfo/截图/打开 fixture 文档；支持无线调试 `-Serial host:port`） |
| `make_device_fixtures.py` | 生成 `docs/audit/2026-10-18/fixtures/` 下合成 md/epub 小文件（真机用） |

## 快速开始

```powershell
# 真实构建产物（需要先有 build；harness 自己绝不运行 vite build）
G:\python\python.exe scripts\qa\mobile\verify_mobile.py

# 只跑一个视口 / 显示浏览器窗口
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --viewport 390x844@3 --headed

# 复用一个已经在跑的静态服务器
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --base-url http://127.0.0.1:4179

# 额外跑真机 CSS 视口（OPPO Find X9：1256x2760@density560 => 359x789 CSS, DPR 3.5）
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --extra-device-viewport

# harness/mock 自检（不依赖 build）
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --fixture
```

若环境里 `PLAYWRIGHT_BROWSERS_PATH` 没配（本机为 `G:\build_cache\playwright-browsers`），
Python Playwright 使用系统 Chrome channel 即可；找不到 Chrome 时才回退到
bundled chromium。

## 断言清单

每个视口（light）都会跑：

- 无 unhandled pageerror、无 console error（图片/资源 404 类浏览器预演噪声会记录但默认不判失败，`--strict-console` 可提升为失败）。
- 书架：`shelf-continue` 可见；3 本 mock 书可发现（继续阅读的书可能只以 hero 卡呈现）；无横向溢出；`--ir-safe-top` 安全区变量存在。
- Markdown：点击书卡/续读卡后必须等到 `[data-testid="reader-screen"]` **且** `reader-content` 文本非空/块级元素 ≥8（点击无效会在这里失败，不会截出"假阅读器"图），再校验章节 marker。
- 阅读器 chrome：`reader-back` 触控目标 ≥44px；内容已挂载且（能定位到滚动容器时）可滚动。
- 目录：`reader-toc` → 目录面板（`toc-sheet`/`toc-item-*`）截图，点击 `toc-item-1` 后正文切到第 2 章 marker。
- 阅读设置面板：`reader-settings` 打开面板并截图。
- 专注模式：存在 `focus-hud` 触发器时点击并断言 `focus-hud`；当前 build 尚未暴露触发器 → 计为 FAIL（见"当前状态"）。
- EPUB：打开 epub 书 → 正文 marker；目录面板截图；目录切到第二 spine。
- 设置：`shelf-settings` → `settings-screen`，且不含 知乎/播客/连读；截图。
- 导入：`shelf-import` → `import-sheet`；截图。
- 性能：移动视口不得请求桌面壳资源（`DesktopApp*`/`PodcastWorkflow*`/`ZhihuWorkflow*`/`TrashPanel*`/`SettingsPanel*`/`window-chrome`）；首屏 JS ≤ `--max-boot-js-bytes`（默认 250 KiB）、CSS ≤ `--max-boot-css-bytes`（默认 80 KiB），预算来自 `PerformanceResourceTiming`。
- 截图证据：每个视口 ≥4 张且 **sha256 互不相同**（防止"点击没进阅读器却截图通过"回归）。

dark 视口（`color_scheme=dark`）：书架/阅读器背景亮度 <0.35，各截一张。

## 视口与截图

| 视口 | DPR | 说明 |
|---|---|---|
| `390x844@3` | 3.0 | OPPO Find X9 CSS 尺寸 |
| `360x800@2` | 2.0 | 小屏 Android 基线 |
| `412x915@2.6` | 2.6 | 大屏 Android |

截图命名 `<viewport>-<序号>-<状态>.png`，默认写入
`docs/audit/2026-10-18/screens/`；JSON 报告为同目录 `mobile-smoke-report.json`
（含每条检查、console error、资源字节、截图 sha256/字节数、每视口截图清单）。

无法运行/尚未就绪时 harness 会明确 SKIP（exit 2），**不会伪造通过**：
- build 目录不存在 → 提示先 `npm.cmd --prefix .\apps\desktop run build`；
- build 仍是 `MobileApp` stub（`[data-testid="mobile-stub"]`，没有书架 testid）→ exit 2。

## mock 路由（`mobile_mock.js`）

- 平台/设置：`get_platform_capabilities`（android caps，snake+camel）、`get_app_settings`、`get_storage_locations`、`get_storage_usage`、`clear_safe_cache`、`set_volume_key_capture`、`plugin:app|version`
- 书架：`scan_library`（`?state=loading|empty|error` 可注入状态）、`open_book`、`get_book_chapter_path`、`read_markdown_file`、`list_trash`、`remove_book`、`delete_book`、`load_recent_files`
- 阅读：`get_readable_chapter`（MD + EPUB XHTML）、`get_reader_locator`/`save_reader_locator`、`list_bookmarks`/`add_bookmark`/`remove_bookmark`、`search_book`、`load_reading_state`/`save_reading_state`、`save_book_progress`
- 导入/外部打开：`begin_import`/`get_import_status`/`cancel_import`、`take_pending_open_uris`（`?mode=pending-open`）
- 事件/资源：`plugin:event|listen|unlisten|emit`、`convertFileSrc`、`transformCallback`/`unregisterCallback`
- 桌面专属命令（`get_acquisition_snapshot`/`list_temporary_content`/`get_secret_status`/`start_reader_session`/`load_reader_preferences`）会**抛错**，用于在移动端误调用时暴露回归。

fixture 数据：2 本 Markdown（`md-shan-yue` 3 章 / `md-du-shu-bi-ji` 2 章）+ 1 本 EPUB
（`epub-xiao-wang-zi`，3 spine + 多级 nav + 内链/图片/脚注）。正文 marker：
`IR-QA-MD-<chapterId>` / `IR-QA-EPUB-<chapterId>`。

## data-testid 契约（UI 由 mobile-shell/mobile-reader 提供）

`shelf-continue`、`shelf-book-<bookId>`、`shelf-import`、`shelf-settings`、
`reader-screen`、`reader-toolbar`、`reader-content`、`reader-back`、`reader-toc`、
`reader-settings`、`reader-progress`、`toc-sheet`、`toc-item-<i>`、`focus-hud`、
`import-sheet`、`settings-screen`。

本 harness 允许的等价 fallback：阅读正文还接受 `reader-body`/`reader-scroll`/
`.reader-content`/`.markdown-body`/`.article`/`article`；阅读设置面板还接受
`reader-settings-panel`/`reader-settings-sheet`/`[role=dialog]`。如果 UI 改
testid，请同步本文件与 `verify_mobile.py` 顶部的常量。

## 当前状态（2026-10-18）

- `--fixture` 自检：**68/68 PASS**，3 视口 × 12 张互不相同截图（harness 逻辑已验证）。
- 最终 build（1.3.0，2026-10-18 19:02 产物）全量冒烟（3 视口 + 真机 CSS 视口
  359x789@3.5）：**94/94 PASS**，4 视口 × 12 张 = 48 张互异截图。书架/MD/EPUB/
  目录切章/阅读设置/专注 HUD（经阅读设置「专注模式」开关进入）/应用设置/导入/深色/
  安全区/首屏预算（JS≈159,881 B、CSS≈24,092 B）/无桌面 chunk 泄漏全部通过。
- 该探针截图归档在 `docs/audit/2026-10-18/screens/_probe-2026-10-18-1908/`
  （最终 build 通知后会重跑并把正式截图写入 `screens/`）。
- 真机证据见 `scripts/qa/mobile/android-device-check.ps1`；无设备时脚本输出
  SKIP（exit 2）并把 `device-summary.json` 写入 `docs/audit/2026-10-18/device/`。

## 真机执行（android-device-check.ps1）

```powershell
# 无线调试设备：-Serial host:port；APK 签名与旧包不同时用 -UninstallFirst
pwsh -NoProfile -File .\scripts\qa\mobile\android-device-check.ps1 `
  -Serial 192.168.2.178:41031 `
  -ApkPath .\output\mobile\<final>.apk -UninstallFirst
```

脚本先校验设备/包，再 `uninstall`（可选）→ `install -r -d` → `pm path`/versionName →
冷启动 `am start -W` ×3 → logcat FATAL/ANR 扫描 → `dumpsys meminfo` TOTAL PSS →
`dumpsys gfxinfo` 滑动 + janky/p50/p90/p99 + framestats → `exec-out screencap` →
push fixtures 到 `/sdcard/Download/IR-QA/` 并尝试 file:// VIEW intent 打开
md/epub。未连接设备时输出 SKIP 并 exit 2；任何解析失败标 UNVERIFIED，绝不补数。
输出目录默认 `docs/audit/2026-10-18/device/`（device-summary.json/device-report.md +
原始 logcat/meminfo/gfxinfo/截图）。

## 注意

- harness 只用本机已装 Playwright/Chrome，**不要** `pip install`/`npm install` 第二份。
- 不要修改 `src/**` 来适配测试；缺 testid 由 UI owner 补。
- 真实 build 的窗口/视口外框不是 Tauri WebView：Android WebView 的真机首帧、PSS、
  帧率由 `android-device-check.ps1` 在真机测量，两者互补。


