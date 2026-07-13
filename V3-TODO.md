# ImmersiveReader V3 To-Do List

更新时间：2026-07-12 22:43（Asia/Shanghai）

这份文件是 `ImmersiveReader 单窗口三合一整合、数据安全与干净历史实施计划 V3` 的持续交接清单，也是后续新对话的首要进度入口。实施者不需要读取旧聊天记录即可从这里继续。

维护规则：

- 每完成一个可独立验证的逻辑步骤，先完成测试与 `ship:dev`，再把对应条目从“未完成”移动到“已完成”并改为 `[x]`。
- 每个逻辑步骤单独 commit；清单更新也必须 commit。
- 不得因为写入本清单而提前勾选尚未经过真实验证的功能。
- `ship:local`、Markdown 文件关联、真实数据迁移、付费长音频、旧前端删除、远程历史修改分别受独立授权门约束。
- 禁止提交 Library、数据库、浏览器 Profile、API Key、模型、输入音频、输出正文、日志和本地配置。

## 当前交接快照

- 分支：`codex/unified-immersive-reader`
- 当前产品 commit：`e389914 feat(settings): add recovery center`
- 基线 `origin/main`：`1c7c72f1b1ebceb7a77d0cb0e7051789d597fa1a`
- 最新开发 EXE：`.dev-install\immersive-reader-dev.exe`
- 最新开发 EXE 时间：`2026-07-12 22:38:31`
- 最新开发 EXE SHA-256：`39CCA58AC8D00A7EE866A1F24B23B278E13A657119808646FA6A56A52372687A`
- 最近全仓验证：`scripts\verify.ps1` 通过
- 当前测试：contracts 5、桌面 TypeScript 38、Svelte 0 警告、桌面 Rust 87、知乎 25、Podcast 27；quick validation 通过
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

### 9. Sidecar READY JSON 握手

- [x] Podcast/知乎 sidecar 通过 stdout 输出 READY JSON；Rust 校验 engine、protocolVersion、PID、动态端口和 15 秒超时。
  - 实现 commit：`5913f91 feat(runtime): add sidecar READY handshake`。
  - Zhihu Node sidecar 与 Podcast Python sidecar 均在 loopback 动态端口绑定后首行输出 `engine`、`protocolVersion`、`pid`、`port`；Rust 首行读取后继续排空 stdout，避免日志阻塞。
  - Rust 校验失败、stdout EOF 或 15 秒超时均关闭 Job、终止并回收 Child；成功握手后 ToolManager 保存动态 port、protocolVersion、health 和内存 token。
  - `cargo test --lib` 74 项、`cargo check --all-targets`、严格 changed-file hygiene checker、`cargo fmt` 和 `scripts\verify.ps1` 全部通过；Zhihu 19 项、Podcast 21 项和 quick validation 通过。
  - 源码与安装 runtime 均实际启动两个 sidecar 并请求 `/health`：READY PID/动态端口匹配且返回 `ok`；QA 进程已清理。
  - `ship:dev` 通过；开发 EXE `2026-07-12 15:31:55`，SHA-256 `4D622EC00B4EAD0D977B2141CBEBE285E0C9902AE17C10DD51E3950DA2E8B54A`；正式 EXE、正式数据和 Markdown 文件关联未改动。
  - 为完整验证修复未安装 PyAV 时的 WAV 时长 fallback，独立 commit：`4995301 fix(podcast): add stdlib wav duration fallback`。

### 10. Sidecar 异步 HTTP、超时与 Bearer 鉴权

- [x] Rust 使用异步 HTTP 与连接/读取/总超时；除 `/health` 外全部 Bearer token 鉴权。
  - 实现 commit：`c1ef6c5 feat(runtime): enforce sidecar bearer HTTP`。
  - Rust reqwest 使用 5 秒连接、10 秒读取、15 秒总超时；READY 后先检查 `/health`，再带 Bearer 请求 `/api/status`，失败时关闭 Job 并回收 Child。
  - Zhihu/Podcast `/api` 均拒绝缺失 Bearer、旧 `X-Zhihu-Packer-Token` 和 query token；`/health` 保持无认证。Zhihu 前端 API、流式事件和下载均改为 Bearer。
  - 内存 token 不写磁盘或备份；Zhihu UI 只通过 URL fragment 接收 token，历史 URL/query token 不再作为认证路径。
  - `cargo test --lib` 77 项、`cargo check --all-targets`、严格 changed-file hygiene checker、`cargo fmt` 和 `scripts\verify.ps1` 全部通过；Zhihu 20 项、Podcast 22 项和 quick validation 通过。
  - 源码与安装 runtime 均实际验证 `/health=200`、缺失/旧认证 `401`、正确 Bearer `200`；QA 进程已清理。
  - `ship:dev` 通过；开发 EXE `2026-07-12 15:50:56`，SHA-256 `0842D41B19E16ADFBB5E1282996199276D936D85316D7042F42AE527681993B9`；正式 EXE、正式数据和 Markdown 文件关联未改动。

### 11. Sidecar token 生命周期

- [x] sidecar token 每次启动随机生成、只存在内存，不写磁盘或备份。
  - 实现与审计证据：`c1ef6c5 feat(runtime): enforce sidecar bearer HTTP`；Rust UUID v4 在每次 launch 前生成，Child 环境和 ToolManager 内存 descriptor 是唯一运行时承载面。
  - ToolManager snapshot 序列化不包含 token；READY JSON、数据库、日志、备份和静态文件不写 token。Zhihu fragment 在 API 初始化前用 `history.replaceState` 移除，query/旧 header 认证路径拒绝。
  - `git grep` 持久化审计无 token write/append/backup/serialization 路径；源码与安装 runtime Bearer 冒烟、完整 Rust/Node/Python 验证和 `ship:dev` 均通过。

### 12. 引擎异常退出与 interrupted 恢复

