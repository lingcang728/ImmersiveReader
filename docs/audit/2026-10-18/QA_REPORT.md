# 1.3.0 移动端重做 + 1.2.1 Android 启动崩溃修复 QA 报告

- 日期：2026-10-18（Asia/Shanghai），QA owner：qa-audit（task-7）
- 范围：Android 端重做为本地 Markdown/EPUB 阅读器（全 UI 重设计、移除知乎/播客/连读）、1.2.1 Android 启动崩溃根因修复、桌面入口分流/懒加载、版本 1.3.0 构建与真机验证
- 基线：`73cf378`（1.2.1 交付头）→ 本轮工作树交付（Lead 集成提交后补）
- 相关报告：`docs/audit/2026-10-18/PERF-AUDIT.md`（性能审计，含 P0/P1/P2 与数据来源）

## 1. 结论摘要

| 验收项 | 结果 | 证据 |
|---|---|---|
| 1.2.1 Android 启动崩溃根因（ClassNotFoundException: SecretsPlugin） | ✅ 真机复现并确认 | §5.1 + `device/1.2.1-crash/EVIDENCE.md` |
| 1.3.0 Android 冷启动不崩溃（真机 OPPO PLJ110/Android 17） | ✅ FATAL/ANR=0；冷启动 TotalTime 141–154 ms（median 141/153.5），启动正常 | §5.2 `device/EVIDENCE.md` |
| 真机阅读性能/内存 | ✅ 阅读滚动 janky **0.00%**（184 帧，p50 6ms/p90 8ms/p99 9ms）；PSS 阅读中 **98.6 MB** | §5.2 |
| 真机返回键（书架打开阅读器 → BACK） | ✅ 回书架且进程存活 | §5.2 `12-after-back-from-shelf-reader.png` |
| 真机深色/浅色主题 | ✅ 实时切换，各一张阅读器截图 | §5.2 `06/07-*.png` |
| 移动端 UI 冒烟（3 视口 + 真机 CSS 视口，浏览器 WebView 等价） | ✅ 94/94 PASS，48 张互异截图 | §3.2 `screens/mobile-smoke-report.json` |
| Android APK | ✅ arm64 16,033,256 B（真机验证）+ universal 57,467,499 B（4 ABI，zipalign 通过） | §2.3 |
| 移动端首屏 JS/CSS 预算（≤250 KiB / ≤80 KiB） | ✅ 159,881 B / 24,092 B | §3.2、§4 |
| 移动视口不加载桌面 chunk | ✅ 0 泄漏（15 个资源） | §3.2 |
| 深色主题 / 安全区 / 44px 触控 / 无横向溢出 | ✅ 全部通过 | §3.2 |
| Markdown 阅读（打开/翻章/目录/阅读设置/脚注/表格/代码块） | ✅ 通过：正文 2,322–2,495 字符、32 块、可滚动 6,869px | §3.2 |
| EPUB 阅读（spine/目录/内链/资源） | ✅ 通过 | §3.2 |
| 专注模式 HUD（聚光灯+底部 HUD） | ✅ 通过（阅读设置「专注模式」开关进入，`[data-testid="focus-hud"]` 可见） | §3.2 |
| 桌面端启动/内存优化（入口分流/懒加载） | ✅ 构建图首屏 89,385 B（1.2.1 基线 488.7 KiB，-82.1%）；桌面运行内存 ⏳ 未实测 | §4 |
| Windows 生产安装（ship:local）+ EXE 哈希 | ✅ 安装器 8,196,420 B SHA-256 `C89631E1…B56DB4`；EXE 19:43:36 SHA-256 `8328BD1C…63F103` | §2.1 |
| `verify.ps1` 全绿 | ✅ exit 0 `all checks passed`：Rust 230 / 前端 263 / 契约 14+60 fixtures / zhihu 88 / podcast 97 / Pester 14；仅 report-only 警告 | §3.1 |

