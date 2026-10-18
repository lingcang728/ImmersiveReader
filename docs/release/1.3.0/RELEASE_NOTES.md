# 沉浸阅读 1.3.0（Android 阅读器重制）

1.3.0 把 Android 端从「桌面页面的响应式补丁」重做为**独立的本地阅读器**，同时修复了 1.2.1 Android 版无法启动的根因，并在不改变桌面端外观与交互的前提下降低了桌面首屏启动成本。

## 重点

### Android：全新移动端阅读界面

- 独立移动外壳：书架（继续阅读 / 书目 / 搜索 / 导入 / 回收站）、阅读器、设置三个界面，不再加载桌面页面与桌面组件。
- 阅读器全部重写：顶部工具栏 + 点按正文显隐、章内/全书进度、上一章/下一章、无缝换章、目录（EPUB 多级）、书内搜索（EPUB 全书 / Markdown 章内）、EPUB 书签、阅读设置。
- 保留桌面阅读能力：MMbook 六套主题、字号/行高/页宽/字体族、专注模式（聚光灯 + 渐进模糊 + 点击跳句 + 底部 HUD + 视口锚点恢复）、音量键翻页、系统返回逐层关闭。
- 本地 Markdown（多章文集 + 单文件）与 EPUB 2/3（无 DRM、可重排）均支持续读进度；SAF 多选导入、外部「打开/分享」自动导入并可打开。
- Android 端彻底移除知乎归档/索引、播客转写、连读、DeepSeek 密钥、桌面窗口/路径/更新检查等桌面专属功能。

### 修复：1.2.1 Android 无法启动

1.2.1 在 Android 启动路径注册了 Kotlin `SecretsPlugin`，但该原生类从未随包提供；Tauri 的 `register_android_plugin` 会同步解析类名，缺失类导致插件 setup 失败、应用在首帧前退出（用户侧表现为卡死/闪退）。1.3.0 移除了这次注册与 Android Keystore 后端（该功能随播客/DeepSeek 一起从移动端移除），保留的 `ContentReaderPlugin` 与 Rust 侧调用一一对应。

### 性能与启动

- **启动阻塞项**：Android `setup` 只保留存储根目录初始化 + 启动日志；control.db 恢复、导入暂存清扫、发布事务恢复/索引核对等桌面专属工作改为 Windows 专属；Android 缓存清扫延后到后台线程（3s 后），不再阻塞 WebView 首帧。
- **入口分流**：`routes/+page.svelte` 只按平台动态加载桌面或移动应用；Android 不再下载/解析桌面 UI chunk。桌面首屏 JS 387,298 → 307,879 B（-20.5%），CSS 113,113 → 74,069 B（-34.5%），合计 -23.7%；设置/EPUB/知乎/播客等重组件首次进入时才加载。
- **Android 原生**：启动画面（core-splashscreen，浅/深主题背景与 MMbook 一致，消除白屏）、WebView 硬件层/禁 overscroll/滚动条、真实 WindowInsets 注入 `--ir-safe-*`（挖孔屏 + 手势条 + 横屏左右安全区）。
- **阅读热路径**：单章读取预分配缓冲、`open_book` 合并 manifest/progress 读取、EPUB 章节邻章信息不再重读 manifest；移动 Markdown 渲染跳过 Shiki、仅按需加载 KaTeX CSS、>1MiB 文档分块挂载、离开页面停止预读/动画。
- **体积**：release 构建启用 R8 + 资源收缩 + 原生符号剥离；前端资源 gzip 前总体积见 QA 报告。

### 桌面端

外观、排版、交互、主题与专注模式行为均未改变；仅内部启用入口分流与重组件懒加载（首次打开设置/EPUB/工作流多一次本地 chunk 读取，毫秒级），并保留 Windows Credential Manager、更新器与 .md 关联等桌面能力。

## 构建产物

| 平台 | 产物 | 大小 | SHA-256 |
|---|---|---|---|
| Android（主） | `output/mobile/ImmersiveReader_1.3.0_android-arm64-release.apk`（arm64-v8a，真机验证） | 16,033,256 B | `D52D358C3913390AC511B33AB9001E4CD1E3275CADEE9EE2BD746DBD0718DED6` |
| Android（兼容） | `output/mobile/ImmersiveReader_1.3.0_android-universal-release.apk`（arm64-v8a / armeabi-v7a / x86 / x86_64） | 57,467,499 B | `3066699C6BF116E3E00410A1E4B2838186900A3C7E8950DC77EE9933C9016224` |
| Windows | `output/desktop/ImmersiveReader_1.3.0_x64-setup.exe`（NSIS） | 8,196,420 B | `C89631E13B0FEE83B4F7CDD947D994CB7F85F31AC1A0EA9C8CBA15C680B56DB4` |

已安装 Windows EXE（`…\Programs\ImmersiveReader\immersive-reader.exe`）时间戳 2026-10-18 19:43:36，SHA-256 `8328BD1C2F752033BA3DE59C80F5F98A67BB27DDC02E303216C8FC786963F103`。

## 兼容性

- Android：minSdk 26（Android 8.0+），targetSdk 36；主测试机 OPPO Find X9（ColorOS 16 / Android 16，实测系统为 Android 17 / ColorOS 17 内测包，SDK 37）；APK 以 arm64-v8a 为主，另提供 universal 兼容包（含 armeabi-v7a / x86 / x86_64）。
- Windows：1.2.x 数据/书库/进度原位兼容，覆盖升级不清空数据。

## 升级须知

- **Android 签名变更**：1.2.1 使用 debug 签名，1.3.0 使用新的正式发布密钥。若手机已安装 1.2.1，请先卸载 `沉浸阅读` 再安装 1.3.0（或 `adb uninstall com.lingcang.immersivereading`），否则会报 `INSTALL_FAILED_UPDATE_INCOMPATIBLE`。
- 旧版 Android 若因启动崩溃无法使用，本次升级不迁移其应用内数据（如有）。
- Android 不支持应用内更新检查，请手动安装新 APK。

## 已知限制

- EPUB：DRM/加密、固定版式（fixed-layout）按设计拒绝；归档 ≤256MiB、展开 ≤1GiB、条目 ≤10,000。
- Android：知乎、播客、连读、DeepSeek 密钥不提供；不支持应用内更新。
- Windows：`.md` 默认程序仍需在系统「默认应用」界面由用户确认（不伪造受保护哈希）。