- [x] 引擎异常退出时把相关任务持久化为 `interrupted`，不继续显示 `running`。
  - 实现 commit：`30fc0d6 feat(runtime): persist interrupted tasks on engine exit`。
  - `engine_instances` 记录受管引擎 PID/端口/协议和运行状态；ToolManager 刷新到退出状态后，将同引擎活动任务一次性写入 `Terminal + Interrupted + ENGINE_CRASHED`，保留可重试标记并禁止继续 pause/resume/cancel。
  - 应用重新打开时只执行一次 stale engine recovery；仍为 `running` 的旧 PID 同样转为 `interrupted`，重复恢复和重复退出通知均幂等，不产生重复 TaskEvent。
  - 新增 live crash、reopen stale recovery、idempotence 测试；`cargo fmt --check`、`cargo test --lib` 79 项、`cargo check --all-targets` 和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:05:27`，SHA-256 `10D194B39A810934EDB28EBEDDD85347558AB99A00004FC11FCDD6706A88D2B7`；精确开发 EXE QA PID `84344` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE `immersive-reader.exe` 时间 `2026-07-11 09:49:40`、SHA-256 `47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；Markdown 文件关联未改动。

### 13. 统一托盘与安全退出

- [x] 实现统一托盘与安全退出：保留 lease；`退出并清理` 只允许明确的 `cancel_and_discard`。
  - 实现 commit：`a81870a feat(desktop): add tray safe-exit actions`。
  - Tauri tray menu 提供显示、隐藏、`退出（保留任务）` 和 `退出并清理（取消任务）`；窗口关闭改为隐藏到托盘，不会自动释放任务 lease。
  - 保留任务退出只保存编辑状态/关闭 reader session 后调用普通 `quit_app`；清理退出才调用 `cancel_and_discard`，停止受管 sidecar，将活动任务写入 `Cancelled + CANCELLED_BY_USER`，并删除 Podcast task cache/recovery。
  - `cancel_active_tasks` 幂等测试通过；cache discard 路径校验受管 Data/Cache 根；桌面 TypeScript 38、Rust 80、Svelte 0 警告、`scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:16:50`，SHA-256 `E41957F1C3CFF863EAAB2000555BA2C70A6FFF960CAED664D59246553C325932`；精确开发 EXE QA PID `91832` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；Markdown 文件关联未改动。

### 14. Podcast queued worker 与结构化任务事件

- [x] 从 queued TaskSpec 启动单个 Podcast worker，并将 stdout/stderr 结构化映射为 TaskEvent。
  - 实现 commit：`5d86e50 feat(podcast): run queued task workers`。
  - 书架 queued Podcast 任务提供“开始”；Rust 校验 taskId/TaskSpec/受管 runtime 后只启动一个 worker，Windows 使用 Job Object，worker stdout/stderr 由单一 dispatcher 顺序写入 control.db 并广播 `acquisition://task-event`。
  - 输出行映射为 `worker_stdout`/`worker_stderr`，识别百分比和 normalizing/chunking/transcribing/translating/writing stages；进程成功/失败分别写入 `worker_completed`/`worker_failed` 终态。
  - 新增 worker 进程状态、stdout/stderr/百分比/终态映射测试；`cargo test --lib` 82 项、`cargo check --all-targets`、Svelte 0 警告和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:27:38`，SHA-256 `C5F66E4458C218CF79C705C62F9D97C72C4FA3D6E4A533BA3B709C571D88A433`；精确开发 EXE QA PID `93060` 启动路径正确，停止后残留开发进程为 0。
  - 未执行真实音频或付费 API；正式 EXE 时间/哈希和 Markdown 文件关联未改动。

### 15. Podcast 兼容性恢复与新 revision

- [x] 实现兼容性恢复：五项 hash 任一不兼容时不混用旧 chunks，提供重新开始新 revision。
  - 实现 commit：`caabaef feat(podcast): restart incompatible tasks as new revisions`。
  - worker fatal JSON 的 `INPUT_CHANGED`、`PIPELINE_INCOMPATIBLE`、`MODEL_INCOMPATIBLE`、`CONFIG_INCOMPATIBLE` 映射为结构化 TaskErrorCode；失败任务显示“新 revision”。
  - 新 revision 重新校验并复制受管 input，创建全新 cache/task.json/recovery，publish revision 递增，旧 task/chunks 保留但不会被新 worker 复用。
  - 新增 fresh-cache/next-revision 测试；`cargo test --lib` 83 项、`cargo check --all-targets`、Svelte 0 警告和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:37:17`，SHA-256 `37EEA47A547A8EF886E3434422DE3596C974767D43D1B837FBCEFAF0DDF3B87B`；精确开发 EXE QA PID `21104` 启动路径正确，停止后残留开发进程为 0。
  - 未执行真实音频或付费 API；正式 EXE 和 Markdown 文件关联未改动。

### 16. Podcast pause/resume/cancel 控制