## 2. 构建产物

### 2.1 Windows（✅ 2026-10-18 19:43 ship:local 完成）

| 项 | 值 |
|---|---|
| 版本 | 1.3.0 |
| 安装器 | `output/desktop/ImmersiveReader_1.3.0_x64-setup.exe`（NSIS，8,196,420 B） |
| 安装器 SHA-256 | `C89631E13B0FEE83B4F7CDD947D994CB7F85F31AC1A0EA9C8CBA15C680B56DB4` |
| 已安装 EXE | `C:\Users\15pro\AppData\Local\Programs\ImmersiveReader\immersive-reader.exe` |
| EXE 时间戳 | 2026-10-18 19:43:36 |
| EXE SHA-256 | `8328BD1C2F752033BA3DE59C80F5F98A67BB27DDC02E303216C8FC786963F103` |
| 备注 | 桌面外观/交互零变化（task-1 验收），桌面壳 lazy `C8pXMjSg.js` 187,791 B；`updater` 签名与 `latest.json` 已生成；`.md` UserChoice 仍指向 VSCode，需用户在 Windows「默认应用」确认（不伪造受保护哈希） |

### 2.2 Android 前端产物（已确认）

| 项 | 值 |
|---|---|
| 版本 | 1.3.0（`apps/desktop/package.json` / `tauri.conf.json` / `Cargo.toml` 均为 1.3.0） |
| 构建时间 | 2026-10-18 19:02（`apps/desktop/build/index.html` mtime；`_app/version.json` hash `1792321319455`） |
| 首屏资源（构建图闭包） | JS 88,363 B + CSS 1,022 B = **89,385 B**（1.2.1 基线 500,385 B，**-82.1%**） |
| 移动壳 chunk / CSS | `CheMj8JJ.js` 44,372 B + `MobileApp.css` 22,470 B（+ 阅读器 `ReaderScreen.css` 21,391 B 懒加载） |
| 桌面壳 | lazy `C8pXMjSg.js` 187,791 B + `DesktopApp.css` 73,047 B（移动端不请求） |
| 构建总字节 | 9,832,320 B（JS 8,496,822 B；worker bundle 2,283,381 B） |
| 构建分析 | `docs/audit/2026-10-18/build-analysis.json`（`scripts/analyze-build.mjs` 生成，只读解析，不触发构建） |

### 2.3 Android APK（✅ 已构建并真机安装验证）

| 项 | 值 |
|---|---|
| 产物路径（主） | `output/mobile/ImmersiveReader_1.3.0_android-arm64-release.apk`（真机验证用） |
| 产物路径（兼容） | `output/mobile/ImmersiveReader_1.3.0_android-universal-release.apk`（arm64-v8a + armeabi-v7a + x86 + x86_64，供其他机型/模拟器） |
| 大小 | **16,033,256 B**（arm64） / **57,467,499 B**（universal） |
| SHA-256 | arm64 **`D52D358C3913390AC511B33AB9001E4CD1E3275CADEE9EE2BD746DBD0718DED6`**；universal **`3066699C6BF116E3E00410A1E4B2838186900A3C7E8950DC77EE9933C9016224`** |
| 包名 | `com.lingcang.immersivereading` |
| versionCode / versionName | **1003000 / 1.3.0**（真机 `dumpsys package` 复核） |
| ABI | arm64 包：**arm64-v8a**（真机 `primaryCpuAbi` 复核）；universal 包含 4 个 ABI（`arm64-v8a` 13,760,744 B、`armeabi-v7a` 10,746,560 B、`x86` 15,596,816 B、`x86_64` 15,086,704 B） |
| zipalign | `zipalign -c -p 4` exit 0（两个包） |
| minSdk / targetSdk | **26 / 36**（真机复核） |
| 签名 | 新 ir-release 自签名 keystore（`keystore.properties` 已 gitignore，未提交密钥） |
| 安装 | `adb install -r -d` → Success（旧 1.2.1 已卸载；arm64 重装 `install -r` 保留数据） |

