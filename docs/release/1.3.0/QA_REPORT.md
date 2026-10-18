# 1.3.0 发布 QA 报告

- 日期：2026-10-18（Asia/Shanghai）
- 版本：`1.3.0`（versionCode `1003000`）
- 范围：Android 端重做为独立本地 Markdown/EPUB 阅读器、1.2.1 Android 启动崩溃修复、桌面入口分流/懒加载（外观零变化）、Windows 生产安装、Android Release APK
- 详细证据：[`docs/audit/2026-10-18/QA_REPORT.md`](../../audit/2026-10-18/QA_REPORT.md)、[`PERF-AUDIT.md`](../../audit/2026-10-18/PERF-AUDIT.md)、[`device/EVIDENCE.md`](../../audit/2026-10-18/device/EVIDENCE.md)
- 设计规范：[`docs/mobile-redesign.md`](../../mobile-redesign.md)

## 1. 交付物

### Android

| 项 | arm64（真机验证） | universal（兼容多机型） |
|---|---|---|
| 路径 | `output/mobile/ImmersiveReader_1.3.0_android-arm64-release.apk` | `output/mobile/ImmersiveReader_1.3.0_android-universal-release.apk` |
| 大小 | 16,033,256 B | 57,467,499 B |
| SHA-256 | `D52D358C3913390AC511B33AB9001E4CD1E3275CADEE9EE2BD746DBD0718DED6` | `3066699C6BF116E3E00410A1E4B2838186900A3C7E8950DC77EE9933C9016224` |
| ABI | arm64-v8a | arm64-v8a / armeabi-v7a / x86 / x86_64 |
| 包名 / versionCode / versionName | `com.lingcang.immersivereading` / 1003000 / 1.3.0 | 同左 |
| minSdk / targetSdk | 26 / 36 | 26 / 36 |
| 签名 | 新 `ir-release` 自签名密钥（`android-release.jks` + `keystore.properties` 均已 gitignore，未提交） | 同左 |
| zipalign | `-c -p 4` 通过 | `-c -p 4` 通过 |

真机：OPPO Find X9（PLJ110），Android 17 / SDK 37，1256×2760 @ density 560（CSS 359×789）。

- 1.2.1 崩溃根因真机复现：`ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin` → 进程死亡（证据 `device/1.2.1-crash/`）。
- 1.3.0：冷启动 `am start -W` TotalTime **141–154 ms**（median 141 / 153.5），logcat FATAL/ANR **0**；启动 PSS **100.8–113.2 MB**；阅读中 PSS **98.6 MB**；阅读正文滚动 **184 帧 / janky 0.00%**（p50 6ms / p90 8ms / p99 9ms）。
- 外部打开（content:// + grant）MD/EPUB、返回键、深/浅主题、安全区均已验证。

### Windows

| 项 | 值 |
|---|---|
| 安装器 | `output/desktop/ImmersiveReader_1.3.0_x64-setup.exe`（NSIS，8,196,420 B） |
| 安装器 SHA-256 | `C89631E13B0FEE83B4F7CDD947D994CB7F85F31AC1A0EA9C8CBA15C680B56DB4` |
| 已安装 EXE | `C:\Users\15pro\AppData\Local\Programs\ImmersiveReader\immersive-reader.exe` |
| EXE 时间戳 / SHA-256 | 2026-10-18 19:43:36 / `8328BD1C2F752033BA3DE59C80F5F98A67BB27DDC02E303216C8FC786963F103` |
| 桌面变化 | 外观/交互零变化；入口按平台分流 + 重组件懒加载（首屏 JS 387,298 → 307,879 B，-20.5%；CSS 113,113 → 74,069 B，-34.5%） |

## 2. 验证矩阵

| 门禁 | 结果 |
|---|---|
| `scripts\verify.ps1` | ✅ exit 0 `all checks passed`（2026-10-18 19:32） |
| 契约 fixtures | 60 fixtures / 14 TS tests 通过 |
| 前端 `svelte-check` | 0 errors / 0 warnings |
| 前端 vitest | 27 files / 263 tests |
| Rust `cargo test` | 230 passed / 0 failed |
| Rust clippy `-D warnings` / fmt | 通过 |
| zhihu-packer | 88 passed；build/compile-reader 通过 |
| podcast-transcriber | 97 passed；ruff/quick_validate 通过 |
| Pester | 14 passed |
| managed runtime manifest | 1851 entries 通过 |
| 移动端 Playwright 冒烟 | 94/94 PASS，4 视口 × 12 张互异截图（39 万字节级合成 fixture，无真实用户数据） |
| 移动首屏预算 | JS 159,881 B ≤ 250 KiB；CSS 24,092 B ≤ 80 KiB；无桌面 chunk 泄漏 |
| 桌面生产安装 | ✅ ship:local 完成（安装器 SHA-256 `C89631E1…B56DB4`；EXE 19:43:36 / `8328BD1C…63F103`；短链与 Markdown handler 已注册，UserChoice 待用户确认） |

report-only 警告：`docs/release/1.3.0/runtime-parts.json` 未生成（打包 6.9GB runtime 分包，本轮跳过）；`cargo audit` 报告 9 条既有依赖告警（quick-xml/zip/rustls 等，属独立安全升级任务）；npm audit 镜像端点不支持；PSScriptAnalyzer 未安装。

## 3. 未验证项（诚实标注）

- Android 8–16 / 非 OPPO 机型矩阵（仅编译期 minSdk 26 断言 + Find X9 实测）。
- 音量键翻页、横滑/边缘轻点手感、旋转、TalkBack、输入法弹起、SAF 文件选择器导入/导出恢复。
- 1.2.1 → 1.3.0 覆盖升级（签名变更，实测为卸载后全新安装）。
- Windows 桌面视觉逐屏比对（仅自动化检查 + 构建产物冒烟）。
- WebView JS-ready 毫秒（`am start -W` 只到 Activity 首帧；交互书架在启动后 2–4s 截图确认）。

## 4. 已知问题（非阻塞）

1. 外部打开同一 Markdown 文件会重复生成 `mobile-import-<uuid>` 书目并出现「N 个项目无法读取」告警；建议后续按来源指纹去重。
2. 通过外部打开进入的 EPUB 阅读器按返回键回桌面（任务栈语义），书架内进入的阅读器返回正常回书架；待产品确认统一。
3. Markdown 渲染管线在主线程 fallback 与 worker 中各打包一份（约 2.28MB），APK 体积可再优化；移动端已使用 lightweight 渲染（无 Shiki）。
4. `ContentReaderPlugin.kt` 仍保留已无 Rust 调用的 secrets @Command（dead code，不影响功能/启动）。
