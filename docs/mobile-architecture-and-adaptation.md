# 沉浸阅读 (ImmersiveReader) 移动端 (Android / iOS) 适配与 APK 打包方案

本文档详细记录《沉浸阅读》针对移动端（特别针对 **OPPO Find X9** 定制优化）的完整设计方案、交互适配、按键映射、跨端规划以及 Android APK 构建指南。

---

## 一、 OPPO Find X9 屏幕尺寸与系统特性画像

针对用户的目标手机 **OPPO Find X9**，调研提取关键规格与软硬件参数：

| 参数类别 | 规格参数 | 移动端软件适配方案 |
| :--- | :--- | :--- |
| **屏幕尺寸** | 6.59 英寸 AMOLED 柔性直屏 | 极佳的单手握持感，底部拇指热区 (Thumb Zone) 布局优化 |
| **分辨率** | 2760 × 1256 像素 (~460 PPI) | 高 DPI 视网膜渲染，文字排版清晰锐利，字号阶梯精细化 |
| **屏幕比例** | ~19.78:9 (~20:9 细长屏) | 竖屏阅读内容容量大；排版宽度收窄至 100% 满屏自适应，去除桌面端两侧多余留白 |
| **刷新率** | 120Hz LTPO 动态高刷 | 动效启用 GPU 硬件加速 (`will-change: transform, opacity`)，触控翻页极致跟手无拖影 |
| **色彩与发光** | AMOLED 超高对比度，3600nit 峰值 | 支持纯黑 OLED 极暗模式（墨石/素纸深色），降低功耗且暗光环境下阅读完全不刺眼 |
| **前置形态** | 居中超微打孔屏 | 顶部状态栏避让：注入 `env(safe-area-inset-top)`，工具栏下移避开前置摄像头 |
| **手势导航** | 底部手势横条 / 虚拟键 | 底部避让：注入 `env(safe-area-inset-bottom)`，确保底部进度条与专注悬浮条不被遮挡 |
| **操作系统** | **ColorOS 16** (基于 Android 16) | 完美兼容 Android 16 全面屏手势返回、暗色模式联动、音量键硬件拦截与高刷调度 |

---

## 二、 核心功能一致性保证

移动端完整保留桌面端的所有功能与核心资产，不阉割任何体验：

1. **沉浸阅读系统 (Immersive Reading)**：
   - 包含全套六款 MMbook 经典主题（素纸·明/暗、墨石·明/暗、暮光·明/暗）。
   - 保留无干扰全屏阅读模式、进度记忆、章节续读、目录导航 (TOC) 与全文搜索。
   - 保留完整的 KaTeX 数学公式渲染、Mermaid 流程图、Shiki 代码高亮与代码块一键复制。
2. **专注系统 (Focus Mode)**：
   - 完整保留核心的分句聚光灯算法 (`splitSentences`) 与视口锚点平滑对齐算法 (`getFocusScrollTarget`)。
   - 聚光灯聚焦当前句，周围句子呈渐进式模糊 (`progressive blur`) 与渐变透明度。
   - 手机端新增一键轻触跳句与底部沉浸式 HUD 悬浮导航条。
3. **播客转录系统 (Podcast Transcriber)**：
   - 保留完整的播客任务队列、中英双语对照渲染、点击展开原文对照与音频播放。
   - 针对移动端资源特性，提供云端/API 模式（DeepSeek API 密钥管理）与本地任务状态实时同步。
4. **知乎文集系统 (Zhihu Packer)**：
   - 保持知乎专栏/回答的目录解析、连读会话以及本地 Library 归档浏览。
5. **本地书架与回收站 (Bookshelf & Trash)**：
   - 本地 Markdown 文件夹扫描、历史阅读记录、软删除回收站与一键还原。

---

## 三、 手机端交互适配与按键方案

### 1. 硬件按键适配：上下音量键翻页（解决无键盘交互痛点）
手机端没有实体键盘的方向键与 PageDown/PageUp，针对性地实现**物理音量按键翻页**：
- **按键拦截**：
  - 在 Android 网页/Webview 环境下，拦截 `AudioVolumeUp` / `VolumeUp` (keyCode 24) 与 `AudioVolumeDown` / `VolumeDown` (keyCode 25)。
- **交互行为**：
  - **正常阅读模式**：
    - 音量下键（Volume Down） $\to$ 向下翻动一页（约 82% 视口高度），流畅滚屏。
    - 音量上键（Volume Up） $\to$ 向上翻动一页。
  - **专注模式 (Focus Mode)**：
    - 音量下键 $\to$ 聚光灯切换至下一句 (`moveFocus(1)`)。
    - 音量上键 $\to$ 聚光灯切换至上一句 (`moveFocus(-1)`)。
  - **控制开关**：
    - 在「设置面板」提供「音量键翻页」独立开关（移动端默认开启），若用户希望使用音量键调节媒体音量可随时一键切换。

### 2. 触控屏幕手势体系 (Markdown 阅读)
为 6.59 英寸直屏定制三区域触控与滑动手势：
- **屏幕三分区触控 (Zone Paging)**：
  - **左侧 25% 区域**：轻触向后翻页（或专注模式上一句）。
  - **右侧 25% 区域**：轻触向前翻页（或专注模式下一句）。
  - **中央 50% 区域**：轻触唤起/隐藏顶部工具栏、阅读进度与控制选项。
