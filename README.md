# 沉浸阅读

沉浸阅读是一个本地优先的长文阅读器：Windows 桌面端把知乎归档、播客转写和 Markdown 精读统一到同一座书库中；Android 端是独立的本地阅读器，专注 **Markdown 与 EPUB** 的流畅阅读。

当前发布版本：1.3.0。

## 使用

安装后从桌面或开始菜单打开“沉浸阅读”。源码工作区的开发入口是：

```powershell
.\scripts\start.ps1 desktop
```

桌面书架可以启动知乎归档、播客转写、Markdown 文件夹导入、临时 Markdown 和阅读器。生产 Node、Chromium、Python、FFmpeg 与 Whisper 模型由受管 `runtime\` 提供，不再依赖已退休的旧项目目录。

### Android

Android 端（1.3.0 起）使用全新的移动界面，只保留本地阅读：书架、导入（系统文件选择器/分享打开）、Markdown 多章文集与单文件、EPUB 2/3、目录/搜索/书签/进度、六套主题、字体排版、专注模式、音量键翻页，并适配挖孔屏与手势条安全区。知乎、播客、连读、DeepSeek 密钥属于桌面能力，Android 端不提供。

```powershell
# 构建 arm64-v8a Release APK（需要 JDK 17/21、Android SDK、NDK 与 Rust aarch64-linux-android target）
.\scripts\build-android.ps1 -Target aarch64 -Release
```

APK 输出到 `output\mobile\`。1.3.0 使用新的正式签名密钥；若手机已安装 1.2.1（debug 签名），需先卸载旧版再安装。

## 书库与阅读

默认书库：

```text
C:\Users\<用户名>\Documents\沉浸阅读\Library
```

长期内容按来源保存于 `Library\知乎`、`Library\手动` 等目录；每本书都有 `manifest.json`，阅读进度使用 `.reading.json`。精读保留 MMbook 的 Focus Mode、章节、搜索、编辑和 viewport-anchor 行为；连读使用受管的本地阅读服务。

## 数据与隐私

- 正文、图片、进度和设置均保存在本机，不进入 Git。
- 知乎数据库位于 `%LOCALAPPDATA%\ImmersiveReader\Data\Zhihu`，登录 Profile 位于 `Data\Private\ZhihuProfile`。
- Podcast 配置和任务位于 `Data\Podcast`，大文件和工作区位于 `Cache\Podcast`；DeepSeek 凭据只进入 Windows Credential Manager。
- 输入音频、模型、缓存、输出正文、日志和真实 QA 数据不属于仓库内容。

## 默认应用

正式安装已注册 `ImmersiveReader.Markdown`、`.md` 和 `.markdown` 的 Capabilities；Windows 默认应用页面已确认两种扩展均显示为“沉浸阅读”，真实 Shell 打开也已验证启动正式 EXE。

## 开发与验证

开发、测试、生产安装和发布说明见 [CONTRIBUTING.md](CONTRIBUTING.md)、[DESIGN.md](DESIGN.md)、[docs/mobile-redesign.md](docs/mobile-redesign.md) 和 [docs/release/1.3.0/RELEASE_NOTES.md](docs/release/1.3.0/RELEASE_NOTES.md)。