- [x] 实现 pause/pausing/resume、cancel、cancel_and_discard，所有控制带 expectedRevision/requestId。
  - 实现 commit：`b3c0f21 feat(podcast): add revision-safe task controls`。
  - Windows worker 通过受管 PID 的线程枚举执行 suspend/resume/terminate；Rust 控制命令先校验 expectedRevision，再通过 control.db requestId 幂等 claim/complete。
  - pause/resume/cancel/cancel_and_discard 状态变化写入 TaskEvent；普通 cancel 保留 cache lease 并允许 retry，cancel_and_discard 才删除 task cache/recovery；书架提供对应控制按钮和二次确认。
  - 新增 revision 冲突、pause/resume/cancel 状态迁移测试；`cargo test --lib` 84 项、`cargo check --all-targets`、Svelte 0 警告和 `scripts\verify.ps1` 全部通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:47:57`，SHA-256 `77F862C6E1E9A5954BFB7C1EF39406108CBEB77A5E1F333EFFE0B083C26F741B`；精确开发 EXE QA PID `20040` 启动路径正确，停止后残留开发进程为 0。
  - 未执行真实音频或付费 API；正式 EXE 和 Markdown 文件关联未改动。

### 17. Podcast 上游错误与 Retry-After

- [x] 实现 401/429/5xx/timeout 结构化错误码和 Retry-After。
  - 实现 commit：`844cba6 feat(podcast): classify upstream errors`。
  - DeepSeek 与 Ollama HTTP/网络路径统一识别 `UPSTREAM_UNAUTHORIZED`、`RATE_LIMITED`、`UPSTREAM_UNAVAILABLE`、`UPSTREAM_TIMEOUT`；429/5xx 的 `Retry-After` 支持秒数与 HTTP-date，重试退避不会超过 120 秒。
  - `transcribe_task.py` 将上游异常输出为 stderr fatal JSON，包含 `errorCode`、消息和可选 `retryAfterSeconds`；Rust 映射到 TaskErrorCode，并在 TaskSnapshot/Event 的 `retryAfterSeconds` 保留结构化等待信息。
  - 新增无网络单元测试覆盖 401、429、503、Retry-After 与 timeout；未调用真实 DeepSeek/Ollama API。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 84、知乎 20、Podcast 25、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 16:58:28`，SHA-256 `B711E29FE6B670E5E5EBC5AAAC939B192A0E8859B88586698048FC5F8668E825`；精确开发 EXE QA PID `90392` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 18. Podcast 累计费用预算

- [x] 实现累计费用预算；超过预算进入 `approve_budget`，重试不得绕过预算。
  - 实现 commit：`28b0974 feat(podcast): enforce cumulative api budget`。
  - 批准的 `budgetLimitCny` 写入每个 TaskSpec；Python worker 在受管 Cache 下维护原子 budget ledger，按输入/输出 token 和 CNY/USD 换算记录累计 spend。
  - 每次请求按内部最大重试次数预留额度，成功按 usage 结算，失败保留预留额度；并发/外层重试读取同一账本，不能绕过累计上限。
  - TaskSpec 预算低于已验证 preview estimate 时 fail closed；超限输出 `BUDGET_CONFIRMATION_REQUIRED`，Rust 映射 `RequiredAction::ApproveBudget` 且禁止直接 retry。
  - 新增 TaskSpec 预算下限、Rust `approve_budget` 状态和无网络预算账本测试；未执行真实音频或付费 API。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 84、知乎 20、Podcast 26、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:13:29`，SHA-256 `D0ED2D94D418E56E1A20F0694DB55312E33C56F92B09F6A46385F9F954CD67D5`；精确开发 EXE QA PID `25492` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 19. Podcast 结果发布事务与 lease 释放

- [x] 将 Podcast 结果写入 Library `.incoming`，生成 manifest/provenance，走发布事务并释放 lease。
  - 实现 commit：`0b23f4c feat(podcast): publish completed results`。
  - worker 成功退出后只读取受管 Cache `output`，拒绝 symlink/不安全路径，复制 Markdown 到 Library `.incoming/<taskId>`，生成 podcast manifest、章节元数据和 provenance。
  - 发布使用现有 Prepared/OldMoved/NewMoved/Committed/RolledBack 事务链，写入 `publish_transaction_index`；重复调用读取已提交 journal 并幂等释放 lease。
  - commit 成功后写 `Podcast/<sourceId>`，旧版本按 `.revisions/<sourceId>/<revision>` 回滚材料保留；发布失败以 `PUBLISH_FAILED` 终态保留恢复空间，不报告成功。
  - 新增无网络发布集成测试：实际复制 Markdown、校验 manifest/provenance、事务 committed、lease 变为 released、重复调用幂等。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 85、知乎 20、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:24:20`，SHA-256 `74768ABE2384AE29049ECFB19E99F4143311BF11599293B5BCE0ACD81B4E4A0A`；精确开发 EXE QA PID `99144` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 20. Podcast 重新转写与 revision 保留

- [x] 重新转写保留旧 revision；bookId/sourceId 不因标题变化。
  - 实现 commit：`4b48703 feat(podcast): preserve revisions on retry`。
  - retry 入口不再只限兼容性错误；所有 terminal 且 `canRetry` 的失败/取消/中断任务都创建新 task/cache，旧 task、旧 revision 和旧 chunks 保留。
  - 新 TaskSpec 的 publish revision 递增，bookId/sourceId 从旧 TaskSpec/快照原样继承；新任务不复用旧 cache/chunks，重新复制并验证输入。
  - 预算确认门任务不会显示直接重试；书架对普通可重试终态显示“重新转写 revision”。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 85、知乎 20、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:29:55`，SHA-256 `FD69AB2696A619AD87C22B7A67912CCAC9BA0B744D8D764A2BD23FD447A3E1EB`；精确开发 EXE QA PID `85300` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 21. `open_task_result` 与主窗口阅读

- [x] 实现 `open_task_result`，成功后在主窗口打开书目。
  - 实现 commit：`ff8ef97 feat(desktop): open podcast task results`。
  - Rust 命令只接受成功终态，读取已发布 snapshot 的 bookId 并重新验证当前 Library 书目后返回 BookDetail；未发布/失败任务 fail closed。
  - 书架成功 Podcast 任务显示“打开结果”，前端复用现有 `openLibraryBook`、chapter path、阅读状态和 viewport anchor 保护路径。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 85、知乎 20、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:35:02`，SHA-256 `3DFCD9622C98365BA0AB1E22DD4554C585DE0E739505374DDC12D49C2B95A4AA`；精确开发 EXE QA PID `94140` 启动路径正确，停止后残留开发进程为 0。
  - 正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 22. Podcast 主窗口执行工作台

