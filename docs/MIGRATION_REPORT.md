# 本地数据迁移报告

完成时间：2026-07-10 16:10（Asia/Shanghai）

## 结果

| 作者目录 | 源 Markdown | manifest 章节 | 非章节索引 | 冲突 |
|---|---:|---:|---:|---:|
| Jonathan Z | 164 | 163 | 1 | 0 |
| 你的ZombieMan | 389 | 388 | 1 | 0 |
| 茶花路莫里亚蒂 | 919 | 918 | 1 | 0 |
| 合计 | 1472 | 1469 | 3 | 0 |

- 目标：`C:\Users\15pro\Documents\沉浸阅读\Library\知乎`
- 运行数据：`%LOCALAPPDATA%\ImmersiveReader\zhihu`
- 1472 个源 Markdown 均已存在于目标书库；迁移脚本逐文件比较大小与 SHA-256。
- 3 个 `index.md` 明确列为非章节索引，没有计入 manifest。
- 数据库识别的 388 章保留原元数据；其余 1081 章写入推断元数据，并从文件名恢复可识别日期。
- 三位作者分别生成独立 `manifest.json` 与离线 `reader.html`。
- 旧归档没有重新联网补图；未来抓取继续将图片保存到书内 `assets/` 并使用相对路径。
- 知乎数据库与 1218 个浏览器登录态文件已复制到本地运行目录；日志没有作为产品数据迁移。

## 源数据保护

| 源仓库 | 分支 | 迁移后 HEAD | 状态 |
|---|---|---|---|
| MMbook | `main` | `888ea6c2ec37bd037971183871fcf8f22bf43919` | clean |
| PodcastTranscriber | `main` | `28471c085551736dc9d9cf988e81e4f159b90175` | clean |
| Zhihu_packer | `master` | `35d78f9ea83f4ebc48c269dfc07075f53f35c5da` | clean |

迁移过程只复制数据，没有提交、移动、覆盖或删除三个源项目及旧输出。

## 三项目退休与单目录交付

完成时间：2026-07-11 10:14（Asia/Shanghai）

三个源仓库的最终 HEAD 与 ImmersiveReader 历史中的 subtree 导入提交完全一致：

| 原项目 | 原 HEAD | ImmersiveReader 导入提交 |
|---|---|---|
| MMbook | `888ea6c2ec37bd037971183871fcf8f22bf43919` | `f799ec9` |
| Zhihu_packer | `35d78f9ea83f4ebc48c269dfc07075f53f35c5da` | `5a2fcff` |
| PodcastTranscriber | `28471c085551736dc9d9cf988e81e4f159b90175` | `242e4a8` |

- 三个旧目录先改名隔离；隔离期间运行时应用刷新、全套验证和真实 sidecar 烟测均通过。
- 隔离验证后，`MMbook`、`Zhihu_packer`、`PodcastTranscriber` 及其隔离目录已永久删除。
- 用户入口仅保留 `immersive-reader.exe`；Node、Chromium、Python、FFmpeg 和 Whisper 模型均由 `runtime/` 内部管理。
- 知乎数据与登录态位于 `%LOCALAPPDATA%\ImmersiveReader\zhihu`；Podcast 配置和工作数据位于 `%LOCALAPPDATA%\ImmersiveReader\podcast`。
- Podcast 模型配置已归一化为可搬移的模型目录名；离线模式成功加载随包 `faster-whisper-large-v3-turbo-local`，未访问旧目录或在线模型。
- 受管配置与日志完成旧绝对路径扫描；删除前烟测留下的 9 条 Podcast 路径已原位脱敏，其余日志行保持不变。

删除后的最终验证结果：contracts 5 项、桌面 Vitest 35 项、Rust 23 项、知乎 15 项、Podcast 15 项全部通过；Svelte 检查为 0 错误、0 警告。知乎 API、内置 Chromium 150、Whisper CUDA 模型和已安装主程序均完成真实启动烟测。书架与阅读器此前使用隔离 Playwright 浏览器完成多尺寸截图验收，没有使用桌面控制。
