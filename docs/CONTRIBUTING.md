# 开发与验证

本项目采用单仓库、分开运行：

- `apps/desktop`：Svelte 5 + Tauri 2 桌面端。
- `tools/zhihu-packer`：Node/TypeScript 知乎生产端与浏览器 Reader。
- `tools/podcast-transcriber`：Python 播客转写端。
- `packages/contracts`：TypeScript/Rust 共享契约与夹具。

优先复用本机已有 Node、Rust、Python、FFmpeg、系统 Chrome与全局 Playwright，不在项目中安装第二套浏览器或运行时。统一入口会自动接入本机已有依赖：

```powershell
.\scripts\start.ps1 desktop
.\scripts\start.ps1 zhihu
.\scripts\start.ps1 podcast
```

完整验证：

```powershell
.\scripts\verify.ps1
```

真实数据迁移先做预演，再正式复制：

```powershell
.\scripts\migrate-local-data.ps1 -DryRun
.\scripts\migrate-local-data.ps1
```

隔离浏览器 QA 使用全局 Python Playwright 和系统 Chrome，不控制用户浏览器：

```powershell
G:\python\python.exe .\scripts\qa\verify_bookshelf.py
G:\python\python.exe .\scripts\qa\verify_reader.py
```

桌面安装包由 Tauri 生成，生产工具运行时、Whisper 模型、Node、Python 与 FFmpeg 均不进入安装包。本地默认安装到 monorepo 根目录（与源码同级，便于整目录清理），而不是 `%LOCALAPPDATA%`：

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
# 等价于：
# powershell -ExecutionPolicy Bypass -File .\apps\desktop\scripts\install-latest-immersive-reader.ps1 -Build
# 安装目标：C:\Users\15pro\Desktop\MyProject\ImmersiveReader\immersive-reader.exe
```

只有最终门全部通过后，才使用安装脚本的 `-RegisterMarkdownAssociations` 参数切换 `.md` / `.markdown` 文件关联。