- [x] 在主窗口实现 Podcast 拖放/文件选择、预检、预算、重复策略、开始、暂停、恢复和结果页。
  - 实现 commit：`1965b42 feat(desktop): add podcast workflow panel`。
  - 新增单窗口 Podcast 工作台：接收 Tauri 拖放和多文件选择，限制 MP3/M4A/WAV；调用受控 Rust `preview_podcast_files` 展示时长、缓存、可用空间和 API 费用上限。
  - 预算超出时要求显式确认；重复来源可选择复用已有书目或创建新 revision；任务通过幂等 requestId 加入统一队列，队列卡片提供开始并复用既有暂停、恢复、取消和打开结果动作。
  - 结果状态展示任务终态，并可从工作台打开成功发布的书目；旧版 Podcast GUI 回退按钮保留在同一流程页。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 85、知乎 20、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:45:23`，SHA-256 `5F34348CBB759937C7113EF11DBACB635564E5E5623646C5E4C4A63E4744223A`；精确开发 EXE QA PID `100568` 启动路径正确，停止后残留开发进程为 0。
  - 未运行真实音频、DeepSeek/Ollama 或付费 API；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 文件关联未改动。

### 23. Zhihu 统一任务执行与控制适配器

- [x] 实现 Rust `create_zhihu_task`、`control_zhihu_task`、`TaskSnapshot/Event` 适配器。
  - 实现 commit：`95393d3 feat(zhihu): add unified task control adapter`。
  - 受管 sidecar 通过 Bearer HTTP 创建任务；Rust 校验答主 ID、内容类型、Top N 和排序，持久化统一 queued 事件，并提供开始、暂停、恢复、取消的幂等 requestId 控制命令。
  - 新增 `/api/tasks/:id` 状态查询与安全停止端点；后台轮询将 sidecar pending/running/paused/success/partial_success/failed 进度映射到共享 TaskSnapshot/Event，统一书目 sourceId/bookId 和恢复/重试能力。
  - `mark_task_starting` 与外部快照更新保持 sequence/revision 单调；sidecar token 只留在进程内存，HTTP 客户端限定 loopback 与 Bearer。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 20、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 17:59:36`，SHA-256 `981254CC74967BA24273399DC69BA1AE3C4EEC971EC9B740354FC160CE1E3F09`；精确开发 EXE QA PID `99468` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录或验证码；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 24. Zhihu 内容类型、排序与 Top N 合计语义

- [x] 实现回答/文章/全部、排序和 Top N 合计语义；Top 5 必须总计 5 条。
  - 实现 commit：`f825aa2 fix(zhihu): enforce combined top n selection`。
  - scheduler 和 CLI 对 `all` 先收集合并回答/文章索引，再用统一排序选择器按发布时间或点赞数排序；Top N 在合并集合上截取，因此 Top 5 不会变成回答 5 + 文章 5。
  - 选择器使用次级排序和稳定 id tie-breaker，避免同时间/同点赞结果漂移；保留 `answers`/`articles` 单类型过滤语义。
  - 新增 2 个选择器测试覆盖合计数量、点赞排序、时间排序和输入不变性；`scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 22、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 20:20:58`，SHA-256 `C6AB0C8A6BFFC6ABED4C7A1A90924BDD4D70DD133B60ADADE44D32D2E9CD40BE`；精确开发 EXE QA PID `94380` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录或验证码；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 25. Zhihu 主窗口执行工作台

- [x] 在主窗口实现知乎答主 ID、类型、排序、Top N、登录状态、队列和结果页。
  - 实现 commit：`78a09b4 feat(desktop): add zhihu workflow panel`。
  - 新增单窗口知乎工作台：答主 ID、回答/文章/全部、发布时间/点赞排序、Top N 输入、登录状态刷新、任务创建和统一队列状态；书架任务栏提供知乎开始/暂停/恢复/取消。
  - 登录状态由受管 sidecar `/api/login-status` 查询 Profile Cookie；旧版登录/控制台按钮保留，未在本次 QA 自动打开浏览器或访问知乎。
  - 结果卡片展示成功/部分成功/失败终态、完成数量与错误信息；后续发布到 Library 仍由独立发布条目负责。
  - `scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 22、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 20:31:55`，SHA-256 `0D59E309D41391B07C2604CBB959B6FB41A6830BF48CECCE71AF00D9BCA99D3E`；精确开发 EXE QA PID `98764` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录、验证码或外部网络任务；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 26. Zhihu 受管登录、验证码与路径隔离

- [x] 实现受管 Chromium 登录/验证码流程；Profile 位于 Data\Private，BrowserCache 位于 Cache。
  - 实现 commit：`a7339db feat(zhihu): isolate managed browser profile and login`。
  - launcher 将 Zhihu SQLite 放入 `Data\Zhihu`，Profile 放入 `Data\Private\ZhihuProfile`，并设置 `IMMERSIVE_ZHIHU_BROWSER_CACHE` 到 `Cache\Zhihu\BrowserCache`；Playwright 使用 managed Chromium 和 `--disk-cache-dir`。
  - sidecar 新增 Bearer 鉴权 `/api/login/status` 与 `/api/login/start`，主窗口可刷新登录态并启动受管有头登录；现有 scheduler 的 CAPTCHA 交互流程继续复用同一受管 Profile。
  - 新增路径解析测试；`scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 23、Podcast 27、quick validation；`cargo check --all-targets` 通过。
  - `ship:dev` 通过；开发 EXE `2026-07-12 20:39:44`，SHA-256 `857D6E0B65E8D8D3BD1B77485024DD47A0A65BCA76733ABC410961697A145FD3`；精确开发 EXE QA PID `67336` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实登录、验证码或知乎网络任务；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 27. Zhihu API 统一鉴权边界

- [x] 修正知乎 API：除 health 外全部由 Rust token 鉴权；禁止前端直接连接/SSE sidecar。
  - 实现 commit：`a10f74b fix(zhihu): guard every api route with bearer auth`。
  - 在所有 `/api` 路由之前安装统一 Bearer token 中间件，`/health` 保持唯一匿名探活入口；旧知乎控制台和 SSE 仅作为显式回退入口保留，新主窗口工作流只经 Rust Tauri 命令访问 sidecar。
  - `npm.cmd --prefix .\tools\zhihu-packer test` 通过（知乎 23）；`npm.cmd --prefix .\tools\zhihu-packer run build` 通过；`scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 23、Podcast 27、quick validation。
  - `ship:dev` 通过；开发 EXE `2026-07-12 20:44:47`，SHA-256 `157FAAEECB6F33453E7BEB258772DA7BB1E6F079EC22238BAD0B867EBD3DFF96`；精确开发 EXE QA PID `69428` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录、验证码或外部网络任务；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 28. Zhihu `.incoming` 与 archive revision 发布事务

