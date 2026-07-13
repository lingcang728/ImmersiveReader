# 开发与验证

ImmersiveReader 是一个 Windows 单仓库产品：`apps/desktop` 是 Svelte 5 + Tauri 2 桌面端，`tools/zhihu-packer` 和 `tools/podcast-transcriber` 是由受管运行时调用的生产工具，`packages/contracts` 保存共享契约。

## 本机依赖

优先复用本机已有 Node、Rust、Python、FFmpeg、系统 Chrome 和全局 Playwright，不在仓库安装第二套浏览器或运行时。

## 常用命令

```powershell
.\scripts\start.ps1 desktop
.\scripts\verify.ps1
npm.cmd --prefix .\apps\desktop run ship:dev
```

跨包或共享契约变更必须通过 `scripts\verify.ps1`。桌面变更还要提交 Git，并报告开发 EXE 的时间和 SHA-256。测试不得读写正式 Library、数据库、Profile、Credential Manager 以外的密钥材料或生产缓存。

## 安装门

`ship:dev` 使用独立的开发 EXE、快捷方式、Data、Cache、Logs 和 Library。正式安装使用：

```powershell
npm.cmd --prefix .\apps\desktop run ship:local
```

正式安装、真实数据迁移、付费长音频、删除旧入口、Markdown 关联和远端历史修改都必须在对应 QA 证据完成后单独执行。`.md` / `.markdown` 的最终选择由 Windows 默认应用 UI 管理，不能通过伪造 `UserChoice` 哈希静默修改。

## 代码边界

UI 文案使用中文，代码标识使用英文。不要修改锁定的 Focus Mode 视觉、滚动或 viewport-anchor 算法；外部链接只允许显式 HTTP/HTTPS；所有文件访问必须走受控 Rust 命令。