## 3. 自动化测试与浏览器验收

### 3.1 测试套件（来源：Lead `scripts\verify.ps1` 全量运行，2026-10-18 19:32，exit 0 `all checks passed`）

| 套件 | 结果 |
|---|---|
| 契约 fixtures parity（schema + TS，60 fixtures） | 通过；契约 TS 测试 14 passed |
| 前端 `svelte-check` | **0 errors / 0 warnings** |
| 前端 vitest | **27 files / 263 tests passed** |
| Rust `cargo test` | **230 passed / 0 failed / 0 ignored** |
| Rust `cargo check` / `clippy -D warnings` / `fmt --check` | 通过 |
| zhihu-packer 测试 / TS build / reader compile | **88 passed**；build 通过 |
| podcast ruff / pytest / quick_validate | 通过；**97 passed** |
| Pester（scripts 安全与运行时包） | **14 passed / 0 failed** |
| managed runtime manifest | 1849 critical entries 通过 |
| 版本字段对齐 | 1.3.0（package/tauri/Cargo 一致） |
| report-only 警告（不阻断） | `docs/release/1.3.0/runtime-parts.json` 未生成（需 6.9GB runtime 分包，本轮跳过）；cargo audit 报告 9 条依赖告警（quick-xml/zip/rustls 等既有依赖，升级属独立安全任务）；npm audit 镜像端点不支持；PSScriptAnalyzer 未安装 |

### 3.2 移动端 Playwright 冒烟（本 QA 执行）

工具：`scripts/qa/mobile/verify_mobile.py`（本机 Python Playwright 1.62.0 + 系统 Chrome，注入 `window.__TAURI_INTERNALS__` mock；mock 只含合成 fixture，不接触真实用户数据）。

```
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --extra-device-viewport
→ summary: pass=94 fail=0 skip=0
```

| 视口 | DPR | 截图数 | 说明 |
|---|---|---|---|
| 390x844 | 3.0 | 12 | 大屏 Android / Find X9 CSS 量级 |
| 360x800 | 2.0 | 12 | 小屏 Android 基线 |
| 412x915 | 2.6 | 12 | 大屏 Android |
| **359x789** | **3.5** | 12 | **OPPO PLJ110 真机 CSS 视口**（1256x2760 @ density 560） |

每视口 12 张截图：`01-shelf`、`02-reader-md`、`03-reader-md-toc`、`04-reader-md-chapter2`、`05-reader-md-settings`、`06-reader-md-focus`、`07-reader-epub`、`08-reader-epub-toc`、`09-settings`、`10-import-sheet`、`11-dark-shelf`、`12-dark-reader`；共 48 张，sha256 全部互异（报告逐张记录）。

断言覆盖：
- 无 unhandled pageerror / console error（0）；无桌面 chunk 泄漏（15 个资源，0 命中 DesktopApp/Podcast/Zhihu/Trash/SettingsPanel）。
- 书架：继续阅读 hero、3 本书可发现、无横向溢出、`--ir-safe-top` 安全区变量。
- Markdown：点击后必须等到 `[data-testid="reader-screen"]` 且正文非空才截图（防"假阅读器"回归）；首章正文 2,322–2,495 字符 / 32 块 / 可滚 6,869px；目录切章命中第 2 章 marker；阅读设置面板；专注 HUD；44px 返回目标。
- EPUB：spine 首章渲染、目录抽屉、目录跳转第二 spine、返回书架。
- 设置：移动设置页无 知乎/播客/连读 字样；导入 sheet 打开。
- 深色：书架与阅读器背景 `rgb(30,30,28)`（prefers-color-scheme: dark）。
- 首屏预算：JS 159,881 B / CSS 24,092 B（门禁 250 KiB / 80 KiB）。