- [x] 新抓取内容先进入 `.incoming`，完整成功后发布 archive revision；partial success 保留旧成功版本。
  - 实现 commit：`7c7b19c feat(zhihu): publish staged archive revisions`。
  - 正文先写入 `Library/知乎/.incoming/<taskId>/<author>`；只有任务全部成功才通过 prepared/old_moved/new_moved/committed journal 将作者目录移动到当前归档，旧目录保留在 `.revisions/<authorId>/<revision>`；partial success 不更新 archive catalog，暂存结果留在 `.incoming` 供失败条目续跑。
  - 成功发布后才把 task item 写入 archive catalog 并生成当前导航索引；发布失败转为失败终态并保持旧成功版本。
  - 新增无网络发布事务测试，覆盖旧版本保留、提交 journal、路径映射和 partial 隔离；`scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 25、Podcast 27、quick validation。
  - `ship:dev` 通过；开发 EXE `2026-07-12 20:56:05`，SHA-256 `24E14B03005F9F5325FC013113FF13CABAE5894E6E72162E742AE754D70A37F2`；精确开发 EXE QA PID `28984` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录、验证码或外部网络任务；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 29. Zhihu manifest、provenance 与 revision hash 绑定

- [x] 生成并核对 manifest、provenance、archive revision 的 bookId/sourceId/revision/hash。
  - 实现 commit：`32f0f21 feat(zhihu): bind archive metadata to revisions`。
  - 发布前在 `.incoming/<taskId>/<author>` 原子写入并解析 `manifest.json` 与 `provenance.json`；journal 同时记录 `bookId=zhihu:<authorId>`、`sourceId`、revision、manifest SHA-256 和 provenance SHA-256，并在旧目录移动前后再次核对身份与哈希。
  - manifest 章节来自本次成功 task items，包含安全相对路径、标题、日期、点赞数和字数；provenance 记录创建任务、最后成功任务、引擎版本及 manifest hash。
  - 新增测试核对 metadata 文件、journal 字段和 SHA-256 长度；`scripts\verify.ps1` 通过：contracts 5、桌面 TypeScript 38、Svelte 0 警告、Rust 87、知乎 25、Podcast 27、quick validation。
  - `ship:dev` 通过；开发 EXE `2026-07-12 21:05:17`，SHA-256 `37CD2417B7286820A61D21E3ADD877659F0D988E543C4037491F2E53CDA0F95E`；精确开发 EXE QA PID `12024` 启动路径正确，停止后残留开发进程为 0。
  - 未启动真实知乎抓取、登录、验证码或外部网络任务；正式 EXE 时间/哈希 `2026-07-11 09:49:40 / 47C39DF121129215735520C18E54919B631CEAB73AF73EB97230441A9B57BA1F` 未变；`.md/.markdown` 关联仍为 `ImmersiveReader.Markdown`，open command 未变。

### 30. Zhihu 指定答主隔离 QA 准备

- [x] 指定 QA 答主：`xiao-xue-shi-46-24`。
  - 实现 commit：`106991e test(zhihu): add isolated qa run preparation`。
  - 新增 `scripts\qa\prepare-zhihu-run.ps1`，固定回答+文章合计 Top 5 方案，生成独立 `ImmersiveReader-QA-<runId>` Data/Cache/Profile/BrowserCache/Library 和 qa receipt；`RunId`、People ID 均做安全字符校验。
  - 已执行 `-RunId zhihu-v3-20260712`，QA Local/Library 均存在且与正式 Local/Data/Library 不重叠；非法 `bad/run` 被拒绝。此步骤未启动知乎网络、登录、验证码或外部任务。

### 31. 独立 QA run root

- [x] 建立独立 QA run root，确保 QA 不读写正式 Data/Profile/Library。
  - 实现与验证 commit：`106991e test(zhihu): add isolated qa run preparation`。
  - `prepare-zhihu-run.ps1` 为 QA channel 创建独立 Local Data/Cache/Profile/BrowserCache 与 Documents\Codex Library，并写入 run receipt；已验证 QA 路径不等于或嵌套于正式路径。

### 32. 统一获取内容工作台

- [x] 实现完整获取内容工作台，不再通过 `launch_companion_tool` 跳到旧链接。
  - 已由 `1965b42 feat(desktop): add podcast workflow panel` 与 `78a09b4 feat(desktop): add zhihu workflow panel` 完成 Podcast/知乎主窗口工作台；旧控制台仅保留为回退入口。
  - 当前主窗口通过受控 Rust 命令执行预检、创建任务、控制任务和打开结果；未授权删除旧入口。

### 33. 临时内容与最近打开页面

- [x] 实现临时内容与最近打开页面。
  - 由 `eedde28 refactor+feat: 拆分 +page.svelte 组件、最近打开列表、外部变更自动重载`、`888ea6c fix: prune stale recent markdown entries` 和现有 `list_temporary_content` Rust 命令覆盖。
  - `scripts\verify.ps1` 与桌面 38 项测试通过；最近文件只保留可访问 Markdown，临时内容不混入正式书目。

### 34. 连续阅读 Focus/章节/进度回归基线

- [x] 补连续阅读 Focus Mode、章节切换、进度和 viewport anchor 回归测试。
  - 由 `3976b67 feat(reader): embed continuous reading in main window`、`7eabd08 fix: 进/出专注模式的定位漂移——按视口锚点补偿字号重排` 与现有 focus/scroll/segment 测试覆盖。
  - `scripts\verify.ps1` 当前桌面 TypeScript 38 项测试与 Svelte 0 警告通过。

### 35. 统一 Shell Playwright harness

- [x] 使用 Playwright/测试 harness 验证统一 Shell；禁止 Computer Use。
  - 实现 commit：`10bf3a4 test(qa): keep bookshelf harness aligned with task state`。
  - `scripts\qa\verify_bookshelf.py` 使用 mock Tauri 数据和 Vite preview 覆盖 ready/loading/empty/error 状态、书架搜索、获取内容入口、视口截图和 page error；已通过，报告时间 `2026-07-12T21:28:25+0800`。
  - mock 补齐 `get_acquisition_snapshot` 与 `list_trash`，并保留 page error stack 便于后续诊断；未读写正式 Library，未使用 Computer Use。

### 36. Archive catalog 清理回归

- [x] 验证清理任务历史后知乎书目、作者导航和 archive catalog 仍存在。
  - 现有 `tools\zhihu-packer\tests\archive-catalog.test.ts` 覆盖删除任务后作者 catalog、成功条目和 revision 仍可查询；`generateAuthorIndex` 使用持久 archive catalog 重建导航。
  - `npm.cmd --prefix .\tools\zhihu-packer test` 通过（25 项），未接触正式数据库或 Library。

## 未完成

以下顺序是建议的继续执行顺序。后续对话应从第一个未勾选且不受关闭授权门阻挡的条目开始。

### A. 最高优先级：让 queued 任务真正执行

### B. Podcast 执行、控制与发布

### C. 知乎执行、登录与发布
- [ ] 在新流程真实 QA 通过前，不删除旧知乎控制台回退入口。

### D. 迁移、继承与数据对账

- [x] 将 migration coordinator 扩展到最近打开记录和旧 MMbook recent-files。
  - `preview_legacy_migration` 现在将旧 `mmbook\recent-files.json` 映射到当前 channel 的 Settings 状态目录，保持只读、敏感性标记和 preview hash 稳定；实现 commit：`792d1c3 feat(migration): preview legacy recent files`。
  - 定向 `migration::preview` 测试 1 项、完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 21:35:36`，EXE SHA-256 前 16 位 `D3D877E53F25310C`，PID `83716` 启动存活并已停止，残留匹配进程 0。
  - 正式 EXE 时间 `2026-07-11 09:49:40`、SHA-256 未变；`.md/.markdown` 仍指向正式 EXE。此项只完成 dry-run preview 覆盖，真实迁移仍受本节“完成 dry-run 后暂停”独立授权门约束。
