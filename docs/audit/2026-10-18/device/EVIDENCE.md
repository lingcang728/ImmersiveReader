# 1.3.0 Android 真机证据（android-device-check.ps1 + 手工 adb 流程）

- 采集时间：2026-10-18 19:12–19:19（Asia/Shanghai）
- 设备：OPPO **PLJ110**（Find X9），Android **17 / SDK 37**，build `PLJ110_17.0.0.101(CN01)`，
  物理 1256×2760、density 560 → CSS 视口 359×789、DPR 3.5
- 连接：无线调试 `adb -s 192.168.2.178:41031`
- 安装包（稳定 arm64 路径，Lead 指定）：
  - `output/mobile/ImmersiveReader_1.3.0_android-arm64-release.apk`
  - **16,033,256 B**，SHA-256 `D52D358C3913390AC511B33AB9001E4CD1E3275CADEE9EE2BD746DBD0718DED6`
  - 与 `ImmersiveReader_1.3.0_android-universal-release.apk` 字节完全相同（同 SHA-256）
- 安装：`adb install -r -d` → **Success**；`dumpsys package`：`versionName=1.3.0`、
  `versionCode=1003000`、`targetSdk=36`、`minSdk=26`、`primaryCpuAbi=arm64-v8a`
- 数据保护：设备 app 库内存在用户手动数据（一本真实 EPUB一本真实 EPUB（用户手动书库数据，书名已隐去））。按 Lead 要求
  **未打开、未截图、未删除**；所有 README/QA 截图均只含合成 fixture，书架类截图已裁剪到
  用户书目标行以上的区域。

## 1.4 启动崩溃修复验证（对比 1.2.1）

| | 1.2.1 APK（SHA F560BC74…） | 1.3.0 arm64 APK（SHA D52D358C…） |
|---|---|---|
| 安装 | Success（debug 签名） | Success（新 ir-release 签名） |
| 冷启动 `am start -W` | Status ok / COLD / **TotalTime 154 ms**，随后崩溃 | Status ok / COLD / **TotalTime 152–154 ms**，正常进入书架 |
| 进程 | 启动后 `pidof` 空、`meminfo` "No process found" | 启动后进程存活，界面可交互 |
| logcat | `FATAL EXCEPTION` + **`ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin`**（WryActivity.getAppClass）→ Force finishing，连续 3 次 | **FATAL/ANR = 0**（3 次冷启动 + 阅读/外部打开全程） |
| 结论 | 打开即闪退（根因见 `1.2.1-crash/EVIDENCE.md`） | 启动崩溃已修复 |

> 说明：`am start -W` 的 TotalTime 是 Activity 首帧（含启动画面）时间；WebView JS 可交互
> 由启动后 2–4 s 截图确认（已进入书架），**未单独测量 JS-ready 毫秒数**（见未验证项）。

## 1.5 冷启动数字（1.3.0）

| 组 | run1 | run2 | run3 | 中位数 TotalTime |
|---|---|---|---|---|
| 首装（universal 文件，字节同 SHA） | 152 ms（Wait 156） | 141 ms（Wait 144） | 141 ms（Wait 145） | **141 ms** |
| arm64 重装后（`install -r`，保留数据） | 0 ms → **判为无效**（force-stop 与启动竞态，Activity 已 resumed） | 154 ms（Wait 157） | 153 ms（Wait 157） | **153.5 ms**（仅用有效两轮） |

原始输出：`cold-start-run1..3.txt`、`cold-start-final-run1..3.txt`、`cold-start-final.json`。
logcat：`logcat-final-fatal.txt` 为空（0 命中）。原始 logcat 含设备系统日志，已按隐私要求移除。

## 1.6 阅读交互证据（合成 fixture）

推送：`adb push docs/audit/2026-10-18/fixtures/... /sdcard/Download/IR-QA/`（md 6,317 B、
EPUB 3,358 B、sample-library 4 文件）。

外部打开（Android 17 上 `file://` 会被系统 Resolver 拦截，按预期；改用 content:// +
`--grant-read-uri-permission` + 显式组件，App 冷启动接收）：

```powershell
$uri = "content://com.android.externalstorage.documents/document/primary%3ADownload%2FIR-QA%2Fsample-notes.md"
adb -s 192.168.2.178:41031 shell am start -W -n com.lingcang.immersivereading/.MainActivity `
  -a android.intent.action.VIEW -d $uri -t "text/markdown" --grant-read-uri-permission
