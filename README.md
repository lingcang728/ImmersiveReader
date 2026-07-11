# 沉浸阅读

沉浸阅读是一个面向 Windows 长文阅读的本地系统，将知乎归档、播客转写和 Markdown 精读统一到同一座书库中。

## 使用方式

安装后从桌面或开始菜单打开“沉浸阅读”。可执行文件默认装在本仓库根目录（`immersive-reader.exe`）。知乎与播客所需的 Node、Chromium、Python、FFmpeg 和 Whisper 模型统一放在同目录的 `runtime/` 中，不再引用三个旧项目。源码工作区也提供统一入口：

```powershell
.\scripts\start.ps1 desktop
.\scripts\start.ps1 zhihu
.\scripts\start.ps1 podcast
```

桌面书架顶部可以直接启动知乎归档、播客转写、导入 Markdown 文件夹、临时打开单个 Markdown，以及刷新书库。生产工具缺失时，应用只显示修复说明，不会执行任意外部命令。

## 书库与阅读

默认书库位于：

```text
C:\Users\<用户名>\Documents\沉浸阅读\Library
```

长期内容按来源分开保存：

```text
Library\
  知乎\<书名>\
  手动\<书名>\
  <其他来源>\<书名>\
```

每本书都有 `manifest.json`。首次产生阅读进度时，会在书目录中创建 `.reading.json`。

- “精读”在桌面端打开，保留单文件打开、编辑、外部变更重载、目录和聚光灯模式。
- “连读”在系统浏览器打开，按书目顺序懒加载章节，并与桌面端共享进度。
- “导入 Markdown”会复制文件夹到 `Library\手动`，源目录保持不变。
- “临时打开”不会加入长期书架；播客转写也默认保持临时，只有主动导入后才成为长期书。

## 数据与隐私

- 正文、图片、进度和应用设置均保存在本机。
- 知乎登录态和数据库保存在 `%LOCALAPPDATA%\ImmersiveReader\zhihu`，不进入书库或 Git。
- 播客配置和临时工作区保存在 `%LOCALAPPDATA%\ImmersiveReader\podcast`；密钥不会写入 Git 或验证日志。
- 应用设置保存在 `%APPDATA%\immersive-reader\settings.json`。
- 播客输入、输出和工作目录仍遵循阅后即焚语义，不会删除书库。
- 浏览器连读服务只绑定随机的 `127.0.0.1` 端口，并使用一次性高熵会话令牌；桌面应用退出时服务随之停止。

## 恢复与回滚

- 备份整个 `Documents\沉浸阅读\Library` 即可保存书目、内容和整书进度。
- 损坏的进度文件会先改名为带时间戳的 `.corrupt` 备份，再恢复默认进度。
- 三个旧项目和旧 MMbook 安装均保留，可在需要时回滚；迁移只复制数据，不移动或删除源文件。
- 浏览器直接双击离线 `reader.html` 时会显示“本地模式”，此时进度仅保存在该浏览器，不会伪装成桌面同步。

项目当前只支持 Windows 桌面。真实书库、登录态、数据库、转写模型和本地配置不会提交到 Git。

开发与验证说明见 [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)。