- [ ] 迁移单 Markdown 阅读状态、Library `.reading.json` 和临时内容记录。
- [ ] 迁移旧 Podcast 未完成任务、非敏感配置与输出索引。
- [ ] 旧明文 DeepSeek Key 写入 Credential Manager并读回验证后，清除新旧 JSON 中的 key 字段。
- [ ] 迁移知乎 SQLite，执行完整 WAL/integrity/count/receipt 流程。
- [ ] 迁移知乎 Profile 到 Data\Private；不得进入 Documents 或 Backups。
- [ ] 迁移已有知乎 Markdown、manifest、provenance 和 archive catalog。
- [ ] 为 legacy `.trash` 生成迁移报告；无法推断原路径的条目保持只读并要求人工选择。
- [ ] 将 reconciliation.json / reconciliation.md 和 migration receipt 持久化到 Data\Migrations。
- [ ] 为每个数据类记录旧位置、新位置、校验、冲突、回滚和敏感性。
- [x] 完成 dry-run 后暂停，等待“真实数据迁移”独立授权，再执行正式数据迁移。
  - 2026-07-13 `cargo test ... migration::preview_tests` 通过（1 passed）；preview 对全量数据类只读、稳定、标记敏感 Profile 且不创建目标数据；证据：`.omo/ulw-loop/evidence/migration-preview-dry-run-20260713.md`。正式迁移未执行。

### E. 统一 Shell、阅读保护与设置

- [x] 把现有阅读器整体封装为独立 `ReaderWorkspace`，不重写 Focus/滚动/viewport anchor 算法。
  - `ReaderWorkspace.svelte` 现在拥有阅读工作区容器、连读时 overflow 策略和 Focus 键盘滚动的 `scroll-behavior` 选择器；`+page.svelte` 继续通过 `bind:element={contentEl}` 使用原有 Focus、滚动和 viewport-anchor 算法。实现 commit：`47ea4ba refactor(reader): isolate workspace shell`。
  - `npm.cmd --prefix .\\apps\\desktop run check`、`scripts\\verify.ps1`、书架 Playwright 多视口/状态 QA 通过；`ship:dev` 时间 `2026-07-13 10:13:32`，开发 EXE SHA-256 前 16 位 `64FDD43D3C1FAF2B`；精确开发 EXE 启动存活 3 秒并停止后残留进程 0。
- [x] 实现 NavigationGuard：保存并继续、放弃并继续、取消导航。
  - `+page.svelte` 增加原生 dialog 三选项 Guard；保存失败会取消导航，放弃会恢复编辑前 DOM，取消保持当前编辑；实现 commit：`9186eec fix(reader): add unsaved navigation guard`。
  - Guard 接入 Markdown 切换、书目/章节切换、返回书架、退出精读和退出应用；完整 `scripts\verify.ps1` 通过，Svelte 0 警告。
  - `ship:dev` 时间 `2026-07-12 21:52:13`，EXE SHA-256 前 16 位 `70354B03744DDBA5`，PID `104540` 启动存活并已停止，残留匹配进程 0；正式 EXE 与 Markdown 关联未变。
