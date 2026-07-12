# ImmersiveReader V3 To-Do List

更新时间：2026-07-12 15:04（Asia/Shanghai）

这份文件是 `ImmersiveReader 单窗口三合一整合、数据安全与干净历史实施计划 V3` 的持续交接清单，也是后续新对话的首要进度入口。实施者不需要读取旧聊天记录即可从这里继续。

维护规则：

- 每完成一个可独立验证的逻辑步骤，先完成测试与 `ship:dev`，再把对应条目从“未完成”移动到“已完成”并改为 `[x]`。
- 每个逻辑步骤单独 commit；清单更新也必须 commit。
- 不得因为写入本清单而提前勾选尚未经过真实验证的功能。
- `ship:local`、Markdown 文件关联、真实数据迁移、付费长音频、旧前端删除、远程历史修改分别受独立授权门约束。
- 禁止提交 Library、数据库、浏览器 Profile、API Key、模型、输入音频、输出正文、日志和本地配置。

## 当前交接快照

- 分支：`codex/unified-immersive-reader`
- 当前产品 commit：`bf19c60 feat(runtime): suspend sidecars until job assignment`
- 基线 `origin/main`：`1c7c72f1b1ebceb7a77d0cb0e7051789d597fa1a`
- 最新开发 EXE：`.dev-install\immersive-reader-dev.exe`
- 最新开发 EXE 时间：`2026-07-12 15:01:44`
- 最新开发 EXE SHA-256：`A96D15A97E2872189E96D0F3FCEA4564C9CC6190B34FDB7DF5026418FCDC8C4E`
- 最近全仓验证：`scripts\verify.ps1` 通过
- 当前测试：contracts 5、桌面 TypeScript 38、桌面 Rust 69、知乎 17、Podcast 19；Podcast quick validation 通过
- 正式版、正式数据、`.md/.markdown` 文件关联均未改动
- 预开发 bundle：`C:\Users\15pro\OneDrive\Documents\Codex\ImmersiveReader-Git-Backup\20260711-150053\01-pre-development.bundle`
- bundle SHA-256：`AA990BC4727505DA4DA65F30FE076859659FC8C1CDF5E4DEEE83DA8108FFCAF4`

## 已完成

### 0. 开发通道、设计与仓库规则

- [x] 将 GPT Image 2 设计稿保存到 `docs/design/reference/unified-shell-gpt-image-2.png`。
- [x] 记录四层信息架构、蓝色主题、Focus Mode、1280×800 与响应式日志抽屉规则。
- [x] 建立 `codex/unified-immersive-reader` 普通开发分支；未在 orphan history 上开发。
- [x] 建立 production/development/QA 三通道根目录模型。
- [x] 新增独立 `ship:dev`、开发 EXE、开发快捷方式和开发数据根。
- [x] `ship:dev` 不覆盖正式 EXE、正式数据或 Markdown 文件关联。
- [x] 更新 `AGENTS.md` 并创建 `docs/CONTRIBUTING.md`，将日常安装切换为 `ship:dev`。
- [x] 创建并校验 pre-development Git bundle。

### 1. 存储、设置、缓存与凭据安全

- [x] Rust 按 channel 计算 Data、Cache、Logs、RuntimeState、Backups、Library 和 runtimeRoot。
- [x] Settings schema 1/2 兼容读取为 schema 3，保留自定义 `libraryRoot`，不自动覆盖旧文件。
- [x] Settings 损坏时进入只读恢复模式。
- [x] 拒绝磁盘根、Temp、受管根、互相包含及 junction/symlink 越界的 Library 路径。
- [x] 权威文件替换改为 fail-closed；替换失败时旧字节保持不变。
- [x] Podcast Data 任务元数据与 Cache 大文件分离。
- [x] 实现 Podcast cache lease；安全清缓存跳过 queued/running/paused/interrupted/recoverable 任务。
- [x] 安全清缓存前后校验 Data、Library、Backups 未变化。
- [x] DeepSeek Key 使用 Windows Credential Manager，production/development target 隔离。
- [x] 凭据状态接口不返回 Key；TaskSpec、JSON、SQLite、日志、provenance 和备份不保存 Key。