harness 自检（`--fixture`，不依赖真实 build）：68/68 PASS；真实 build 探针（18:58 中间产物）：71/71 PASS。证据：`docs/audit/2026-10-18/screens/mobile-smoke-report.json`、`scripts/qa/mobile/README.md`。

## 4. 性能审计结论（详见 PERF-AUDIT.md）

- **P0-1** 1.2.1 Android 启动崩溃：`SecretsPlugin` 注册了不存在的 Kotlin 类 → 真机 FATAL（§5.1）。已由 rust-core 修复（Android 只注册存在的 `ContentReaderPlugin`）。
- **P0-2** Android `setup()` 同步执行桌面级恢复/清扫：已 `#[cfg(desktop)]` 门控 + 暂存清扫移后台线程；真机冷启动耗时见 §5.2。
- **P1（仍开放，归属 frontend-perf）** Markdown 管线双份打包：worker 图 2,283,381 B 与主线程 fallback 图 22 对 chunk 重复，合计 **2,283,240 B**；APK 的前端资源内嵌在 `lib*.so`（1.2.1 APK 中 lib 13.75/16.6 MB），建议生产构建将主线程 fallback 改为 DEV-only 或共享 chunk 图。
- **P1（仍开放，归属 rust-core）** `scan_library` 递归 stat 全库 payload + 逐本 manifest/progress；每 IPC 重读 `settings.json`；`reader.db` 每命令重开连接。详见 PERF-AUDIT §3.2–3.4。
- 首屏与移动端预算：本轮分流后达标（89,385 B 构建图 / JS 159,881 B、CSS 24,092 B 浏览器实测），并已写入 harness 门禁（防回归）。

## 5. 真机验证（OPPO Find X9 / PLJ110，Android 17 / SDK 37，无线调试）

环境：`adb devices -l` → `192.168.2.178:41031 device product:PLJ110 model:PLJ110 device:OP5E17L1`；物理 1256×2760、density 560；CSS 视口 359×789、DPR 3.5。
脚本：`scripts/qa/mobile/android-device-check.ps1`（无设备时诚实 SKIP exit 2；本报告所有数字均来自真实 adb 输出）。

### 5.1 1.2.1 崩溃复现（✅ PASS：根因确认）

- 安装 `release/ImmersiveReader_1.2.1_android-universal-release.apk`（16,635,119 B，SHA-256 `F560BC74E620CFCC759BCE0AF095A56909D2A6BCCC5A739928C34BB681E1638C`）→ `Success`，`versionName=1.2.1`，`arm64-v8a`。
- 冷启动 `am start -W`：`Status: ok` / `LaunchState: COLD` / **TotalTime 154 ms / WaitTime 161 ms**（首帧前崩溃）。
- logcat：`FATAL EXCEPTION: main` → `RuntimeException: InvocationTargetException` → **`Caused by: java.lang.ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin`**（`WryActivity.getAppClass`）→ `Force finishing activity` → 进程死亡；系统重试拉起连续 3 次（PID 26759/27028/27201）均同一崩溃；`pidof` 为空、`dumpsys meminfo` = "No process found"。
- 证据：`docs/audit/2026-10-18/device/1.2.1-crash/`（EVIDENCE.md、install.txt、cold-start.txt、过滤后的 crash-lines.txt、01-after-crash.png 1,719,040 B）。原始 logcat 含设备系统日志，已移除（隐私），仅保留与该崩溃相关的过滤摘录。
- 旧包已 `adb uninstall`（Success），`pm path` 为空——1.3.0 为**全新安装**，未验证 1.2.1→1.3.0 升级路径。

### 5.2 1.3.0 真机验证（✅ 已执行；证据 `docs/audit/2026-10-18/device/EVIDENCE.md`）