- [x] NavigationGuard 覆盖工作区切换、书目切换、精读/连读、返回书架、第二实例 Markdown 和退出。
  - 已覆盖书目/章节、精读/连读进入与返回、返回书架、第二实例 `open-file`、外部 Markdown 和退出；实现补充 commit：`cdcdda9 fix(reader): guard continuous reader navigation`。
  - `npm.cmd --prefix .\apps\desktop run check` 与 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 22:07:58`，EXE SHA-256 前 16 位 `41601D20AA2656BE`，PID `106196` 启动存活并已停止，残留匹配进程 0；正式 EXE 与 Markdown 关联未变。
- [x] 实现 Podcast 配置页、知乎配置页和 Markdown 导入页。
  - `PodcastWorkflow.svelte` 提供文件选择/拖放、翻译、预算、重复策略、预检和任务加入；`ZhihuWorkflow.svelte` 提供受管登录态、回答/文章类型、排序、Top N 和任务控制；`Bookshelf.svelte` 提供 Markdown 文件/文件夹导入入口。
  - 现有 `npm.cmd --prefix .\apps\desktop run check` 与 `scripts\verify.ps1` 已通过；统一 Shell Playwright harness 已覆盖书架获取内容入口，未使用 Computer Use。
- [x] 实现完整任务队列、结构化事件/日志面板和控制按钮。
  - 书架任务 rail 保留 Podcast/知乎开始、暂停、恢复、取消、重试、打开结果和可恢复字节显示；新增最近 60 条结构化事件缓存与最近 12 条展开面板；实现 commit：`b451822 feat(tasks): show structured event log`。
  - 完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 21:57:16`，EXE SHA-256 前 16 位 `07DA7EDCD1FFF7F6F`，PID `105744` 启动存活并已停止，残留匹配进程 0。
  - 正式 EXE 时间 `2026-07-11 09:49:40`、SHA-256 未变；`.md/.markdown` 仍指向正式 EXE。
- [x] 实现书目详情、provenance、revision、来源链接和任务记录。
  - 书架“详情”对话框展示 manifest 的 source/sourceId、生成/更新时间、章节列表和当前阅读状态；Rust `open_book` 校验并返回匹配的 provenance.json（revision、任务 ID、engineVersion、manifest SHA-256）；实现基础 commit：`aa1016f feat(library): expose book provenance details`。
  - 知乎 sourceId 现在可通过受限 `https://www.zhihu.com/people/<id>` 来源按钮打开，并复用既有 HTTP/HTTPS 外链校验；实现 commit：`1d45a26 feat(library): add safe source links`。
  - `BookDetail.taskRecords` 现在按 `bookId` 关联 control.db 的持久化任务快照，详情页展示任务类型、状态、revision、engineStage、时间和错误信息；实现 commit：`23f6c5e feat(library): show associated task records`。
  - 书架 Playwright harness 于 `2026-07-13T10:22:28+0800` 通过，覆盖详情/provenance/来源按钮/任务记录、900×700、1280×800、1440×900 与 ready/loading/empty/unwritable 状态；使用 mock Tauri 数据，未接触正式 Library。
  - `npm.cmd --prefix .\\apps\\desktop run check`、Rust 87 项、前端 38 项通过；`ship:dev` 时间 `2026-07-13 10:24:29`，EXE SHA-256 前 16 位 `711CDC0B900CB181`，精确开发 EXE 启动存活 3 秒并停止后残留进程 0。
- [x] 完成设置页：Library/Data/Cache/Logs/Backups 路径、大小、打开目录、安全清理、备份、凭据、迁移和恢复状态。
  - Settings 已接入受管路径展示/大小/固定根目录打开/路径复制、安全缓存清理、只读迁移 preview、发布恢复检查、Credential Manager 状态、排除 Library/Cache/Logs/凭据/Profile 的状态备份和 migration run 状态；实现 commit：`9b1378d feat(settings): add state backup and migration status`。
  - 完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 22:29:33`，EXE SHA-256 前 16 位 `5B9E03673723D583`，PID `96324` 启动存活并已停止，残留匹配进程 0；正式 EXE 与 Markdown 关联未变。完整 Library 备份与 migration recovery 操作页面仍归入第 453 项。
- [x] 实现 DeepSeek 配置/删除 UI，永不显示 Key。
  - `SettingsPanel.svelte` 提供密码输入、Credential Manager 写入/删除和仅显示 configured 状态，不显示 Key；实现 commit：`7eff974 feat(settings): expose local maintenance controls`。
  - 完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 21:42:14`，EXE SHA-256 前 16 位 `FD5EDB16478B591E`，PID `105528` 启动存活并已停止，残留匹配进程 0。
  - 正式 EXE 时间 `2026-07-11 09:49:40`、SHA-256 未变；`.md/.markdown` 仍指向正式 EXE。