### 2. 发布事务与数据库迁移基座

- [x] Library `.incoming` 与 `.transactions` 使用同卷发布。
- [x] 实现 `prepared / old_moved / new_moved / committed / rolled_back` 持久事务阶段。
- [x] 对 prepared、old_moved、new_moved 崩溃恢复进行幂等测试。
- [x] manifest/provenance 校验失败时恢复最后成功版本。
- [x] 实现 SQLite WAL checkpoint、integrity、foreign-key、user_version、表计数和 verified target 流程。
- [x] SQLite 迁移失败时不创建空权威库、不切换路径。
- [x] 实现 migration receipt、rollback 材料和 migration run 持久状态。
- [x] 实现只读、确定性的 legacy migration preview。
- [x] 实现 Settings schema 迁移的 preview freshness、requestId 重放和回滚验证。
- [x] 实现知乎数据库/文件系统 reconciliation 分类器，不自动选择或删除冲突候选。

### 3. Rust 控制面、单实例与进程安全基座

- [x] 创建 `Data\App\control.db`。
- [x] 持久化 `task_snapshots`、`task_events`、`command_results`、`cache_leases`、`engine_instances`、`publish_transaction_index` 和 `migration_runs`。
- [x] TaskSnapshot 使用 lifecycle/outcome/requiredAction 三维状态及 errorCode/engineStage/engineStatus/recoverable。
- [x] TaskEvent sequence/revision 单调检查与 sequence-gap 查询已实现。
- [x] requestId 幂等结果跨应用重启保留；同 key 不同输入返回冲突。
- [x] 正式版、开发版各自单实例，第二实例 Markdown 参数转发给现有窗口。
- [x] Windows Job Object `KILL_ON_JOB_CLOSE` 基座及子进程树终止测试完成。

### 4. Podcast 已完成部分

- [x] 新增 `transcribe_task.py --task-spec` 单任务入口；不再要求扫描整个 input 目录。
- [x] Python 二次验证 TaskSpec schemaVersion、受管根、相对路径和输入 SHA-256。
- [x] Podcast 输入使用 `input.partial`、流式 SHA-256、字节数与源文件稳定性校验后再原子改名。
- [x] 保存 inputSha256、pipelineVersion、engineVersion、configHash、modelHash 五项恢复兼容性。
- [x] 实现音频只读预检：SHA-256、时长、重复书目、预计磁盘、翻译规模、费用上限、可用空间。
- [x] 两个桌面真实音频已完成只读预检；未复制、未转写、未调用 API。
- [x] 已有相同 sourceId 时默认 `reuse_existing`，显式选择才创建新 revision 任务。
- [x] 未批准预算时不复制输入、不创建任务、不广播事件。
- [x] 批准的 preview 可创建持久 queued TaskSpec、recovery 和 cache lease。
- [x] queued snapshot/event 先写入 control.db，再广播 `acquisition://task-event`。
- [x] completed requestId 重放不会再次复制音频或再次广播。

### 5. 知乎已完成部分

- [x] 数据库拆分 `acquisition_tasks/acquisition_task_items` 与 `archive_authors/archive_items/archive_revisions`。
- [x] 删除任务历史不再删除永久 archive catalog、作者导航和成功输出路径。
- [x] 保留 legacy 表，不 drop 旧表。
- [x] force recrawl 不再预先物理删除成功 Markdown。
- [x] 同一成功路径不会重复创建 archive revision。
- [x] 作者与成功条目查询改为 archive catalog 驱动。

### 6. 单窗口阅读、任务与回收站 UI

