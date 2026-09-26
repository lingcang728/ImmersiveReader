# 1.2.1 移动适配 + EPUB 支持 QA 报告

- 基线：`07c3d1d` → 交付头：`5384d21`（已推送 `origin/main`），收尾文档提交见本报告之后
- 计划来源：`C:\Users\15pro\Desktop\ir手机版.txt`
- 范围：Android 全面适配修正、导入/存储加固、阅读数据独立（reader.db）、EPUB 2/3（无 DRM、可重排）双端支持、ZIP 文集、Windows 生产包、Android Release APK

## 构建产物

### Windows

| 项 | 值 |
|---|---|
| 安装器 | `ImmersiveReader_1.2.1_x64-setup.exe`（NSIS，8,139,036 bytes） |
| 安装器 SHA-256 | `FFD4EF64D85E5D6E6A3B23F0AFCE3DE6131C06510E0DB014E1067EE054B44209` |
| 已安装 EXE | `C:\Users\15pro\AppData\Local\Programs\ImmersiveReader\immersive-reader.exe` |
| EXE 时间戳 | 2026-09-26 23:46:54 |
| EXE SHA-256 | `9339375F3358086EFCC3692647103222D3134E4C774AEF33866627A6F0C6355C` |
| 备注 | 已生成 updater 签名与 `latest.json`；`.md` UserChoice 仍待 Windows「默认应用」界面确认（不伪造受保护哈希，符合规则） |

### Android

| 项 | 值 |
|---|---|
| 产物 | `output/mobile/ImmersiveReader_1.2.1_android-universal-release.apk`（不随仓库提交，`output/` 已 gitignore） |
| 大小 | 16,635,119 bytes（≈15.9 MB） |
| SHA-256（签名后最终文件） | `F560BC74E620CFCC759BCE0AF095A56909D2A6BCCC5A739928C34BB681E1638C` |
| 包名 | `com.lingcang.immersivereading` |
| versionCode / versionName | `1002001` / `1.2.1` |
| ABI | `arm64-v8a`（aarch64-linux-android 单架构） |
| minSdk / targetSdk / compileSdk | 26 / 36 / 36 |
| zipalign | `zipalign -c -p 4` 验证通过 |
| 签名 | **debug 密钥**签名（`CN=Android Debug`）。仓库无 `keystore.properties`，release 签名配置按设计回退；APK 可安装，正式发布需自有密钥重签 |

## 测试证据

`scripts\verify.ps1` 全绿（2026-09-26，提交前）：

| 套件 | 结果 |
|---|---|
| 契约 parity（fixtures） | 60 fixtures 通过 |
| 前端 `svelte-check` + `vite build` | 0 错误 0 警告 |
| 前端 vitest | 177 通过 |
| Rust `cargo test` | 227 通过 / 0 失败（含 epub.rs 13 项、reader_db、import_ops、contracts） |
| Rust `cargo clippy -D warnings` | 通过 |
| `cargo fmt --check` | 通过 |
| zhihu-packer 测试+构建 | 88 通过 |
| podcast pytest | 97 通过 |
| Pester | 14 通过 |
| runtime manifest | 通过 |

非阻塞观察：npm audit 镜像端点返回 404/NOT_IMPLEMENTED（报告级，不阻塞）。

Android 构建链：JDK 21（自动切换自 JAVA_HOME=Zulu 25）、NDK 28.2.13676358、Gradle 8.14.3。Rust aarch64 release 编译仅 dead-code/unused 警告（desktop-only 函数在 Android cfg 下未引用，预期）。

## 逐条关闭表（对照计划）

### A 导入与存储
- A1 Kotlin 复制移出主线程：executor 后台 I/O ✅
- A2 超时≠取消：`begin_import`/`get_import_status`/`cancel_import` 操作模型，超时不再误删活动暂存 ✅
- A3 临时 `.part-*` + 原子改名，失败清理 ✅
- A4 保留相对目录/图片/附件/自然排序 ✅
- A5 双侧 canonical containment（`..`/符号链接/越界拒绝）✅
- A6 大小/总量/剩余空间预算；保留原扩展名 ✅
- A7 移动缓存根统一 + 清理统计 ✅
- A8 逐文件 issues 前端展示、文集名可编辑 ✅
- A9 `stage_content_uri` 入超时预算表 ✅