# → Status ok / COLD / TotalTime 151–161 ms；截图 02-reader-md.png 显示合成 md 正文
```

| 检查 | 结果 | 证据 |
|---|---|---|
| MD 外部打开 → 阅读器 | ✅ 合成 md 正文渲染（标题/引用/行内代码/脚注） | `02-reader-md.png` |
| 阅读滚动 3 次 | ✅ 可滚动，进度保存（书架续读 13%） | `03-reader-scrolled.png` |
| 深色主题（阅读器，跟随系统） | ✅ `rgb` 深底浅字 | `07-reader-dark.png` |
| 浅色主题（阅读器，`cmd uimode night no` 实时切换） | ✅ 浅底深字，无需重启 | `06-reader-light.png` |
| EPUB 外部打开 → EPUB 阅读器 | ✅ 合成《QA 合成 EPUB》封面 + 章节入口 | `08-epub-import.png` |
| 系统返回（从书架打开 MD 阅读器后按一次 BACK） | ✅ 回到书架，进程存活（`pidof` 有值） | `12-after-back-from-shelf-reader.png` |
| 系统返回（从**外部打开**的 EPUB 阅读器按 BACK） | ⚠️ 回到桌面（App 退出），与"外部打开"路径有关；非书架进入路径，未作为失败项，但对产品体验有影响，记录待确认 | 观察 |
| 顶部安全区（挖孔/状态栏） | ✅ 正文起始在状态栏下方，未与时间/图标重叠 | `02/06/07-*.png` |
| 底部手势条 | ⚠️ 正文可滚动到手势条区域（沉浸式滚动设计）；固定控件未被遮挡；**文末最后一行是否可完整露出未验证** | `06/07-*.png` |

## 1.7 性能测量（真机）

`dumpsys gfxinfo`（阅读器内滚动 3 次，reset 后统计）：

| 指标 | 值 |
|---|---|
| Total frames rendered | 184 |
| Janky frames | **0（0.00%）** |
| p50 / p90 / p99 | **6 ms / 8 ms / 9 ms** |
| Number Missed Vsync / Slow UI thread | 0 / 0 |

原始输出：`gfxinfo-reader.txt`（另有 `gfxinfo-framestats.csv`）。
注：书架空态下的滑动曾测得 329 帧 / janky 41（12.46%），该场景是无正文滚动 + 后台系统
活动干扰，不作为阅读性能结论；阅读正文数据以本表为准。

`dumpsys meminfo`：

| 时机 | TOTAL PSS | 原始文件 |
|---|---|---|
| 首装后启动（书架） | **113,228 KB**（≈110.6 MB） | `meminfo-launch.txt` |
| arm64 重装后启动（书架） | **100,771 KB**（≈98.4 MB） | `meminfo-launch-final.txt` |
| 合成 MD 阅读中 | **98,631 KB**（≈96.3 MB），TOTAL RSS 285,432 KB | `meminfo-reader.txt` |

## 1.8 QA 过程观察（非阻塞，建议跟进）

1. **外部打开同一 MD 会累积重复书目**：每次 content:// VIEW 会新建
   `mobile-import-<uuid>` 目录（本次 3 个，均为合成 sample-notes），书架出现多条同名/近似
   记录，且"无法读取"告警从 2 → 3 → 5 递增（`.incoming` 目录为空，重复目录内
   manifest+章节文件齐全）。建议 mobile-shell / rust-core 评估：外部打开去重（同 source
   指纹复用已有 book）与 issue 归类。
2. **外部打开 EPUB 的 BACK 行为**：返回桌面而非书架（外部打开任务栈语义），若产品期望
   "阅读器返回书架"，需要统一返回栈处理。
3. 设备上用户手动书库数据已按 Lead 要求保留且未在证据中呈现。

## 1.9 截图索引（本目录，*.png 均已确保无用户数据）

| 文件 | 内容 | 备注 |
|---|---|---|
| `01-launch.png` | 1.3.0 首启书架（空态） | 首次安装时无用户数据进入 App 库的瞬间 |
| `01-launch-final.png` | arm64 重装后启动书架 | 已裁剪到用户书目标行以上 |
| `02-reader-md.png` | 合成 MD 阅读器（深色） | |
| `03-reader-scrolled.png` | 合成 MD 滚动后 | |
| `04-back-to-shelf.png` | 阅读器 BACK 后书架 | 已裁剪 |
| `05-shelf-light.png` | 浅色书架 | 已裁剪 |
| `06-reader-light.png` | 合成 MD 阅读器（浅色） | |
| `07-reader-dark.png` | 合成 MD 阅读器（深色，实时切换） | |
| `08-epub-import.png` | 合成 EPUB 阅读器（封面/章节入口） | |
| `12-after-back-from-shelf-reader.png` | 书架打开阅读器后按 BACK 回书架 | 已裁剪 |

## 1.10 未验证项

- 文末/超长文档最后一行的底部安全区可见性。
- WebView JS-ready 精确耗时（仅 Activity 首帧 TotalTime + 截图确认可交互）。
- SAF 手动选择导入、系统文档选择器导出/恢复、音量键翻页、横滑手感、旋转/TalkBack。
- 外部打开 EPUB 的 BACK-to-shelf 行为是否为产品预期（需 Lead/产品确认）。
- 多机型/Android 8–16 矩阵（仅 Android 17/ColorOS）。