- [x] 连续阅读从系统浏览器迁入 Tauri 主窗口 iframe/panel。
- [x] reader session 使用随机 token、loopback random port、路径限制和跨源写入保护。
- [x] 关闭/切换连续阅读时撤销 session；旧 token 返回 403。
- [x] 主窗口订阅 Rust TaskEvent；sequence 缺口、窗口 focus/visibility 恢复时重取完整 snapshot。
- [x] 书架顶部显示统一任务轨：任务类型、结构化状态、进度和可恢复 Cache 占用。
- [x] 新移出书架的书目写入 `trash-entry.json`，包含原相对路径、bookId、时间和 revision。
- [x] 回收站恢复拒绝覆盖冲突目录，并删除恢复后的 trash metadata。
- [x] 永久删除只能操作受管 trashId；`.trash` junction/symlink 越界被拒绝。
- [x] 回收站 restore/delete 使用 expectedRevision 和持久 requestId。
- [x] 主窗口新增回收站列表、恢复和二次确认永久删除页面。
- [x] 无 `trash-entry.json` 的 legacy `.trash` 目录保持不可操作，不会被普通清理或永久删除命令触碰。

### 7. ToolManager 受管进程所有权基座

- [x] 实现完整 ToolManager：保存 Child/进程句柄、PID、port、protocolVersion、内存 token、Job handle、startedAt、health 和 exit status。
  - 实现 commit：`8ab50b4 feat(runtime): track managed sidecar processes`。
  - 旧 HashSet 启动标记已替换为真实 Child/Job Object 所有权；状态查询会刷新实际退出状态，退出后不再误报 running。
  - Job Object 改由标准库 `OwnedHandle` 唯一持有；token 使用每次启动生成的 UUID，只保存在进程内存，快照序列化不包含 token。
  - `cargo test --lib` 68 项、严格 changed-file hygiene checker 和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE 已实际启动并确认路径，QA 进程已清理为 0；正式 EXE 和 Markdown 文件关联未改动。

### 8. Sidecar 挂起启动与 Job 绑定顺序