### B 移动阅读交互
- B1 返回栈修复（popstate 计数对齐）✅
- B2 横滑排除代码块/表格/图片/横向滚动容器 ✅
- B3 专注底栏跨章导航 ✅
- B4 触控仲裁：选择/控件 > 横滚 > 翻页；多指/取消不误触 ✅
- B5 音量键桥接（前台+阅读中+开关启用才接管）✅ ⚠️真机
- B6 visibilitychange/pagehide/pause 进度快照 + revision 防旧写 ✅
- B7 VIEW/SEND/SEND_MULTIPLE + onNewIntent + 待开 URI 队列 ✅
- B8 快捷键说明隐藏、返回按钮加宽、目录不自动聚焦 ✅
- SAF 目录树/多选/ZIP 文集统一导入流 ✅ ⚠️真机

### C 数据库与资源预算
- C1 手机隐藏桌面"连读"入口 ✅
- C2 Android 上"打开目录"动作降级 ✅
- C3 Keystore AES/GCM 密钥、明文 `.key` 迁移、备份排除、删除入口 ✅ ⚠️真机
- C4/C5 reader.db 独立 + bookId→目录索引，翻章/存进度不扫全库；control.db 失败仅降级 ✅
- C6 手机跳过任务轮询/维护启动成本 ✅
- C7 书库/进度/书签/偏好导出恢复 ✅ ⚠️真机 SAF
- C8 渲染预算：分块挂载、预读≤1 章、缓存 64MiB、>8MiB 分块、>64MiB 拒绝 ✅
- C9 后台停轮询/预读/动画 ✅
- 结构化日志（操作ID/阶段/耗时/错误）+ 手机诊断导出 ✅

### E EPUB
- E1 EPUB2 NCX / EPUB3 多级 nav、OPF 元数据、封面/作者/spine/图片/字体/脚注/内链 ✅
- E2 安全预算与拒绝路径（DRM/加密、fixed-layout、畸形包、路径穿越、超限归档）✅ 测试全覆盖
- E3 `publication.json` 版本化 + 旧 v1 manifest 兼容 ✅
- E4 `ReaderLocator`（章节+元素/文本锚点+章内比例）统一续读/书签/搜索 ✅
- E5 XHTML/SVG/CSS 净化、资源限本书目录、无 Tauri IPC 权限泄漏 ✅
- E6 前端 EPUB 阅读器（纵滚/跨章/目录/全书搜索/书签/续读/主题字号/专注）✅ ⚠️真机
- E7 新增命令全部注册于 lib.rs ✅
- E8 Rust/TS/JSON Schema/fixtures/契约测试同步 ✅

### F 交付
- F1 build-android.ps1 只收本次产物 + 可靠抛错 ✅（另修两处严格模式 bug：`java -version` stderr NativeCommandError、单 FileInfo `.Count`）
- F2 条件式 release 签名配置 + QA applicationId 后缀 ✅
- F3 verify.ps1 全绿 ✅
- F4 ship:local 生产包已安装，哈希见上 ✅
- F5 本报告 + git push ✅（收尾提交含脚本修复与本报告）

## 未验证项（无 ADB 设备，诚实标记）

- 真机安装/升级路径未验证（APK 已签名可装，但未在设备上执行）
- ANR、PSS/内存水位目标未在硬件上量测
- 音量键、手势仲裁手感、键盘/旋转/TalkBack/导航模式未真机验证
- Android API 26 旧机型、Android 13/15/16 实机矩阵未跑（仅编译期 minSdk/targetSdk 断言 + 单元/JVM 测试）
- SAF 树导入、系统文档选择器导出/导入未真机验证
- debug 签名与正式签名 package signature 不同，若用户机上有旧 debug 包需先卸载再装

## 已知限制

- EPUB：DRM/加密与 fixed-layout 书籍按设计拒绝；归档 256MiB/展开 1GiB/1 万条目上限
- Windows `.md` 默认程序仍须用户在系统设置中确认（不伪造 UserChoice）
- Android Release 正式分发需配置 `keystore.properties`（已加入 .gitignore）重签
- `androidx.documentfile:documentfile` 锁定 1.1.0；本地 Gradle 缓存为 1.0.0，首次构建需网络拉取（本次构建已联网成功）