- **快速双击 (Double Tap)**：
  - 在文章阅读区域任意位置快速双击，立即进入或退出「专注模式」，实现无缝沉浸体验。
- **水平滑动手势 (Swipe Navigation)**：
  - 向左滑（指尖由右往左滑过 >60px） $\to$ 切换至下一章节。
  - 向右滑（指尖由左往右滑过 >60px） $\to$ 切换至上一章节。
- **文本选择防护**：
  - 当用户在屏幕上长按选词或复制文本时，自动挂起翻页手势，不打断原生选词体验。

### 3. 专注模式与触屏操作适配
- **直接轻触聚焦 (Touch-to-Focus)**：
  - 在专注模式下，读者可直接用手指轻点屏幕上的任意一句话，算法立即将聚光灯锚定在该句，并通过 `getFocusScrollTarget` 自动平滑滚动至黄金视线锚点位置。
- **移动端专注悬浮条 (Mobile Focus HUD)**：
  - 针对大屏手机单手操作优化的底部胶囊悬浮条：
    - `[ 上一句 ]` `第 X / Y 句` `[ 下一句 ]` `[ 退出 ]`
    - 位于屏幕底部安全区，拇指单手即可轻松点按连续精读。

### 4. 移动端 UI 响应式重塑
- **去除桌面窗口按钮**：
  - 在手机视图下，自动隐藏桌面端的窗口最小化、最大化、关闭按钮以及四周缩放拖拽条 (`WindowResizeHandles`)。
- **全屏自适应与安全区域 (Safe Area Insets)**：
  - 采用 `viewport-fit=cover`，文章排版自适应手机宽度，左右边距自动叠加安全距离。
  - 底部预留手势操作条安全距离，避免与系统手势产生触摸冲突。
- **抽屉式面板 (Bottom-Sheet Drawers)**：
  - 目录面板 (TOC)、设置面板 (Settings)、搜索栏在手机端均适配为顺滑的底部抽屉或全屏浮层，更符合触控人机工程学。

---

## 四、 跨端规划 (Android 与 iOS 双端架构)

本项目基于 **Tauri 2 + SvelteKit** 跨端架构设计，实现桌面端（Windows / macOS / Linux）与移动端（Android / iOS）的一套代码全覆盖：

```
                    ┌────────────────────────┐
                    │      Svelte 5 前端     │
                    │  响应式适配 + 触控手势 │
                    └───────────┬────────────┘
                                │
                                ▼
                    ┌────────────────────────┐
                    │      Tauri 2 IPC       │
                    │   (Rust 跨平台核心)    │
                    └───────────┬────────────┘
         ┌──────────────────────┼──────────────────────┐
         ▼                      ▼                      ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│   Windows 桌面   │  │   Android (APK)  │  │    iOS (App)     │
│   NSIS 安装包    │  │ OPPO / ColorOS 16│  │  iPhone / iPad   │
└──────────────────┘  └──────────────────┘  └──────────────────┘
```

1. **Android 专项配置 (`tauri.android.conf.json`)**：
   - 目标 SDK：`minSdkVersion 26` (Android 8.0+), `targetSdkVersion 35` (Android 15/16)。
   - 打包目标：生成 `aarch64` (ARM64-v8a 对应 Find X9 等所有现代安卓机) 以及 universal APK。
   - 权限申请：网络、存储、通知。
2. **iOS 专项规划 (`tauri.ios.conf.json`)**：
   - 目标：iOS 15.0+。
   - 前端手势与视口规范遵循 Apple Human Interface Guidelines，平滑过渡到 Xcode 打包。
3. **平台抽象层 (`$lib/platform/device.ts`)**：
   - 纯前端解耦平台判断，自动识别 Android、iOS、桌面或 OPPO Find X9 特征，动态切换交互逻辑。

---

## 五、 Android APK 构建与打包流程

项目提供了开箱即用的自动化构建脚本 `scripts/build-android.ps1`。

### 1. 环境准备
确保本机安装了以下工具：
- **JDK 17 或 21**（已在机器检测到 Java）
- **Android SDK**（安装 Android Studio 并勾选 Android SDK Platform 34/35 以及 Command-line Tools）
- **Android NDK**（在 Android Studio SDK Manager -> SDK Tools 中勾选 NDK）
- **Rust Android 目标平台**：
  ```powershell
  rustup target add aarch64-linux-android
  ```

### 2. 执行打包命令
在仓库根目录直接运行提供的自动化脚本：

```powershell
# 打包专供 OPPO Find X9 (ARM64) 的 Release APK
.\scripts\build-android.ps1 -Target aarch64 -Release

# 或打包全架构通用 APK (Universal)
.\scripts\build-android.ps1 -Target universal -Release
```

脚本将自动执行：
1. 检测 JDK、Android SDK、NDK 及环境变量配置。
2. 检查并安装缺少的 Rust Android targets。
3. 运行前端 `check`（类型检查）与 `build`（生产打包）。
4. 调用 Tauri 2 生成 APK 安装包。
5. 输出最终 APK 文件的路径、文件大小以及 SHA-256 校验码。
