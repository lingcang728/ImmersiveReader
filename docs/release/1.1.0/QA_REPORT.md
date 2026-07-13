# ImmersiveReader 1.1.0 QA Report

验证日期：2026-07-13（Asia/Shanghai）

## 自动化门

最终 `scripts\verify.ps1` 通过：contracts 5、桌面 Vitest 38、Svelte 0 errors/0 warnings、Rust 90、Zhihu 34、Podcast 29、quick validation 通过。隔离 Playwright bookshelf harness 覆盖 3 个 viewport 和 ready/loading/empty/unwritable 4 个状态；未使用 Computer Use。

根级 `.github/workflows/release.yml` 通过 YAML 解析和 contract 检查。GitHub Action run `29240071171` 成功完成，Windows draft release 的 `v1.1.0` target 为 clean tip `a292ac6`，NSIS 资产已上传，6,482,453 bytes，digest `sha256:7ce4d06918b38b75439d6dcb1a78b557ef7d1f42141b4bfc809b11ace8efb553`。

## 真实数据与知乎

- 正式数据迁移 run `20260713-v3-production` 回执状态 `verified`，阅读状态、Podcast 配置、知乎 SQLite/Profile/Library 和 rollback 材料均完成对账，unresolved reconciliation 为 0。
- 隔离答主 `xiao-xue-shi-46-24` 完成回答+文章合计 Top 5：5/5 success、revision 1、incoming 残留 0；正式数据库、Library 和用户原有 Chrome 未被 QA 改写。
- 旧 Podcast/Zhihu 控制台和 companion frontend 已删除；sidecar READY、health、Bearer 鉴权、root 404 和进程清理均通过。

## 两份完整原始音频

- 中文原始音频：本地 faster-whisper CUDA ASR，1,460.792s、796 segments、API 请求 0；输出 26,809 bytes，SHA-256 `E8DACA5B949633FC62738404E62AD23040236E3E269F7948FBDABAE760E45BBC`。
- 英文原始音频：2,133.211438s、753/753 segments 翻译成功；DeepSeek 共 53 requests（含 polish），预算账本实付上限 `CNY 0.1063272`，低于 `CNY 0.30`；输出 67,816 bytes，SHA-256 `05D32D7EBA3426019A62926EA2ADB8BAE4629B12720598BAB4E09B57C801B95C`。
- 两个源音频前后 SHA-256 均未变化；日志无真实错误、预算拒绝、密钥形态匹配或受管残留进程。

## 安装与默认应用

- 正式 EXE：`immersive-reader.exe`，时间 `2026-07-13 17:14:32`，19,151,360 bytes，SHA-256 `0F9787EBD4D0B44ED0DD0E685CB9BD448BD344896B5A17BD3F1B697DB9503870`。
- 开发 EXE：`.dev-install\immersive-reader-dev.exe`，时间 `2026-07-13 17:38:09`，19,151,360 bytes，SHA-256 `C34EDCBF3005C406AA69706E9879F49E2DB20203733B8E677F08ED0CF9088AFA`。
- NSIS：`沉浸阅读_1.1.0_x64-setup.exe`，SHA-256 `589012A11C547E58329FA04F47EA3C085A33B50C456100A279AB738DC96364BC`；runtime manifest 17 entries，SHA-256 `C7F103D42902EB588A033B3F71D13CBBA4D0627CA90460AD791274AFCCEFEDD6`。
- `.md` 与 `.markdown` 在 Windows 默认应用页面均显示“沉浸阅读”。真实 ShellExecute 打开临时 Markdown 的进程路径为正式 `immersive-reader.exe`，验证后进程和临时文件均清理；不伪造受保护 UserChoice 哈希。

## Clean history 与外部仓库

- `BASE=1c7c72f`，`CLEAN_ROOT=700d146f`（无父），`CLEAN_TIP=a292ac6`；168 个 commits 按作者/邮箱/标题顺序重放，最终 170 commits、单根、无 merge、无 Claude trailers，产品 tree 与 `DEV_TIP` 一致。
- `origin/main` 已用 `--force-with-lease` 替换为 `a292ac6`，`v1.1.0` 指向同一 commit；fresh clone、`git fsck --full --strict`、README、版本、Actions、tag、shortlog 均通过。
- `lingcang728/Zhihu_packer` 已从原始 7 个提交的 Git 对象恢复为私有仓库，`master=35d78f9` 验真后设置 `archived=true`。

真实用户数据、Profile、数据库、音频、模型、缓存、输出和凭据不属于 Git 发布内容。