| 检查 | 结果 |
|---|---|
| `adb install -r -d` arm64 稳定路径 | ✅ Success；`versionName=1.3.0` / `versionCode=1003000` / `arm64-v8a` / `targetSdk=36` |
| 冷启动 `am start -W` ×3 | ✅ Status ok / COLD；首装 152/141/141 ms（median **141 ms**）；arm64 重装后 154/153 ms（median **153.5 ms**，另一轮 TotalTime=0 因 force-stop 竞态判无效） |
| logcat FATAL EXCEPTION / ANR | ✅ **0 / 0**（3 次冷启动 + 阅读/外部打开全程；原始 logcat 含设备日志已删除，仅保留命中计数与过滤摘录） |
| `dumpsys meminfo` TOTAL PSS | 启动后 **113,228 KB**（首装）/ **100,771 KB**（arm64 重装）；阅读中 **98,631 KB**（RSS 285,432 KB） |
| `dumpsys gfxinfo` 阅读滚动 | ✅ **184 帧 / janky 0（0.00%）**，p50 **6 ms** / p90 **8 ms** / p99 **9 ms**，Missed Vsync 0；framestats 已保存 |
| 截图 | ✅ 启动书架、MD 阅读（深/浅）、滚动后、EPUB 阅读器、BACK 回书架（`device/*.png`，共 10 张） |
| 外部打开（content:// + `--grant-read-uri-permission`，显式组件） | ✅ MD/EPUB 均可冷启动直接进入阅读器；`file://` 在 Android 17 被系统 Resolver 拦截（预期，改用 content://） |
| 系统返回键 | ✅ 从书架打开 MD 阅读器后 BACK 回书架、进程存活；⚠️ 从**外部打开**的 EPUB 阅读器 BACK 回桌面（外部打开任务栈语义，待产品确认） |
| 主题/安全区 | ✅ 深/浅主题实时切换；顶部正文在状态栏下方；⚠️ 文末最后一行与底部手势条关系未验证 |
| 首帧白屏/启动画面 | ✅ 2–4 s 后截图显示书架已可交互；无崩溃/白屏卡死；JS-ready 精确毫秒未测 |

真机 fixtures（合成、非真实用户数据，`docs/audit/2026-10-18/fixtures/`）：
- `sample-notes.md`（6,317 B，标题/代码块/表格/脚注/长文 marker）
- `sample-library/`（manifest + 2 章，供文件夹导入）
- `sample-book-epub.epub`（3,358 B，EPUB3：mimetype stored、2 章、多级 nav、内联图、内链；SHA-256 `BBE366FB1476EFB1DDD4610343742D73CFC06EF7A2BDDC00E4425420A2389C4D`）

### 5.3 真机观察（非阻塞，建议跟进）

1. **外部打开同一 MD 会累积重复书目**：每次 content:// VIEW 新建 `mobile-import-<uuid>` 目录（本次 3 份合成 sample-notes），书架出现多条近似记录、"N 个项目无法读取"告警从 2 递增到 5（`.incoming` 为空、重复目录内 manifest+章节齐全）。建议评估外部打开去重与 issue 归类（mobile-shell / rust-core）。
2. **外部打开 EPUB 的 BACK 行为**：返回桌面而非书架；从书架进入的阅读器 BACK 正常回书架。若产品要求统一返回栈，需要处理外部打开路径。
3. **用户手动书库数据**：设备 app 库内存在用户真实 EPUB（一本真实 EPUB（用户手动书库数据，书名已隐去）），按 Lead 要求**未打开/未截图/未删除**；书架类截图已裁剪到用户书目标行以上，仓库证据不含用户内容。
4. 设备上用户书 `source.epub` 与 `content/` 未被读取或改动（只做了目录名层面的检查）。

## 6. 未验证项（诚实标注）

