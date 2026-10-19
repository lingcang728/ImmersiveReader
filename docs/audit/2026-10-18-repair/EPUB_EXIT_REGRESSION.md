# EPUB 返回书架时进度归零回归

2026-10-19 的补充真机循环检查发现：音量键将长句从约 53 px 推进到 557 px，退出专注时仍在 557 px，但返回书架后数据库的定位变成 ratio 0 / ratio 锚点。无页面异常。

在原浏览器回归中增加“等待阅读器实际卸载，再读取持久化结果”后，同样复现 ratio 0。原检查只等待仍挂在阅读器下方的书架出现，没有检查卸载后的最终写入。

根因是 Svelte 清理时正文已从文档中移除，布局尺寸归零；卸载回调再次采样并保存，覆盖了离开前已经落盘的有效位置。`sampleScroll` 现在要求正文与滚动容器均仍连接到文档，卸载时只保留最后一个有效定位。

`reading-unmount-before.log` 的新增断言失败；修改后 `reading-unmount-fixed.log` 全部 8 项通过，卸载后仍为 ratio 0.4904024767801858、文本锚点 offset 418，样式隔离、字号重排和长句分页继续通过。原始日志在忽略的 `output/repair-20261018/`，未提交书籍和数据库。

`frontend-unmount-check.log` 为 Svelte 0 errors / 0 warnings；`frontend-unmount-tests.log` 为 270 / 270。此次仅修改共享前端阅读组件，没有修改 Rust 或 IPC 契约。

最终 Windows/Android 安装版的返回书架复验和 12 次真机循环结果归入同目录 `QA_REPORT.md`；构建产物必须包含此修复，先前候选包不作为最终交付。
