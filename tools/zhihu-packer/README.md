# Zhihu Packer

这是 ImmersiveReader 的受管知乎归档引擎。桌面端负责登录态、答主筛选、Top N、任务队列和结果发布；本目录负责索引、正文提取、SQLite archive catalog、revision 发布和本地 Reader 资源。

## 能力

- 使用隔离的 Playwright Profile 和受管 Chromium；系统 Chrome 只用于用户明确授权的有头登录。
- 回答、文章或合并集合支持发布时间/点赞排序与统一 Top N；成功内容先写入 `.incoming`，完整成功后才发布 archive revision。
- sidecar 通过 loopback Bearer 鉴权、READY/health 协议与桌面 Rust 控制面通信；根路径和未授权路由不得暴露控制台。
- SQLite、Profile、Cookie、缓存、输出正文和调试快照均为本地私有数据，不属于 Git。

## 本地检查

```powershell
npm.cmd --prefix .\tools\zhihu-packer test
npm.cmd --prefix .\tools\zhihu-packer run build
npm.cmd --prefix .\tools\zhihu-packer run compile-reader
```

跨包改动使用 `scripts\verify.ps1`。真实抓取必须使用独立 QA root 和受管登录态，只归档自己有权限访问的内容，并保持低速、可暂停和可恢复。