- **WebView JS-ready 精确耗时**：仅测得 Activity 首帧 TotalTime（141–154 ms）+ 截图确认 2–4 s 后可交互。
- **Windows 生产安装**（`ship:local` 安装器/EXE 时间戳/SHA-256）：⏳ 待 Lead。
- **完整 `verify.ps1` 套件**：本轮只收到 Lead 的前端 check 0/0 + 263 tests；Rust/契约/Pester/zhihu/podcast 的最终数字待 Lead。
- **1.2.1→1.3.0 升级安装**：未验证（本次旧包已卸载，1.3.0 为全新安装；且签名从 debug 改为新 release 密钥，跨签名升级本身不可行，属预期）。
- **文末最后一行与底部手势条**：未验证（正文可滚动到手势条区域属沉浸式设计，固定控件未被遮挡）。
- **真实用户 SAF 导入、系统文档选择器导出/恢复**：未在真机验证（需要人工在手机上选文件；本轮用 content:// 外部打开覆盖了 external-open 路径）。
- **音量键翻页、横滑/触控仲裁手感、旋转/键盘/TalkBack/导航模式**：未真机验证。
- **多机型矩阵**（Android 8/13/15、其他厂商 ROM）：未验证（本机仅 Android 17/ColorOS）。
- **EPUB 复杂书**（DRM/加密、fixed-layout、大图/字体子集、超大归档）按设计拒绝或未覆盖真实样本。
- **桌面运行内存/启动耗时**：未实测（未启动生产应用采样；`ship:local` 后可由 Lead 或本 QA 补测）。
- **gfxinfo 的 janky 指标**只代表脚本滑动窗口内的 UI 线程帧统计，不等同整本书阅读全程性能；长文/大 EPUB 的持续滚动需更大样本。
- 浏览器验收（Playwright）不等于 Android WebView：真机首帧/PSS/帧率以 §5.2 的 adb 数据为准。

## 7. 复现命令

```powershell
# 1) 前端构建（Lead 负责；本 QA 不触发构建）
npm.cmd --prefix .\apps\desktop run build

# 2) 移动端浏览器验收（3 视口 + 真机 CSS 视口；截图写入 docs/audit/2026-10-18/screens/）
G:\python\python.exe scripts\qa\mobile\verify_mobile.py --extra-device-viewport

# 3) 真机（无线调试；先卸载旧包避免签名冲突）
pwsh -NoProfile -File .\scripts\qa\mobile\android-device-check.ps1 `
  -Serial 192.168.2.178:41031 `
  -ApkPath .\output\mobile\ImmersiveReader_1.3.0_android-universal-release.apk `
  -UninstallFirst

# 4) fixture 重新生成（可选，确定性）
G:\python\python.exe scripts\qa\mobile\make_device_fixtures.py

# 5) 构建图体积/重复打包分析（只读，不构建）
node docs\audit\2026-10-18\scripts\analyze-build.mjs --json > docs\audit\2026-10-18\build-analysis.json
```

## 8. 证据索引

| 证据 | 路径 |
|---|---|
| 性能审计（P0/P1/P2 + 数据来源） | `docs/audit/2026-10-18/PERF-AUDIT.md` |
| 构建图分析脚本/结果 | `docs/audit/2026-10-18/scripts/analyze-build.mjs`、`build-analysis.json` |
| 浏览器冒烟报告 + 48 张截图 | `docs/audit/2026-10-18/screens/mobile-smoke-report.json`、`screens/*.png` |
| harness/mock/fixture 说明 | `scripts/qa/mobile/README.md`、`mobile_mock.js`、`fixture/index.html` |
| 1.2.1 崩溃真机证据 | `docs/audit/2026-10-18/device/1.2.1-crash/` |
| 1.3.0 真机证据 | `docs/audit/2026-10-18/device/EVIDENCE.md`、`device-final-summary.json`、`*.png`、`cold-start-*.txt`、`logcat-*.txt`、`meminfo-*.txt`、`gfxinfo-*.txt` |
| 真机 fixtures | `docs/audit/2026-10-18/fixtures/` |