- [x] 使用 `CREATE_SUSPENDED | CREATE_NO_WINDOW` 启动 sidecar，先加入 Job Object 再恢复所属线程，防止子进程在绑定前逃逸。
  - 实现 commit：`bf19c60 feat(runtime): suspend sidecars until job assignment`。
  - `JobObject::spawn_suspended` 先创建挂起子进程、加入 `KILL_ON_JOB_CLOSE` Job，再通过 ToolHelp 按子进程 PID 选择所属线程并调用 `ResumeThread`。
  - Job 分配、线程枚举或恢复失败时均 fail closed：终止并回收子进程；非预期 suspend count 不继续执行。
  - `cargo test --lib` 69 项、`cargo check --all-targets`、严格 changed-file hygiene checker 和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 15:01:44`，SHA-256 `A96D15A97E2872189E96D0F3FCEA4564C9CC6190B34FDB7DF5026418FCDC8C4E`。
  - 已实际启动精确开发 EXE（QA PID 22688）并仅清理该进程，清理后为 0；正式 EXE 时间/哈希和 Markdown 文件关联均未改动。

## 未完成

以下顺序是建议的继续执行顺序。后续对话应从第一个未勾选且不受关闭授权门阻挡的条目开始。

### A. 最高优先级：让 queued 任务真正执行

- [ ] Podcast/知乎 sidecar 通过 stdout 输出 READY JSON；Rust 校验 engine、protocolVersion、PID、动态端口和 15 秒超时。
- [ ] Rust 使用异步 HTTP 与连接/读取/总超时；除 `/health` 外全部 Bearer token 鉴权。
- [ ] sidecar token 每次启动随机生成、只存在内存，不写磁盘或备份。
- [ ] 引擎异常退出时把相关任务持久化为 interrupted，不继续显示 running。
- [ ] 实现统一托盘与安全退出：保留 lease；`退出并清理` 只允许明确的 cancel_and_discard。

### B. Podcast 执行、控制与发布

- [ ] 从 queued TaskSpec 启动单个 Podcast worker，并将 stdout/stderr 结构化映射为 TaskEvent。
- [ ] 实现兼容性恢复：五项 hash 任一不兼容时不混用旧 chunks，提供重新开始新 revision。
- [ ] 实现 pause/pausing/resume、cancel、cancel_and_discard，所有控制带 expectedRevision/requestId。
- [ ] 实现 401/429/5xx/timeout 结构化错误码和 Retry-After。
- [ ] 实现累计费用预算；超过预算进入 `approve_budget`，重试不得绕过预算。
- [ ] 将 Podcast 结果写入 Library `.incoming`，生成 manifest/provenance，走发布事务并释放 lease。
- [ ] 重新转写保留旧 revision；bookId/sourceId 不因标题变化。
- [ ] 实现 `open_task_result`，成功后在主窗口打开书目。
- [ ] 在主窗口实现 Podcast 拖放/文件选择、预检、预算、重复策略、开始、暂停、恢复和结果页。
- [ ] 在上述流程真正可运行前，不删除旧 Podcast GUI 回退入口。

### C. 知乎执行、登录与发布

- [ ] 实现 Rust `create_zhihu_task`、control_task、TaskSnapshot/Event 适配器。
- [ ] 实现回答/文章/全部、排序和 Top N 合计语义；Top 5 必须总计 5 条。
- [ ] 实现受管 Chromium 登录/验证码流程；Profile 位于 Data\Private，BrowserCache 位于 Cache。
- [ ] 修正知乎 API：除 health 外全部由 Rust token 鉴权；禁止前端直接连接/SSE sidecar。
- [ ] 新抓取内容先进入 `.incoming`，成功后再发布 archive revision；partial success 保留旧成功版本。
- [ ] 生成并核对 manifest、provenance、archive revision 的 bookId/sourceId/revision/hash。
- [ ] 在主窗口实现知乎答主 ID、类型、排序、Top N、登录状态、队列和结果页。
- [ ] 指定 QA 答主：`xiao-xue-shi-46-24`。
- [ ] 在新流程真实 QA 通过前，不删除旧知乎控制台回退入口。

### D. 迁移、继承与数据对账

- [ ] 将 migration coordinator 扩展到最近打开记录和旧 MMbook recent-files。
- [ ] 迁移单 Markdown 阅读状态、Library `.reading.json` 和临时内容记录。
- [ ] 迁移旧 Podcast 未完成任务、非敏感配置与输出索引。
- [ ] 旧明文 DeepSeek Key 写入 Credential Manager并读回验证后，清除新旧 JSON 中的 key 字段。
- [ ] 迁移知乎 SQLite，执行完整 WAL/integrity/count/receipt 流程。
- [ ] 迁移知乎 Profile 到 Data\Private；不得进入 Documents 或 Backups。
- [ ] 迁移已有知乎 Markdown、manifest、provenance 和 archive catalog。
- [ ] 为 legacy `.trash` 生成迁移报告；无法推断原路径的条目保持只读并要求人工选择。
- [ ] 将 reconciliation.json / reconciliation.md 和 migration receipt 持久化到 Data\Migrations。
- [ ] 为每个数据类记录旧位置、新位置、校验、冲突、回滚和敏感性。
- [ ] 完成 dry-run 后暂停，等待“真实数据迁移”独立授权，再执行正式数据迁移。

### E. 统一 Shell、阅读保护与设置

- [ ] 把现有阅读器整体封装为独立 `ReaderWorkspace`，不重写 Focus/滚动/viewport anchor 算法。
- [ ] 实现 NavigationGuard：保存并继续、放弃并继续、取消导航。
- [ ] NavigationGuard 覆盖工作区切换、书目切换、精读/连读、返回书架、第二实例 Markdown 和退出。
- [ ] 实现完整获取内容工作台，不再通过 `launch_companion_tool` 跳到旧链接。
- [ ] 实现 Podcast 配置页、知乎配置页和 Markdown 导入页。
- [ ] 实现完整任务队列、结构化事件/日志面板和控制按钮。
- [ ] 实现书目详情、provenance、revision、来源链接和任务记录。
- [ ] 实现临时内容与最近打开页面。
- [ ] 完成设置页：Library/Data/Cache/Logs/Backups 路径、大小、打开目录、安全清理、备份、凭据、迁移和恢复状态。
- [ ] 实现 DeepSeek 配置/删除 UI，永不显示 Key。
- [ ] 实现缓存占用与可恢复任务空间 UI。
- [ ] 实现 publish recovery 与 migration recovery 页面。
- [ ] 补连续阅读 Focus Mode、章节切换、进度和 viewport anchor 回归测试。

### F. 安全收紧与旧前端移除

- [ ] 新 Podcast/知乎流程自动测试与短样本 QA 全部通过后，暂停等待“删除旧前端”独立授权。
- [ ] 获准后删除 Podcast 旧 GUI/PowerShell 托盘打包入口。
- [ ] 获准后删除知乎旧控制台打包入口。
- [ ] 收紧 Tauri CSP、capabilities、通用 `fs:default` 和 `opener:default`。
- [ ] 所有文件访问经过受控 Rust 命令；外部打开只允许显式来源 http/https 链接。

### G. QA、发布与安装

- [ ] 建立独立 QA run root，确保 QA 不读写正式 Data/Profile/Library。
- [ ] 从两个真实音频制作短片段副本并完成免费/低成本回归。
- [ ] 在 QA Library 对指定知乎账号执行回答+文章合计 Top 5。
- [ ] Podcast 与知乎各一个活动任务并行测试。
- [ ] 验证清理任务历史后知乎书目、作者导航和 archive catalog 仍存在。
- [ ] 验证托盘隐藏/恢复、退出和 Job Object 无遗留 Python/Node/FFmpeg/Chromium。
- [ ] 使用 Playwright/测试 harness 验证统一 Shell；禁止 Computer Use。
- [ ] 生成两个完整音频的时长、磁盘、文本规模、费用上限与可用空间报告。
- [ ] 暂停等待“完整长音频/API 费用 QA”独立授权。
- [ ] 获准后完整执行两个原始音频，并核对前后 SHA-256 不变。
- [ ] 完成 1.1.0 version、README、release notes、runtime manifest、release manifest 和 QA report。
- [ ] 暂停等待正式 `ship:local` 授权；获准后只安装正式版，不修改文件关联。
- [ ] Markdown 文件关联另行报告 UserChoice/Classes/恢复方案并等待独立授权。

### H. 干净 Git 历史与远程

- [ ] 产品、数据、QA 和安装全部通过后创建第二份 pre-force-push bundle并 verify。
- [ ] 记录执行时 BASE、DEV_TIP、旧 origin/main SHA。
- [ ] 从 `BASE^{tree}` 创建无父 CLEAN_ROOT，再按顺序重放 `BASE..DEV_TIP` 新 commits。
- [ ] 生成 CLEAN_TIP 与 `chore(release): package ImmersiveReader 1.1.0`。
- [ ] 核对 DEV_TIP/CLEAN_TIP 产品 tree 一致。
- [ ] 审计 clean history：无 `Co-Authored-By: Claude`、无 `Claude-Session`、Author/shortlog 符合预期。
- [ ] 生成 force-push 前完整报告：bundle 路径/哈希、恢复命令、提交数、refs 差异、EXE/NSIS/runtime/QA。
- [ ] 暂停等待 `force-push main` 独立授权；只能使用 `--force-with-lease` 替换 main。
- [ ] 删除/替换其他 remote branches/tags 必须逐项另行授权。
- [ ] force-push 后重新 clone 到新目录，验证 tree、version、README、Actions、tag、shortlog 和 trailers。
- [ ] GitHub contributors 缓存延迟不得触发第二次历史重写。
- [ ] 恢复外部 `Zhihu_packer` 仓库与设置 archive 分别等待两个独立授权。

## 当前关闭的独立授权门

- [ ] 真实生产数据迁移。
- [ ] 删除旧 Podcast/Zhihu 前端。
- [ ] 完整长音频及可能产生 API 费用的 QA。
- [ ] 正式 `ship:local`。
- [ ] 修改 `.md/.markdown` 文件关联。
- [ ] force-push `origin/main`。
- [ ] 删除或替换其他远程 branches/tags。
- [ ] 恢复外部 `Zhihu_packer` 仓库。
- [ ] 将外部 `Zhihu_packer` 仓库设为 archived。

## 下一项推荐执行

继续“A. 最高优先级”：实现 Podcast/知乎 sidecar 的 stdout READY JSON 握手，由 Rust 校验 engine、protocolVersion、PID、动态端口和 15 秒超时。暂不自动运行桌面长音频、暂不调用付费 API。完成后立即更新并勾选本文件对应条目。