- [x] 实现缓存占用与可恢复任务空间 UI。
  - Settings 显示 Library/Data/Cache/Logs/Backups/RuntimeState 受管目录大小；书架任务 rail 显示可恢复任务字节并保留安全清理入口；实现 commit：`111041d feat(settings): show storage usage and open roots`。
  - Rust 仅允许按当前 channel 打开固定受管目录；完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 22:03:47`，EXE SHA-256 前 16 位 `7A579831E8D47252`，PID `102452` 启动存活并已停止，残留匹配进程 0。
  - 正式 EXE 时间 `2026-07-11 09:49:40`、SHA-256 未变；`.md/.markdown` 仍指向正式 EXE。
- [x] 实现 publish recovery 与 migration recovery 页面。
  - Settings 的“恢复中心”集中展示只读 migration preview/run 状态、publish 待恢复事务和无待恢复空状态；publish 仍提供“执行恢复检查”，真实 migration 执行不在此页面触发；实现 commit：`e389914 feat(settings): add recovery center`。
  - 完整 `scripts\verify.ps1` 通过；`ship:dev` 时间 `2026-07-12 22:38:31`，EXE SHA-256 前 16 位 `39CCA58AC8D00A7EE`，PID `99312` 启动存活并已停止，残留匹配进程 0；正式 EXE 与 Markdown 关联未变。
  - 完整 Library 备份、真实 migration recovery 执行仍受数据迁移授权门约束。

### F. 安全收紧与旧前端移除

- [x] 新 Podcast/知乎流程自动测试与短样本 QA 全部通过后，暂停等待“删除旧前端”独立授权。
  - 2026-07-13 `scripts\verify.ps1` 通过（Podcast 27、Zhihu 25）；两份真实音频短样本本地回归通过；旧 Podcast/知乎入口不删除，等待清单末尾独立授权门。
- [ ] 获准后删除 Podcast 旧 GUI/PowerShell 托盘打包入口。
- [ ] 获准后删除知乎旧控制台打包入口。
- [x] 收紧 Tauri CSP、capabilities、通用 `fs:default` 和 `opener:default`。
  - 实现 commit：`08332d5 security(desktop): tighten runtime capabilities and csp`。
  - `tauri.conf.json` 设置显式 CSP，允许受控 Tauri IPC、loopback Reader、asset/data/blob 图片和显式外链；main capability 移除 `fs:default`/`opener:default`，只保留 `dialog:allow-open` 与 `opener:allow-open-url`。
  - `scripts\verify.ps1` 通过；开发安装启动/停止 QA PID `84624`，停止后残留开发进程 0；正式 EXE 与 Markdown 关联未变。
- [x] 所有文件访问经过受控 Rust 命令；外部打开只允许显式来源 http/https 链接。
  - 实现 commit：`73fad17 security(desktop): restrict external links to http`。
  - Markdown 外链处理拒绝 `mailto:`、`tel:` 和其他协议，只允许显式 `http:`/`https:`；文件和书库路径继续只通过受控 Rust 命令取得。
  - `scripts\verify.ps1`、`ship:dev` 和精确启动/停止 QA 通过；开发 EXE `2026-07-12 21:17:35 / 7F59FE603FD1C4DBF08A8B5CE6B1258CD5D128BCB0277DC046B3A5A0463E28A7`，正式 EXE/关联未改动。

### G. QA、发布与安装

- [x] 从两个真实音频制作短片段副本并完成免费/低成本回归。
  - 2026-07-13 在 `artifacts/qa/podcast-short` 从 Desktop 上两份真实音频生成 30 秒 WAV；Podcast runtime 使用本地 faster-whisper CUDA 与本地 Ollama `qwen3.5:9b`，未调用 DeepSeek/API；`work/reports/run_summary.md`：中文 1/1 成功、英文转录+翻译 1/1 成功、失败 0；证据：`.omo/ulw-loop/evidence/podcast-short-qa-20260713.md`。
- [ ] 在 QA Library 对指定知乎账号执行回答+文章合计 Top 5。
  - 2026-07-13 已在隔离 `ImmersiveReader-QA-zhihu-v3-20260713` 实际 dry-run：受管 Profile 无登录态，目标 answers/articles 页面返回 404/空索引（`logged=false`），结果 `0 + 0`，未创建任务、未写 Library、未发布；登录前置/目标可达性需人工处理；验证码未触发。证据：`.omo/ulw-loop/evidence/zhihu-qa-20260713.md`。
- [x] Podcast 与知乎各一个活动任务并行测试。
  - 2026-07-13 新增并通过 `control::tests::podcast_and_zhihu_active_snapshots_can_coexist`：同一 control.db 同时保存 Podcast/Zhihu 两个 `Running` 快照并按 kind/id 验证；未启动外部真实账号任务。
- [ ] 验证托盘隐藏/恢复、退出和 Job Object 无遗留 Python/Node/FFmpeg/Chromium。
  - 2026-07-13 最新 `.dev-install` EXE 启动/停止后 WebView 子进程残留 0；Rust Job Object 两项测试通过。CLI 关闭请求虽保持进程存活，但原生可见性检查未证明隐藏/恢复闭环，因此该项保持未完成。
- [x] 生成两个完整音频的时长、磁盘、文本规模、费用上限与可用空间报告。
  - 2026-07-13 只读 FFprobe 与预算公式报告：总时长 `3593.990427s`、预计磁盘 `595659231` bytes、翻译规模 `43128` tokens、API 费用上限 `¥0.258768`、C: 可用 `131308507136` bytes；证据：`.omo/ulw-loop/evidence/full-audio-preflight-20260713.md`。
- [x] 暂停等待“完整长音频/API 费用 QA”独立授权。
  - 预检已完成；完整原始音频执行仍未进行，等待独立授权；授权门记录见本清单末尾“明确授权门”。
- [ ] 获准后完整执行两个原始音频，并核对前后 SHA-256 不变。
- [x] 完成 1.1.0 version、README、release notes、runtime manifest、release manifest 和 QA report。
  - 版本已同步到 desktop package/Cargo/Tauri config；`docs/release/1.1.0/` 包含 release notes、QA report、17 项 runtime manifest snapshot 和 release manifest；`scripts\verify.ps1` 及 `ship:dev` 通过，开发 EXE `2026-07-13 11:16:37 / C302561A94048DA2...`。
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

继续“G. QA、发布与安装”：在隔离 QA Library 对 `xiao-xue-shi-46-24` 执行回答+文章合计 Top 5，并记录真实登录/验证码/发布结果。
